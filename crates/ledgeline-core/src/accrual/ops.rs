//! Every write plan 45 makes, in one place: Ledgeline's server handlers and
//! any program built on this crate (an invoicing app) call these, never their
//! own copies.
//!
//! The shape is always **load → plan → verify → write**:
//!
//! 1. [`Books::load`] reads the journal (through a
//!    [`JournalEditor`](crate::edit::JournalEditor), so every file keeps the
//!    revision it was read at), `ledgeline.toml`'s `[books]`, and the
//!    recognition file.
//! 2. A [`Loaded`] method plans the change against exactly what was read: a
//!    due entry already posted is dropped, a date after `today` is refused, a
//!    stale [`Generation`] is refused.
//! 3. The journal edits are made in memory (one reparse per step) and the
//!    recognition file's new text is verified by
//!    [`RecognitionDoc::verify`](super::book::RecognitionDoc::verify).
//! 4. The journal is saved — every file written only over the revision it was
//!    read at ([`OpsError::ExternalChange`] otherwise, and nothing is written)
//!    — and then the recognition file, the same way.
//!
//! The journal goes first and the rules last (D4's write order). If the
//! second write fails, the first stands and [`OpsError::RulesNotWritten`]
//! says so; the mismatch then shows as a `recognition-sum` or
//! `recognition-orphan` finding until the same save is repeated.
//!
//! No global state: `today` is always a parameter, and nothing here knows
//! about UI preferences (auto-posting is the caller's decision to call
//! [`Loaded::post_due`]).

use super::apply::{self, AppliedPayment, ApplyPayment, ApplyUndo};
use super::book::{
    self, BookError, GroupEdit, RecognitionBook, RecognitionDoc, directives_for, group_text,
    heading, recognition_path,
};
use super::books_config::{self, BooksConfig, BooksError};
use super::documents::{DocumentIndex, FoundDocument};
use super::due::{DueSet, closed_adjustments, comment_with, due, entries_to_post, new_transaction};
use super::index::RecognitionIndex;
use super::invoice::{DocumentError, DocumentSpec, build_document_with, posting};
use super::receivables::{Receivables, ReceivablesOptions};
use super::restate::{RestateError, Restatement, restatement};
use super::schedule::{
    Period, PeriodPart, RuleRole, RuleText, Schedule, ScheduleError, ScheduleSpec, build,
};
use super::settle::{self, SettleError};
use super::{
    AccrualGate, AccrualRole, AccrualRoles, INVOICE_TAG, RECOGNITION_TAG, VOID_TAG, accrual_gate,
    is_iso_date,
};
use crate::edit::{EditError, EditOp, Fingerprint, InsertPosition, JournalEditor};
use crate::matching::{IdGenerator, MATCH_TAG, MatchIds, NOMATCH_TAG, PostingRef};
use crate::model::{Journal, Tindex, Transaction};
use crate::parse::JournalStyles;
use crate::vault::{LocalVault, Revision, Vault, VaultError, VaultPath};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use thiserror::Error;

/// The largest recognition file read: far beyond 5,000 schedules.
const MAX_RECOGNITION_BYTES: u64 = 256 * 1024 * 1024;

