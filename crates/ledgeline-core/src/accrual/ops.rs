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

use super::book::{
    self, BookError, GroupEdit, RecognitionBook, RecognitionDoc, directives_for, group_text,
    heading, recognition_path,
};
use super::books_config::{self, BooksConfig, BooksError};
use super::due::{DueSet, closed_adjustments, due, entries_to_post};
use super::index::RecognitionIndex;
use super::invoice::{DocumentError, DocumentSpec, build_document};
use super::restate::{RestateError, Restatement, restatement};
use super::schedule::{
    Period, PeriodPart, RuleRole, RuleText, Schedule, ScheduleError, ScheduleSpec, build,
};
use super::settle::{self, SettleError};
use super::{AccrualGate, AccrualRoles, RECOGNITION_TAG, accrual_gate, is_iso_date};
use crate::edit::{EditError, EditOp, Fingerprint, InsertPosition, JournalEditor};
use crate::matching::{IdGenerator, MatchIds, PostingRef};
use crate::model::{Journal, Tindex, Transaction};
use crate::parse::JournalStyles;
use crate::vault::{LocalVault, Revision, Vault, VaultError, VaultPath};
use std::path::Path;
use std::sync::Arc;
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
    /// Storage failed.
    #[error("storage: {0}")]
    Storage(String),
}

impl From<EditError> for OpsError {
    fn from(error: EditError) -> Self {
        match error {
            EditError::ExternalChange => Self::ExternalChange,
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

/// A ledger's books as read by [`Books::load`].
pub struct Loaded {
    vault: Arc<dyn Vault>,
    main: VaultPath,
    editor: JournalEditor,
    config: BooksConfig,
    path: VaultPath,
    book: RecognitionBook,
    revision: Option<Revision>,
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
        let rewrites: Vec<(Tindex, Fingerprint, Vec<EditOp>)> = restatements
            .iter()
            .flat_map(|r| r.rewrites.iter().cloned())
            .map(|(tindex, ops)| {
                self.editor
                    .transaction_fingerprint(tindex)
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
    /// # Errors
    /// [`OpsError::Document`] for a spec that won't build; otherwise as
    /// [`Loaded::save`].
    pub fn write_document(
        &mut self,
        document: &DocumentSpec,
        today: &str,
        create_file: bool,
    ) -> Result<(Transaction, SavePlan), OpsError> {
        let built = build_document(document, &mut self.id_generator())?;
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
    /// settlement postings. Returns the id.
    ///
    /// # Errors
    /// [`OpsError::Settle`] when the two aren't a legal pair, the payment is
    /// over, or a fee account is missing; [`OpsError::ExternalChange`].
    pub fn link_settlement(
        &mut self,
        invoice: PostingRef,
        payment: PostingRef,
        fee_account: Option<&str>,
        tolerance_bps: u32,
    ) -> Result<String, OpsError> {
        let journal = Arc::clone(self.editor.journal());
        let roles = AccrualRoles::of(&journal);
        let proposal =
            settle::pair(&journal, &roles, invoice, payment, tolerance_bps).ok_or_else(|| {
                SettleError::NotAPair("these postings are not an invoice and its payment".into())
            })?;
        let id = MatchIds::new(&journal).next_id();
        let ops = settle::link_ops(&journal, &proposal, fee_account, &id)?;
        let edits: Vec<(Tindex, Fingerprint, Vec<EditOp>)> = ops
            .into_iter()
            .map(|(tindex, ops)| {
                self.editor
                    .transaction_fingerprint(tindex)
                    .map(|fingerprint| (tindex, fingerprint, ops))
                    .ok_or(OpsError::Stale)
            })
            .collect::<Result<_, _>>()?;
        self.editor.apply_batch(&edits)?;
        self.editor.save()?;
        Ok(id)
    }
}