/// Why an operation wrote nothing (or, for
/// [`OpsError::RulesNotWritten`], only the journal).
#[derive(Debug, Error)]
pub enum OpsError {
    /// A file changed between the load and the write; nothing was written.
    #[error("a file changed on disk since it was read; nothing was written")]
    ExternalChange,
    /// The state the caller previewed is not the state on disk.
    #[error("the books changed since this was previewed; preview again")]
    Stale,
    /// A date after `today` was asked for.
    #[error("{date} is after today ({today}); nothing is ever posted for a future date")]
    FutureDate {
        /// The date asked for.
        date: String,
        /// Today.
        today: String,
    },
    /// The recognition file is needed and doesn't exist (and creating it
    /// wasn't asked for).
    #[error("the recognition file {0} does not exist yet")]
    FileMissing(String),
    /// The recognition file already exists.
    #[error("the recognition file {0} already exists")]
    FileExists(String),
    /// The recognition file's path collides with one of the journal's files.
    #[error("the recognition file can't be {path}: {reason}")]
    Collision {
        /// The path.
        path: String,
        /// Why.
        reason: &'static str,
    },
    /// The journal was saved but the rules weren't: repeat the save.
    #[error("the journal was saved but the recognition file was not: {0}")]
    RulesNotWritten(Box<OpsError>),
    /// A date argument isn't a date.
    #[error("`{0}` is not a date (YYYY-MM-DD)")]
    BadDate(String),
    /// The journal couldn't be read or edited.
    #[error(transparent)]
    Edit(EditError),
    /// The recognition file couldn't be read or edited.
    #[error(transparent)]
    Book(#[from] BookError),
    /// `ledgeline.toml`'s `[books]` couldn't be read or written.
    #[error(transparent)]
    Books(#[from] BooksError),
    /// A schedule couldn't be built.
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    /// A restatement couldn't be computed.
    #[error(transparent)]
    Restate(#[from] RestateError),
    /// A settlement couldn't be linked.
    #[error(transparent)]
    Settle(#[from] SettleError),
    /// A document couldn't be built.
    #[error(transparent)]
    Document(#[from] DocumentError),
    /// No document is booked under this number.
    #[error("no booked document is numbered {0}")]
    DocumentNotFound(String),
    /// The number is already booked (writing it again would double-book it),
    /// or is booked more than once (which one to change is unclear).
    #[error("document {0} is already booked; find it and re-book it instead")]
    DuplicateDocument(String),
    /// The books' rules refuse the change (a closed period, linked payments,
    /// a schedule that would be dropped); the message says which.
    #[error("{0}")]
    Refused(String),
    /// Storage failed.
    #[error("storage: {0}")]
    Storage(String),
}

impl From<EditError> for OpsError {
    fn from(error: EditError) -> Self {
        match error {
            EditError::ExternalChange => Self::ExternalChange,
            // A batch entry's stale fingerprint is the same race.
            EditError::BulkTransaction { source, .. }
                if matches!(*source, EditError::ExternalChange) =>
            {
                Self::ExternalChange
            }
            other => Self::Edit(other),
        }
    }
}

fn storage(error: VaultError) -> OpsError {
    match error {
        VaultError::Changed { .. } => OpsError::ExternalChange,
        other if other.kind() == std::io::ErrorKind::AlreadyExists => OpsError::ExternalChange,
        other => OpsError::Storage(other.to_string()),
    }
}

/// An opaque token for "the books as read": every journal file's content and
/// the recognition file's. A caller that previews holds it and hands it back
/// to the write, which refuses ([`OpsError::Stale`]) if anything changed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Generation(String);

impl Generation {
    /// The token's text, for the wire.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.0
    }

    /// A token received back from the wire.
    #[must_use]
    pub fn from_token(token: impl Into<String>) -> Self {
        Self(token.into())
    }
}

/// One ledger's accrual books: its journal and the files beside it.
#[derive(Debug, Clone)]
pub struct Books {
    vault: Arc<dyn Vault>,
    main: VaultPath,
}

impl Books {
    /// The ledger whose main journal is the local file `main_journal`.
    ///
    /// # Errors
    /// [`OpsError::Storage`] when the path has no usable file name.
    pub fn open(main_journal: &Path) -> Result<Self, OpsError> {
        let (vault, main) = LocalVault::for_journal(main_journal).map_err(storage)?;
        Ok(Self {
            vault: Arc::new(vault),
            main,
        })
    }

    /// The ledger whose main journal is `main` in `vault`.
    #[must_use]
    pub fn open_in(vault: Arc<dyn Vault>, main: VaultPath) -> Self {
        Self { vault, main }
    }

    /// Read everything an operation needs.
    ///
    /// # Errors
    /// [`OpsError::Edit`] when the journal can't be read; [`OpsError::Books`]
    /// when `ledgeline.toml` is unusable; [`OpsError::Book`] when the
    /// recognition file's path is unusable or the file doesn't parse;
    /// [`OpsError::Collision`] when its path names the main journal, the
    /// budget file, or (when configured) a file the journal includes.
    pub fn load(&self) -> Result<Loaded, OpsError> {
        let editor = JournalEditor::open_in(Arc::clone(&self.vault), self.main.clone())?;
        self.load_with(editor)
    }

    /// [`load`](Self::load), over a copy of an editor the caller already has
    /// open on this journal — a server's bound editor — instead of reading and
    /// parsing the journal again. The copy is cheap and `editor` itself is
    /// never changed: take the edited one back with [`Loaded::into_editor`]
    /// once a write has landed. If `editor` is behind the files, the write is
    /// refused with [`OpsError::ExternalChange`] and nothing is written.
    ///
    /// # Errors
    /// As [`load`](Self::load), less reading the journal.
    pub fn load_from(&self, editor: &JournalEditor) -> Result<Loaded, OpsError> {
        self.load_with(editor.clone())
    }

    fn load_with(&self, editor: JournalEditor) -> Result<Loaded, OpsError> {
        let config = books_config::read(&*self.vault, &self.main)?;
        let path = recognition_path(&self.main, config.recognition_file.as_deref())?;
        let key = self.vault.key(&path);
        if let Some(reason) = book::collision(editor.journal(), &key)
            && (config.recognition_file.is_some() || !book::is_included(editor.journal(), &key))
        {
            return Err(OpsError::Collision {
                path: path.to_string(),
                reason,
            });
        }
        let inherited = &editor.journal().commodity_styles;
        let (book, revision) = match self.vault.read_pinned(&path, Some(MAX_RECOGNITION_BYTES)) {
            Ok((bytes, revision)) => {
                let text = String::from_utf8(bytes)
                    .map_err(|_| BookError::Parse("the recognition file is not UTF-8".into()))?;
                (
                    RecognitionBook::parse(&text, &key, inherited)?,
                    Some(revision),
                )
            }
            Err(error) if error.is_not_found() => (RecognitionBook::missing(&key), None),
            Err(error) => return Err(storage(error)),
        };
        Ok(Loaded {
            vault: Arc::clone(&self.vault),
            main: self.main.clone(),
            editor,
            config,
            path,
            book,
            revision,
            documents: OnceLock::new(),
        })
    }

    /// Set or clear `[books] closed-through` (the explicit **Close books
    /// through…** action; creates `ledgeline.toml` if missing).
    ///
    /// # Errors
    /// [`OpsError::Books`].
    pub fn set_closed_through(&self, date: Option<&str>) -> Result<(), OpsError> {
        Ok(books_config::set_closed_through(
            &*self.vault,
            &self.main,
            date,
        )?)
    }
}

/// What a save writes for its source transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceChange {
    /// Nothing (a restatement of an edit made elsewhere, or rules only).
    Unchanged,
    /// Add this transaction, placed by date.
    Add(Box<Transaction>),
    /// Replace the transaction `tindex`, which must still be the one the
    /// caller saw (`fingerprint`).
    Replace {
        /// Which.
        tindex: Tindex,
        /// Its fingerprint as the caller saw it.
        fingerprint: Fingerprint,
        /// Its new content.
        transaction: Box<Transaction>,
    },
}

/// A D-editor save: the source transaction, then its schedules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
    /// The source transaction's change.
    pub source: SourceChange,
    /// The schedules, new or existing (an existing id is restated).
    pub schedules: Vec<ScheduleSpec>,
    /// Today: the date of any restatement adjustment.
    pub today: String,
    /// Create the recognition file if it doesn't exist.
    pub create_file: bool,
    /// Refuse unless the books are still as previewed.
    pub expected: Option<Generation>,
}

/// What a save will write (a preview) or wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePlan {
    /// The new schedules, built.
    pub appended: Vec<Schedule>,
    /// The restated schedules: the rules and every posted entry.
    pub restatements: Vec<Restatement>,
    /// The recognition file's new text.
    pub recognition_text: String,
    /// Whether the recognition file is created.
    pub creates_file: bool,
    /// The books as read.
    pub generation: Generation,
}

/// What **Post through ‹date›** posts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostDue {
    /// The server's (or app's) today. Nothing after it is ever posted.
    pub today: String,
    /// Post only entries dated on or before this (≤ today); `None` is today.
    pub through: Option<String>,
    /// Post only these `(id, recognized)`; `None` posts every open due entry.
    pub only: Option<Vec<(String, String)>>,
    /// Also fold the due entries in closed periods into one adjustment per
    /// schedule, dated today.
    pub include_closed: bool,
    /// Refuse unless the books are still as listed.
    pub expected: Option<Generation>,
}

/// What a post wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Posted {
    /// `(id, recognized)` of every entry written.
    pub entries: Vec<(String, String)>,
    /// The ids that got a closed-period adjustment.
    pub adjustments: Vec<String>,
}

/// One invoice ↔ payment link of a [`Loaded::link_settlements`] batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleLink {
    /// The invoice (or bill) posting on the receivable (payable) account.
    pub invoice: PostingRef,
    /// The payment's posting on the same account.
    pub payment: PostingRef,
    /// Where a short payment's difference goes (required for one).
    pub fee_account: Option<String>,
}

/// What [`Loaded::void_document`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voided {
    /// The reversing transaction.
    pub transaction: Transaction,
    /// The `match:` id joining it to the document.
    pub link: String,
}

/// A ledger's books as read by [`Books::load`].
pub struct Loaded {
    vault: Arc<dyn Vault>,
    main: VaultPath,
    editor: JournalEditor,
    config: BooksConfig,
    path: VaultPath,
    book: RecognitionBook,
    revision: Option<Revision>,
    /// The document index and its documents' fingerprints, built on first
    /// use and dropped by every in-memory edit.
    documents: OnceLock<LoadedDocuments>,
}

/// [`DocumentIndex`] plus each document's fingerprint, taken in one pass.
#[derive(Debug)]
struct LoadedDocuments {
    index: DocumentIndex,
    fingerprints: super::FxHashMap<usize, Fingerprint>,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded")
            .field("main", &self.main)
            .field("recognition", &self.path)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Loaded {
    /// The editor, with whatever this `Loaded` wrote: after a write it is in
    /// step with the files, so a caller holding a long-lived editor can adopt
    /// it rather than re-read the journal. A `Loaded` is spent after one write;
    /// [`Books::load`] again for the next.
    #[must_use]
    pub fn into_editor(self) -> JournalEditor {
        self.editor
    }

    /// The journal.
    #[must_use]
    pub fn journal(&self) -> &Journal {
        self.editor.journal()
    }

    /// The editor the journal was read through (for a caller's own edits).
    #[must_use]
    pub fn editor(&self) -> &JournalEditor {
        &self.editor
    }

    /// `[books]`.
    #[must_use]
    pub fn config(&self) -> &BooksConfig {
        &self.config
    }

    /// The recognition file's place in the vault.
    #[must_use]
    pub fn recognition_path(&self) -> &VaultPath {
        &self.path
    }

    /// Whether the recognition file exists.
    #[must_use]
    pub fn recognition_exists(&self) -> bool {
        self.revision.is_some()
    }

    /// The parsed recognition file (empty when it doesn't exist).
    #[must_use]
    pub fn book(&self) -> &RecognitionBook {
        &self.book
    }

    /// The books as read.
    #[must_use]
    pub fn generation(&self) -> Generation {
        let mut text = String::new();
        for (path, fingerprint) in self.editor.source_fingerprints() {
            text.push_str(path.as_str());
            text.push('=');
            text.push_str(&fingerprint.map(Fingerprint::token).unwrap_or_default());
            text.push('\n');
        }
        text.push_str(self.path.as_str());
        text.push('=');
        if self.revision.is_some() {
            text.push_str(&self.book.revision.token());
        }
        Generation(Fingerprint::of_bytes(text.as_bytes()).token())
    }

    /// The accrual roles of the journal's accounts.
    #[must_use]
    pub fn roles(&self) -> AccrualRoles {
        AccrualRoles::of(self.journal())
    }

    /// The gate, under `[books] basis`.
    #[must_use]
    pub fn gate(&self) -> AccrualGate {
        accrual_gate(&self.roles(), self.config.basis)
    }

    /// The recognition index.
    #[must_use]
    pub fn index(&self) -> RecognitionIndex {
        RecognitionIndex::build(self.journal(), &self.book)
    }

    /// What is due as of `today`, under `[books] closed-through`.
    #[must_use]
    pub fn due(&self, today: &str) -> DueSet {
        due(
            &self.index(),
            &self.book,
            today,
            self.config.closed_through.as_deref(),
        )
    }

    /// A generator of fresh `recognition:` ids: none already in the journal
    /// or the recognition file.
    #[must_use]
    pub fn id_generator(&self) -> IdGenerator {
        let journal = self.journal();
        let in_journal = journal.transactions.iter().flat_map(|txn| {
            txn.tags
                .iter()
                .chain(txn.postings.iter().flat_map(|p| &p.tags))
                .filter(|(name, _)| name == RECOGNITION_TAG)
                .map(|(_, value)| value.trim().to_string())
        });
        let in_book = self.book.ids().into_iter().map(str::to_string);
        IdGenerator::avoiding(in_journal.chain(in_book).collect::<Vec<_>>())
    }

    fn check_expected(&self, expected: Option<&Generation>) -> Result<(), OpsError> {
        match expected {
            Some(expected) if *expected != self.generation() => Err(OpsError::Stale),
            _ => Ok(()),
        }
    }

    /// Create the recognition file (D1): its header, beside the journal.
    ///
    /// # Errors
    /// [`OpsError::FileExists`]; [`OpsError::ExternalChange`] when something
    /// appeared at the path since the load.
    pub fn create_recognition_file(&mut self) -> Result<VaultPath, OpsError> {
        if self.recognition_exists() {
            return Err(OpsError::FileExists(self.path.to_string()));
        }
        let text = self.new_file_text();
        self.vault
            .create_new_shared(&self.path, text.as_bytes())
            .map_err(storage)?;
        Ok(self.path.clone())
    }

    fn new_file_text(&self) -> String {
        let relative = self
            .path
            .as_str()
            .strip_prefix(&format!(
                "{}/",
                self.main.parent().unwrap_or_default().as_str()
            ))
            .unwrap_or(self.path.as_str())
            .to_string();
        book::new_file_text(
            self.main.file_name().unwrap_or("journal"),
            &relative,
            self.journal(),
        )
    }

    /// The schedule `id` as its rules stand now, for a restatement: the
    /// periods' dates from the rules, the lines from the source (else the
    /// new spec's), and the group's text.
    fn current_schedule(
        &self,
        index: &RecognitionIndex,
        doc: &RecognitionDoc,
        text: &str,
        new: &ScheduleSpec,
    ) -> Schedule {
        let spec = index
            .spec_of(&new.id, &self.book)
            .unwrap_or_else(|| new.clone());
        let periods = self
            .book
            .occurrence_dates(&new.id)
            .into_iter()
            .map(|date| Period {
                date,
                amounts: Vec::new(),
                part: PeriodPart::Regular,
            })
            .collect();
        let rules_text = doc
            .groups()
            .iter()
            .find(|group| group.id == new.id)
            .map(|group| {
                let slice = &text[group.span.clone()];
                match slice.strip_prefix(book::HEADING_MARK) {
                    Some(rest) => rest.split_once('\n').map_or("", |(_, r)| r).to_string(),
                    None => slice.to_string(),
                }
            })
            .unwrap_or_default();
        Schedule {
            id: new.id.clone(),
            periods,
            rules: vec![RuleText {
                role: RuleRole::Main,
                period: String::new(),
                text: rules_text,
            }],
            renewal_date: String::new(),
            spec,
        }
    }

    /// Plan a save against the books as read, applying the journal half to
    /// the in-memory editor.
    fn plan_save(&mut self, request: &SaveRequest) -> Result<SavePlan, OpsError> {
        self.check_expected(request.expected.as_ref())?;
        self.documents.take();
        if !is_iso_date(&request.today) {
            return Err(OpsError::BadDate(request.today.clone()));
        }
        let generation = self.generation();
        let creates_file = !self.recognition_exists();
        if creates_file && !request.create_file && !request.schedules.is_empty() {
            return Err(OpsError::FileMissing(self.path.to_string()));
        }
        let before = RecognitionIndex::build(self.journal(), &self.book);
        let text = if creates_file {
            self.new_file_text()
        } else {
            self.book.text.clone()
        };
        let inherited = self.journal().commodity_styles.clone();
        let current = RecognitionBook::parse(&text, &self.book.key, &inherited)?;
        let doc = RecognitionDoc::new(&current)?;
        let olds: Vec<Option<Schedule>> = request
            .schedules
            .iter()
            .map(|spec| {
                (!current.rules_of(&spec.id).is_empty())
                    .then(|| self.current_schedule(&before, &doc, &text, spec))
            })
            .collect();

        // 1. The source transaction, in memory.
        match &request.source {
            SourceChange::Unchanged => {}
            SourceChange::Add(txn) => self
                .editor
                .add_transactions(std::slice::from_ref(txn), InsertPosition::DateOrdered)?,
            SourceChange::Replace {
                tindex,
                fingerprint,
                transaction,
            } => self.editor.apply_batch(&[(
                *tindex,
                fingerprint.clone(),
                vec![EditOp::Replace(transaction.clone())],
            )])?,
        }

        // 2. Each schedule: build it, and restate an existing one.
        let journal = Arc::clone(self.editor.journal());
        let styles = JournalStyles::of(&journal);
        let after = RecognitionIndex::build(&journal, &current);
        let closed = self.config.closed_through.clone();
        let mut appended = Vec::new();
        let mut restatements = Vec::new();
        let mut edits = Vec::new();
        for (spec, old) in request.schedules.iter().zip(olds) {
            let schedule = build(spec, &styles)?;
            let party = spec
                .lines
                .iter()
                .find_map(|line| line.party.as_ref())
                .map(|party| party.name.as_str());
            let source = after
                .schedule(&spec.id)
                .and_then(|info| info.sources.first())
                .map(|source| &journal.transactions[source.at.position]);
            let heading = heading(
                party,
                &spec.id,
                source.map_or(spec.start.as_str(), |t| t.date.as_str()),
                source.map_or(spec.description.as_str(), |t| t.description.as_str()),
            );
            let edit_text = group_text(&heading, &schedule);
            let directives = directives_for(&schedule, &journal);
            match old {
                None => {
                    edits.push(GroupEdit::Append {
                        id: spec.id.clone(),
                        text: edit_text,
                        directives,
                    });
                    appended.push(schedule);
                }
                Some(old) => {
                    let info = after.schedule(&spec.id).cloned().unwrap_or_default();
                    let restated = restatement(
                        &old,
                        &schedule,
                        &journal,
                        &info,
                        closed.as_deref(),
                        &request.today,
                        &styles,
                    )?;
                    if restated.rule_change.is_some() {
                        edits.push(GroupEdit::Replace {
                            id: spec.id.clone(),
                            text: edit_text,
                            directives,
                        });
                    }
                    restatements.push(restated);
                }
            }
        }

        // 3. The restatements' journal half, in memory: rewrites in place,
        // deletes from the bottom up, adjustments placed by date.
        // Every fingerprint in one pass (a restatement can rewrite dozens).
        let pending: Vec<(Tindex, Vec<EditOp>)> = restatements
            .iter()
            .flat_map(|r| r.rewrites.iter().cloned())
            .collect();
        let tindices: Vec<Tindex> = pending.iter().map(|(tindex, _)| *tindex).collect();
        let rewrites: Vec<(Tindex, Fingerprint, Vec<EditOp>)> = pending
            .into_iter()
            .zip(self.editor.transaction_fingerprints(&tindices))
            .map(|((tindex, ops), fingerprint)| {
                fingerprint
                    .map(|fingerprint| (tindex, fingerprint, ops))
                    .ok_or(OpsError::Stale)
            })
            .collect::<Result<_, _>>()?;
        if !rewrites.is_empty() {
            self.editor.apply_batch(&rewrites)?;
        }
        let mut deletes: Vec<Tindex> = restatements
            .iter()
            .flat_map(|r| r.deletes.iter().copied())
            .collect();
        deletes.sort_by_key(|tindex| std::cmp::Reverse(tindex.0));
        for tindex in deletes {
            self.editor.delete_transaction(tindex)?;
        }
        let adjustments: Vec<Transaction> = restatements
            .iter()
            .filter_map(|r| r.adjustment.clone())
            .collect();
        if !adjustments.is_empty() {
            self.editor
                .add_transactions(&adjustments, InsertPosition::DateOrdered)?;
        }

        // 4. The recognition file's new text, each edit verified.
        let (mut text, mut current) = (text, current);
        for edit in &edits {
            let doc = RecognitionDoc::new(&current)?;
            let next = doc.apply(edit)?;
            current = doc.verify(&current, edit, &next, &inherited)?;
            text = next;
        }
        Ok(SavePlan {
            appended,
            restatements,
            recognition_text: text,
            creates_file: creates_file && !edits.is_empty(),
            generation,
        })
    }

    /// What [`Loaded::save`] would write, without writing it.
    ///
    /// # Errors
    /// As [`Loaded::save`], less the write's own.
    pub fn preview(mut self, request: &SaveRequest) -> Result<SavePlan, OpsError> {
        self.plan_save(request)
    }

    /// A D-editor save (D-editor, D4): the source transaction, then for each
    /// schedule either a new rule group or a restatement (rewritten and
    /// deleted entries, one adjustment, the replaced group). The journal is
    /// written first, the recognition file last.
    ///
    /// # Errors
    /// [`OpsError::Stale`], [`OpsError::FileMissing`], [`OpsError::Schedule`],
    /// [`OpsError::Restate`], [`OpsError::Book`], [`OpsError::Edit`] —
    /// nothing written; [`OpsError::ExternalChange`] — nothing written;
    /// [`OpsError::RulesNotWritten`] — the journal was written.
    pub fn save(&mut self, request: &SaveRequest) -> Result<SavePlan, OpsError> {
        let plan = self.plan_save(request)?;
        self.editor.save()?;
        let unchanged = !plan.creates_file && plan.recognition_text == self.book.text;
        if !unchanged {
            self.write_recognition(&plan.recognition_text)
                .map_err(|error| OpsError::RulesNotWritten(Box::new(error)))?;
        }
        Ok(plan)
    }

    fn write_recognition(&self, text: &str) -> Result<(), OpsError> {
        match &self.revision {
            Some(revision) => self
                .vault
                .replace(&self.path, text.as_bytes(), revision)
                .map(|_| ())
                .map_err(storage),
            None => self
                .vault
                .create_new_shared(&self.path, text.as_bytes())
                .map(|_| ())
                .map_err(storage),
        }
    }

    /// Write a document (invoice, bill or prepayment) and set up its
    /// schedules in one save. New ids come from [`Loaded::id_generator`].
    ///
    /// `expected` is the [`generation`](Self::generation) the caller planned
    /// against (L3): when the books aren't that any more, nothing is written.
    /// A numbered invoice or bill whose number is already booked is refused,
    /// so a retried send never books twice — re-book it instead.
    ///
    /// # Errors
    /// [`OpsError::Document`] for a spec that won't build;
    /// [`OpsError::DuplicateDocument`] for a number already booked;
    /// [`OpsError::Stale`] for a generation mismatch; otherwise as
    /// [`Loaded::save`].
    pub fn write_document(
        &mut self,
        document: &DocumentSpec,
        today: &str,
        create_file: bool,
        expected: Option<&Generation>,
    ) -> Result<(Transaction, SavePlan), OpsError> {
        self.check_expected(expected)?;
        if let (Some(number), Some(role)) = (&document.number, self.settlement_role(document))
            && !self.documents().get(role, number).is_empty()
        {
            return Err(OpsError::DuplicateDocument(number.clone()));
        }
        let mut ids = self.id_generator();
        let built = build_document_with(document, &mut || ids.next_id())?;
        let request = SaveRequest {
            source: SourceChange::Add(Box::new(built.transaction.clone())),
            schedules: built.schedules,
            today: today.to_string(),
            create_file,
            expected: None,
        };
        let plan = self.save(&request)?;
        Ok((built.transaction, plan))
    }

    /// The role of `document`'s settlement account, when it is a receivable
    /// or payable one (a prepayment settles through the bank: `None`).
    fn settlement_role(&self, document: &DocumentSpec) -> Option<AccrualRole> {
        AccrualRoles::of_accounts(
            &self.journal().accounts,
            [document.settlement_account.as_str()],
        )
        .role(&document.settlement_account)
        .filter(|role| role.is_settlement())
    }

    fn loaded_documents(&self) -> &LoadedDocuments {
        self.documents.get_or_init(|| {
            let journal = self.journal();
            let index = DocumentIndex::build(journal, &AccrualRoles::of(journal));
            let refs: Vec<PostingRef> = index
                .iter()
                .flat_map(|(_, _, refs)| refs.iter().copied())
                .collect();
            let tindices: Vec<Tindex> = refs.iter().map(|at| at.txn).collect();
            // Every document's fingerprint in ONE pass over each file: one
            // at a time rebuilds the file's line index per call.
            let fingerprints = refs
                .iter()
                .zip(self.editor.transaction_fingerprints(&tindices))
                .filter_map(|(at, fingerprint)| Some((at.position, fingerprint?)))
                .collect();
            LoadedDocuments {
                index,
                fingerprints,
            }
        })
    }

    /// Every numbered invoice and bill in the books, built once per load
    /// (and again after an in-memory edit).
    #[must_use]
    pub fn documents(&self) -> &DocumentIndex {
        &self.loaded_documents().index
    }

    /// The booked invoice numbered `number` (L3): `invoice:` matched exactly
    /// on a transaction with a receivable debit, never a payment, a void
    /// reversal or a cash-books deposit. With the number used twice, the
    /// first in file order; [`documents`](Self::documents) lists duplicates.
    ///
    /// The first call builds the index for every document; later calls are a
    /// hash lookup and one transaction clone.
    #[must_use]
    pub fn find_document(&self, number: &str) -> Option<FoundDocument> {
        self.find_document_as(AccrualRole::Receivable, number)
    }

    /// [`find_document`](Self::find_document) for an invoice
    /// ([`AccrualRole::Receivable`]) or a bill ([`AccrualRole::Payable`]).
    #[must_use]
    pub fn find_document_as(&self, role: AccrualRole, number: &str) -> Option<FoundDocument> {
        let loaded = self.loaded_documents();
        let at = *loaded.index.get(role, number).first()?;
        let transaction = self.journal().transactions.get(at.position)?.clone();
        Some(FoundDocument {
            tindex: at.txn,
            position: at.position,
            fingerprint: loaded.fingerprints.get(&at.position)?.clone(),
            transaction,
            settlement: at,
            role,
        })
    }

    /// The one booked `role` document numbered `number`.
    fn the_document(&self, role: AccrualRole, number: &str) -> Result<FoundDocument, OpsError> {
        if self.documents().get(role, number).len() > 1 {
            return Err(OpsError::DuplicateDocument(number.to_string()));
        }
        self.find_document_as(role, number)
            .ok_or_else(|| OpsError::DocumentNotFound(number.to_string()))
    }

    /// Refuse `date` on or before `[books] closed-through` (I8).
    fn check_open(&self, what: &str, date: &str) -> Result<(), OpsError> {
        match self.config.closed_through.as_deref() {
            Some(closed) if date <= closed => Err(OpsError::Refused(format!(
                "{what} is dated {date}, on or before closed-through {closed}; void and \
                 reissue it in an open period instead"
            ))),
            _ => Ok(()),
        }
    }

    /// **Re-book** a sent document (ARCHITECTURE §4.5): replace its
    /// transaction with `document` rebuilt, found by `document.number`, and
    /// restate its schedules in the same save.
    ///
    /// - Its deferred lines keep their `recognition:` ids, in line order, so
    ///   each existing schedule is restated (D4) rather than started again.
    ///   Dropping a schedule is refused: end its contract first.
    /// - The settlement posting keeps its `match:` link. If the amount
    ///   changed, the link no longer balances and shows as one to check.
    /// - Refused when the original or the new date is on or before
    ///   `closed-through` (I8): void and reissue instead.
    ///
    /// # Errors
    /// [`OpsError::DocumentNotFound`], [`OpsError::DuplicateDocument`],
    /// [`OpsError::Refused`], [`OpsError::Stale`]; otherwise as
    /// [`Loaded::save`].
    pub fn rebook_document(
        &mut self,
        document: &DocumentSpec,
        today: &str,
        create_file: bool,
        expected: Option<&Generation>,
    ) -> Result<(Transaction, SavePlan), OpsError> {
        self.check_expected(expected)?;
        let number = document.number.as_deref().ok_or_else(|| {
            OpsError::Document(DocumentError::Invalid(
                "a re-booked document needs its number".into(),
            ))
        })?;
        let role = self.settlement_role(document).ok_or_else(|| {
            OpsError::Document(DocumentError::Invalid(format!(
                "{} is not a receivable or payable account",
                document.settlement_account
            )))
        })?;
        let found = self.the_document(role, number)?;
        self.check_open(&format!("document {number}"), &found.transaction.date)?;
        self.check_open(&format!("document {number}"), &document.date)?;
        let mut old_ids: Vec<String> = Vec::new();
        for posting in &found.transaction.postings {
            if let Some(id) = super::tag_value(&posting.tags, RECOGNITION_TAG)
                && !id.is_empty()
                && id != super::NONE_VALUE
                && !old_ids.iter().any(|old| old == id)
            {
                old_ids.push(id.to_string());
            }
        }
        let mut fresh = self.id_generator();
        let mut reused = old_ids.clone().into_iter();
        let built = build_document_with(document, &mut || {
            reused.next().unwrap_or_else(|| fresh.next_id())
        })?;
        if let Some(dropped) = old_ids
            .iter()
            .find(|id| !built.schedules.iter().any(|s| &s.id == *id))
        {
            return Err(OpsError::Refused(format!(
                "re-booking document {number} would drop its schedule {dropped}; end that \
                 contract first"
            )));
        }
        let mut transaction = built.transaction;
        let old_settlement = &found.transaction.postings[found.settlement.posting];
        if let Some(link) = super::tag_value(&old_settlement.tags, MATCH_TAG) {
            let settlement = &mut transaction.postings[0];
            settlement.comment = format!(
                "{}\n",
                crate::edit::upsert_tag(
                    settlement.comment.trim_end_matches('\n'),
                    MATCH_TAG,
                    Some(link)
                )
            );
            settlement.tags = crate::parse::parse_tags(&settlement.comment);
        }
        let request = SaveRequest {
            source: SourceChange::Replace {
                tindex: found.tindex,
                fingerprint: found.fingerprint,
                transaction: Box::new(transaction.clone()),
            },
            schedules: built.schedules,
            today: today.to_string(),
            create_file,
            expected: None,
        };
        let plan = self.save(&request)?;
        Ok((transaction, plan))
    }

    /// **Void** a booked document (ARCHITECTURE §4.1): a reversing
    /// transaction dated `date`, tagged `invoice:` and [`VOID_TAG`], each
    /// posting the original's negated (its `customer:`/`item:` tags kept),
    /// and a `match:` link joining the two settlement postings, so the
    /// receivable nets to zero and [`super::receivables`] reads the reversal
    /// as the document's settlement. One write.
    ///
    /// # Errors
    /// [`OpsError::Refused`] when payments are already linked to it (that
    /// needs a credit note), it has deferred lines (end their contracts
    /// first), or `date` is before the document or on or before
    /// `closed-through`; [`OpsError::DocumentNotFound`],
    /// [`OpsError::DuplicateDocument`], [`OpsError::BadDate`],
    /// [`OpsError::Stale`], [`OpsError::ExternalChange`].
    pub fn void_document(
        &mut self,
        role: AccrualRole,
        number: &str,
        date: &str,
        expected: Option<&Generation>,
    ) -> Result<Voided, OpsError> {
        self.check_expected(expected)?;
        if !is_iso_date(date) {
            return Err(OpsError::BadDate(date.to_string()));
        }
        let found = self.the_document(role, number)?;
        let original = &found.transaction;
        if date < original.date.as_str() {
            return Err(OpsError::Refused(format!(
                "a void can't be dated {date}, before document {number} ({})",
                original.date
            )));
        }
        self.check_open(&format!("the void of document {number}"), date)?;
        let settlement = &original.postings[found.settlement.posting];
        if super::tag_value(&settlement.tags, MATCH_TAG).is_some() {
            return Err(OpsError::Refused(format!(
                "document {number} already has payments linked; a void would reverse paid \
                 work — issue a credit note"
            )));
        }
        if original
            .postings
            .iter()
            .any(|p| super::tag_value(&p.tags, RECOGNITION_TAG).is_some())
        {
            return Err(OpsError::Refused(format!(
                "document {number} has deferred lines; end their contracts before voiding it"
            )));
        }
        let link = MatchIds::new(self.journal()).next_id();
        let dropped = [MATCH_TAG, NOMATCH_TAG];
        let postings = original
            .postings
            .iter()
            .enumerate()
            .filter(|(_, p)| p.generated.is_none())
            .map(|(at, p)| {
                let [amount] = p.amounts.as_slice() else {
                    return Err(OpsError::Refused(format!(
                        "document {number}'s posting on {} has no single amount to reverse",
                        p.account.0
                    )));
                };
                if amount.cost.is_some() {
                    return Err(OpsError::Refused(format!(
                        "document {number}'s posting on {} has a cost",
                        p.account.0
                    )));
                }
                let reversed = crate::model::Amount {
                    quantity: amount
                        .quantity
                        .neg()
                        .map_err(|_| OpsError::Document(DocumentError::Overflow))?,
                    ..amount.clone()
                };
                let mut tags: Vec<(&str, String)> = p
                    .tags
                    .iter()
                    .filter(|(name, _)| !dropped.contains(&name.as_str()))
                    .map(|(name, value)| (name.as_str(), value.clone()))
                    .collect();
                if at == found.settlement.posting {
                    tags.push((MATCH_TAG, link.clone()));
                }
                Ok(posting(&p.account.0, reversed, &tags))
            })
            .collect::<Result<Vec<_>, OpsError>>()?;
        let reversal = new_transaction(
            date,
            &format!("{} (void)", original.description),
            comment_with("", &[(INVOICE_TAG, number), (VOID_TAG, date)]),
            postings,
        );
        self.documents.take();
        self.editor.apply_batch(&[(
            found.tindex,
            found.fingerprint.clone(),
            vec![EditOp::SetPostingTag {
                posting: found.settlement.posting,
                name: MATCH_TAG.to_string(),
                value: Some(link.clone()),
            }],
        )])?;
        self.editor
            .add_transactions(std::slice::from_ref(&reversal), InsertPosition::DateOrdered)?;
        self.editor.save()?;
        Ok(Voided {
            transaction: reversal,
            link,
        })
    }

    /// Every document's settlement, open and unapplied amounts, aging and
    /// DSO ([`super::receivables`]).
    #[must_use]
    pub fn receivables(&self, options: &ReceivablesOptions) -> Receivables {
        let journal = self.journal();
        Receivables::build(journal, &AccrualRoles::of(journal), options)
    }

    /// Join `payments` to the settlement link of the document whose
    /// settlement posting is `document` (invoicer L4: N:1, one invoice paid
    /// in parts), starting the link if it has none. Returns the link id.
    ///
    /// Each payment must be a payment-side posting on the document's account
    /// and commodity (a receivable credit for an invoice), in another
    /// transaction, with no `match:` yet. Together with what is already
    /// linked they may cover the document exactly (the link is then
    /// complete) or not yet (it stays open, a partial payment); they may not
    /// exceed it. One write.
    ///
    /// # Errors
    /// [`OpsError::Settle`] for a posting that isn't a document or a
    /// payment, a mismatched account or commodity, a posting already linked,
    /// a document already settled, or an overpayment;
    /// [`OpsError::Stale`] / [`OpsError::ExternalChange`] — nothing written.
    pub fn link_payments(
        &mut self,
        document: PostingRef,
        payments: &[PostingRef],
        expected: Option<&Generation>,
    ) -> Result<String, OpsError> {
        self.check_expected(expected)?;
        let journal = Arc::clone(self.editor.journal());
        let roles = AccrualRoles::of(&journal);
        let not_a_pair = |why: String| OpsError::Settle(SettleError::NotAPair(why));
        let resolve = |at: PostingRef| {
            journal
                .transactions
                .get(at.position)
                .filter(|txn| txn.index == at.txn)
                .and_then(|txn| txn.postings.get(at.posting).map(|p| (txn, p)))
                .ok_or_else(|| not_a_pair("a posting is no longer where it was".into()))
        };
        let (doc_txn, doc_posting) = resolve(document)?;
        let Some((_, role)) = super::documents::document_posting(doc_txn, &roles)
            .filter(|(at, _)| *at == document.posting)
        else {
            return Err(not_a_pair(
                "that posting is not an invoice's (or bill's) settlement posting".into(),
            ));
        };
        let [doc_amount] = doc_posting.amounts.as_slice() else {
            return Err(not_a_pair("the document has no single amount".into()));
        };
        let links = crate::matching::links(&journal);
        let existing = super::tag_value(&doc_posting.tags, MATCH_TAG).map(str::to_string);
        let mut sum = doc_amount.quantity;
        if let Some(id) = &existing {
            if !links.is_open(id) {
                return Err(not_a_pair(format!(
                    "the document's link {id} is already settled (or broken); unlink it first"
                )));
            }
            for member in links.members(id).unwrap_or_default() {
                if member != document {
                    let (_, p) = resolve(member)?;
                    sum = sum
                        .add(p.amounts[0].quantity)
                        .map_err(|_| OpsError::Document(DocumentError::Overflow))?;
                }
            }
        }
        if payments.is_empty() {
            return Err(not_a_pair("no payment was given".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for &payment in payments {
            let (txn, posting) = resolve(payment)?;
            let [amount] = posting.amounts.as_slice() else {
                return Err(not_a_pair("a payment has no single amount".into()));
            };
            let payment_side = (amount.quantity.mantissa > 0) != (role == AccrualRole::Receivable);
            if posting.account != doc_posting.account
                || amount.commodity != doc_amount.commodity
                || amount.cost.is_some()
                || amount.quantity.is_zero()
                || !payment_side
                || posting.generated.is_some()
                || payment.position == document.position
                || !seen.insert(payment.position)
                || super::tag_value(&txn.tags, VOID_TAG).is_some()
            {
                return Err(not_a_pair(format!(
                    "{} on {} is not a payment of this document (same account and commodity, \
                     the other side, its own transaction)",
                    txn.description, txn.date
                )));
            }
            if super::tag_value(&posting.tags, MATCH_TAG).is_some() {
                return Err(not_a_pair(format!(
                    "{} on {} is already linked",
                    txn.description, txn.date
                )));
            }
            sum = sum
                .add(amount.quantity)
                .map_err(|_| OpsError::Document(DocumentError::Overflow))?;
        }
        if !sum.is_zero() && (sum.mantissa > 0) != (role == AccrualRole::Receivable) {
            return Err(OpsError::Settle(SettleError::Overpayment));
        }
        let id = existing
            .clone()
            .unwrap_or_else(|| MatchIds::new(&journal).next_id());
        let tag = |posting: usize| EditOp::SetPostingTag {
            posting,
            name: MATCH_TAG.to_string(),
            value: Some(id.clone()),
        };
        let mut targets: Vec<(PostingRef, EditOp)> =
            payments.iter().map(|&p| (p, tag(p.posting))).collect();
        if existing.is_none() {
            targets.push((document, tag(document.posting)));
        }
        targets.sort_by_key(|(at, _)| at.position);
        let tindices: Vec<Tindex> = targets.iter().map(|(at, _)| at.txn).collect();
        let edits = targets
            .into_iter()
            .zip(self.editor.transaction_fingerprints(&tindices))
            .map(|((at, op), fingerprint)| {
                fingerprint
                    .map(|fingerprint| (at.txn, fingerprint, vec![op]))
                    .ok_or(OpsError::Stale)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.documents.take();
        self.editor.apply_batch(&edits)?;
        self.editor.save()?;
        Ok(id)
    }

    /// **Apply payment** (cash books, invoicer I17): re-split the deposit
    /// `request.tindex`'s income side into the invoice's row accounts (pro
    /// rata for a partial payment), tag it `invoice:` and `customer:`, and
    /// keep every other line — the bank leg above all — byte for byte. One
    /// write, behind the transaction's fingerprint and the books'
    /// generation. Returns what was written and how to undo it.
    ///
    /// # Errors
    /// [`OpsError::Refused`] for a transaction that isn't a cash-books
    /// deposit, one already applied, or a deposit larger than the invoice
    /// (see [`apply::plan`]); [`OpsError::Stale`] for a generation mismatch;
    /// [`OpsError::ExternalChange`] when the deposit changed since its
    /// fingerprint was taken — nothing written.
    pub fn apply_payment(
        &mut self,
        request: &ApplyPayment,
        expected: Option<&Generation>,
    ) -> Result<AppliedPayment, OpsError> {
        self.check_expected(expected)?;
        let journal = Arc::clone(self.editor.journal());
        let txn = journal
            .transactions
            .iter()
            .find(|txn| txn.index == request.tindex)
            .ok_or(OpsError::Stale)?;
        let original_text = self
            .editor
            .transaction_source(request.tindex)
            .ok_or(OpsError::Stale)?;
        let planned = apply::plan(&journal, txn, request)?;
        self.documents.take();
        self.editor
            .apply_batch(&[(request.tindex, request.fingerprint.clone(), planned.ops)])?;
        let fingerprint = self
            .editor
            .transaction_fingerprint(request.tindex)
            .ok_or(OpsError::Stale)?;
        let transaction = self
            .journal()
            .transactions
            .iter()
            .find(|txn| txn.index == request.tindex)
            .cloned()
            .ok_or(OpsError::Stale)?;
        self.editor.save()?;
        Ok(AppliedPayment {
            transaction,
            received: planned.received,
            applied: planned.applied,
            partial: planned.partial,
            undo: ApplyUndo {
                number: request.number.clone(),
                fingerprint,
                original_text,
            },
        })
    }

    /// Undo an [`apply_payment`](Self::apply_payment): write the deposit's
    /// original text back, exactly. The deposit is found by its `invoice:`
    /// tag and the fingerprint the apply left, wherever it has moved since.
    ///
    /// # Errors
    /// [`OpsError::Stale`] when no deposit tagged with the number still has
    /// that fingerprint (it was edited since: nothing is written), or for a
    /// generation mismatch; [`OpsError::ExternalChange`].
    pub fn undo_apply_payment(
        &mut self,
        undo: &ApplyUndo,
        expected: Option<&Generation>,
    ) -> Result<(), OpsError> {
        self.check_expected(expected)?;
        let tagged: Vec<Tindex> = self
            .journal()
            .transactions
            .iter()
            .filter(|txn| super::tag_value(&txn.tags, INVOICE_TAG) == Some(undo.number.as_str()))
            .map(|txn| txn.index)
            .collect();
        let tindex = tagged
            .iter()
            .zip(self.editor.transaction_fingerprints(&tagged))
            .find(|(_, fingerprint)| fingerprint.as_ref() == Some(&undo.fingerprint))
            .map(|(tindex, _)| *tindex)
            .ok_or(OpsError::Stale)?;
        self.documents.take();
        self.editor.apply_batch(&[(
            tindex,
            undo.fingerprint.clone(),
            vec![EditOp::SetText(undo.original_text.clone())],
        )])?;
        self.editor.save()?;
        Ok(())
    }

    /// **Post through ‹date›** (D6): every open due entry dated on or before
    /// `through` (capped at today), in one `add_transactions`, placed by
    /// date. What is already posted is dropped, so a repeat writes nothing.
    ///
    /// # Errors
    /// [`OpsError::FutureDate`] for a `through` after today;
    /// [`OpsError::Stale`]; [`OpsError::ExternalChange`] — nothing written.
    pub fn post_due(&mut self, request: &PostDue) -> Result<Posted, OpsError> {
        self.check_expected(request.expected.as_ref())?;
        let today = request.today.as_str();
        if !is_iso_date(today) {
            return Err(OpsError::BadDate(today.to_string()));
        }
        let through = request.through.as_deref().unwrap_or(today);
        if !is_iso_date(through) {
            return Err(OpsError::BadDate(through.to_string()));
        }
        if through > today {
            return Err(OpsError::FutureDate {
                date: through.to_string(),
                today: today.to_string(),
            });
        }
        let set = self.due(today);
        let wanted = |id: &str, recognized: &str| {
            request
                .only
                .as_ref()
                .is_none_or(|only| only.iter().any(|(i, r)| i == id && r == recognized))
        };
        let open: Vec<_> = set
            .through(through)
            .into_iter()
            .filter(|entry| wanted(&entry.id, &entry.recognized))
            .collect();
        let closed: Vec<_> = if request.include_closed {
            let ids: Vec<&str> = set
                .closed
                .iter()
                .filter(|entry| wanted(&entry.id, &entry.recognized))
                .map(|entry| entry.id.as_str())
                .collect();
            // A schedule's closed occurrences fold together, always.
            set.closed
                .iter()
                .filter(|entry| ids.contains(&entry.id.as_str()))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        self.documents.take();
        let mut transactions = entries_to_post(&self.book, &open);
        let adjustments = match self.config.closed_through.as_deref() {
            Some(closed_through) => closed_adjustments(&self.book, &closed, today, closed_through),
            None => Vec::new(),
        };
        transactions.extend(adjustments.iter().cloned());
        if transactions.is_empty() {
            return Ok(Posted::default());
        }
        self.editor
            .add_transactions(&transactions, InsertPosition::DateOrdered)?;
        self.editor.save()?;
        Ok(Posted {
            entries: open
                .into_iter()
                .map(|entry| (entry.id, entry.recognized))
                .collect(),
            adjustments: adjustments
                .iter()
                .filter_map(|txn| super::tag_value(&txn.tags, RECOGNITION_TAG).map(str::to_string))
                .collect(),
        })
    }

    /// Link an invoice (or bill) to its payment (D-settle): with the fee edit
    /// for a short payment (`fee_account` required), `match:ID` on both
    /// settlement postings. Returns the id. One [`link_settlements`]
    /// (Self::link_settlements) of one.
    ///
    /// # Errors
    /// As [`link_settlements`](Self::link_settlements).
    pub fn link_settlement(
        &mut self,
        invoice: PostingRef,
        payment: PostingRef,
        fee_account: Option<&str>,
        tolerance_bps: u32,
    ) -> Result<String, OpsError> {
        let link = SettleLink {
            invoice,
            payment,
            fee_account: fee_account.map(str::to_string),
        };
        let mut ids = self.link_settlements(std::slice::from_ref(&link), tolerance_bps)?;
        ids.pop()
            .ok_or_else(|| OpsError::Storage("no link was made".into()))
    }

    /// Link several invoices to their payments in ONE write: every pair is
    /// checked first, then all their edits go through one batch — one
    /// reparse, one save — so either every link lands or none does. Returns
    /// the match ids in `links` order.
    ///
    /// Nothing is re-proposed: exactly these pairs are linked, as given.
    ///
    /// # Errors
    /// - [`OpsError::Settle`] when a pair isn't a legal settlement pair, the
    ///   payment is over, a fee account is missing, a posting appears in two
    ///   links, or a fee edit (which rewrites its payment's postings) would
    ///   share a transaction with another link in the batch;
    /// - [`OpsError::Stale`] / [`OpsError::ExternalChange`] — nothing written.
    pub fn link_settlements(
        &mut self,
        links: &[SettleLink],
        tolerance_bps: u32,
    ) -> Result<Vec<String>, OpsError> {
        if links.is_empty() {
            return Ok(Vec::new());
        }
        self.documents.take();
        let journal = Arc::clone(self.editor.journal());
        let roles = AccrualRoles::of(&journal);
        // Once for the batch: a scan of the journal per pair is what made a
        // 25-pair Link all cost 25 scans.
        let candidates = settle::pair_candidates(&journal, &roles);
        let mut ids = MatchIds::new(&journal);
        let mut used = std::collections::HashSet::new();
        // Per transaction (keyed by tindex, file order kept by the editor):
        // its ops, and whether one of them rewrites its postings (a fee).
        let mut by_txn: std::collections::BTreeMap<u32, (Tindex, Vec<EditOp>, bool)> =
            std::collections::BTreeMap::new();
        let mut out = Vec::with_capacity(links.len());
        for (n, link) in links.iter().enumerate() {
            for side in [link.invoice, link.payment] {
                if !used.insert((side.txn.0, side.posting)) {
                    return Err(SettleError::NotAPair(format!(
                        "link {}: posting {} of transaction #{} is in another link too",
                        n + 1,
                        side.posting,
                        side.txn.0
                    ))
                    .into());
                }
            }
            let proposal = settle::pair_among(
                &journal,
                &candidates,
                link.invoice,
                link.payment,
                tolerance_bps,
            )
            .ok_or_else(|| {
                SettleError::NotAPair(format!(
                    "link {}: these postings are not an invoice and its payment",
                    n + 1
                ))
            })?;
            let rewrites = matches!(proposal.fit, settle::Fit::Short { .. });
            let id = ids.next_id();
            for (tindex, ops) in
                settle::link_ops(&journal, &proposal, link.fee_account.as_deref(), &id)?
            {
                let entry = by_txn
                    .entry(tindex.0)
                    .or_insert((tindex, Vec::new(), false));
                if !entry.1.is_empty() && (entry.2 || rewrites) {
                    return Err(SettleError::NotAPair(format!(
                        "link {}: transaction #{} is in another link, and a fee edit rewrites it; link these one at a time",
                        n + 1,
                        tindex.0
                    ))
                    .into());
                }
                entry.1.extend(ops);
                entry.2 |= rewrites;
            }
            out.push(id);
        }
        // Every fingerprint in ONE pass: one at a time was ~20 ms each at 200k
        // transactions, most of a 60-pair Link all.
        let tindices: Vec<Tindex> = by_txn.values().map(|(tindex, _, _)| *tindex).collect();
        let fingerprints = self.editor.transaction_fingerprints(&tindices);
        let edits: Vec<(Tindex, Fingerprint, Vec<EditOp>)> = by_txn
            .into_values()
            .zip(fingerprints)
            .map(|((tindex, ops, _), fingerprint)| {
                fingerprint
                    .map(|fingerprint| (tindex, fingerprint, ops))
                    .ok_or(OpsError::Stale)
            })
            .collect::<Result<_, _>>()?;
        self.editor.apply_batch(&edits)?;
        self.editor.save()?;
        Ok(out)
    }
}
