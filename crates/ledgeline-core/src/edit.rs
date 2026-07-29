//! The journal **write path** — safe, format-preserving edits of the plain-text
//! hledger journal (Phase 5.1).
//!
//! [`JournalEditor`] holds one text buffer ([`ropey::Rope`] + load-time
//! fingerprint) **per source file** — the main journal plus every `include`d
//! file a transaction was parsed from — keyed by the file's resolved absolute
//! path, together with the parsed [`Journal`]. Edits are addressed through each
//! [`Transaction`]'s own file ([`Transaction::source_file`]) and its
//! `source_span` within that file: a transaction occupies its rope's character
//! range `[line_to_char(span.0.line - 1), line_to_char(span.1.line - 1))` — the
//! header line through the last body line, inclusive of their trailing
//! newlines. Because `source_span` lines are relative to the transaction's own
//! file, editing a transaction that lives in an `include`d file touches only
//! that file's rope and leaves the main journal (and every other include)
//! byte-identical.
//!
//! Those line numbers are resolved through [`Lines`], an **LF-only** index, not
//! through ropey's own line index — see [`Lines`] for why the difference
//! silently destroyed postings (DL-1).
//!
//! Two operations are implemented, proving the two edit patterns:
//! - [`JournalEditor::delete_transaction`] removes a transaction's span (plus a
//!   trailing blank-line separator) and leaves every *other* transaction's
//!   source text byte-identical.
//! - [`JournalEditor::add_transaction`] formats a [`Transaction`] to clean,
//!   valid journal text (see [`format_transaction`]) and inserts it, either at
//!   end-of-file or in date order.
//!
//! # Safety model (data integrity is paramount — this writes real books)
//! - **Reparse-and-verify.** After any mutation the WHOLE journal is re-parsed
//!   with [`parse_journal_with_overrides`], resolving `include`s from the EDITED
//!   in-memory ropes (not the stale on-disk copies). The edit is committed only
//!   if it parses cleanly, keeps the expected transaction count, leaves every
//!   OTHER transaction's source text byte-identical, and introduces no
//!   unbalanced transaction anywhere (and, for an add, the new transaction
//!   balances and round-trips). See [`JournalEditor::validate_with`]. On failure
//!   `self` is left untouched.
//! - **External-change guard.** [`JournalEditor::save`] re-reads each file it is
//!   about to write and compares its content hash to that file's load-time
//!   fingerprint, refusing (with [`EditError::ExternalChange`]) — and writing
//!   nothing — rather than clobbering a file that changed underneath it. The
//!   content hash is the only evidence consulted: an unchanged mtime proves
//!   nothing (DL-3).
//! - **Atomic write.** `save` writes each changed file to a temp file in the
//!   same directory, `fsync`s it, and `rename`s it over the target. The temp
//!   file is created exclusively (so a planted symlink cannot capture the write)
//!   and inherits the target's mode and ownership, so saving never widens the
//!   permissions on the user's books. See [`atomic_write`] for exactly what does
//!   and does not survive the rename.
//! - **Single writer.** Mutations take `&mut self`; the server will wrap the
//!   editor in a `Mutex` in the next increment (no OS-level lock yet).

use crate::decimal::{Dec, DecError};
use crate::model::{
    Amount, AmountStyle, BalanceAssertion, Commodity, CommoditySide, CostKind, Journal, Posting,
    PostingType, Status, Tindex, Transaction,
};
use crate::parse::{
    ParseError, check_transaction_balances, parse_journal, parse_journal_with_overrides,
    resolve_source_file,
};
use ropey::Rope;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

/// Errors produced by the write path.
///
/// Unlike [`ParseError`] this is intentionally not `Clone`/`PartialEq`: it wraps
/// [`std::io::Error`], which is neither.
#[derive(Debug, Error)]
pub enum EditError {
    /// No transaction with the requested [`Tindex`] exists.
    #[error("transaction #{0} not found in the journal")]
    TransactionNotFound(u32),
    /// A posting index was out of range for the addressed transaction.
    #[error("transaction #{txn} has no posting at index {posting}")]
    PostingNotFound {
        /// The addressed transaction's `tindex`.
        txn: u32,
        /// The out-of-range posting index.
        posting: usize,
    },
    /// The journal failed to parse while loading it.
    #[error("failed to parse the journal: {0}")]
    Parse(#[from] ParseError),
    /// The edit would make the journal unparseable, so it was rejected and no
    /// state changed.
    #[error("the edit would make the journal invalid and was rejected: {0}")]
    ParseInvalidAfterEdit(ParseError),
    /// A transaction being added does not balance.
    #[error("the transaction does not balance")]
    Unbalanced,
    /// A transaction being added is not supported by the formatter/write path
    /// (e.g. a posting carrying multiple commodity amounts).
    #[error("unsupported transaction for add: {0}")]
    Unsupported(String),
    /// The formatted transaction did not re-parse back to the intended one — a
    /// formatting/round-trip guard tripped, so the edit was rejected.
    #[error("the formatted transaction did not round-trip to the intended value")]
    RoundTripMismatch,
    /// The file on disk changed since it was loaded; `save` refuses to overwrite
    /// it rather than clobber an external edit.
    #[error("the journal file changed on disk since it was loaded; refusing to overwrite")]
    ExternalChange,
    /// An exact-decimal arithmetic error while checking a balance.
    #[error("decimal error: {0}")]
    Decimal(#[from] DecError),
    /// An I/O error reading or writing the journal file.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// An internal invariant/addressing guard tripped (should not happen for a
    /// well-formed journal); surfaced instead of panicking.
    #[error("internal edit error: {0}")]
    Internal(String),
}

/// Where [`JournalEditor::add_transaction`] places the new transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPosition {
    /// Append after the last transaction (end of the **main** file).
    Append,
    /// Place the new transaction next to its chronological neighbors ACROSS all
    /// source files: immediately after the latest existing transaction dated `<=`
    /// the new one (in that predecessor's file, so a per-year/per-month `include`
    /// receives the row), or before the earliest transaction when the new one
    /// precedes them all, or — for an empty journal — appended to the main file.
    DateOrdered,
}

/// A load-time fingerprint used to detect external changes before saving: the
/// content `hash` and `len`, and deliberately nothing else.
///
/// It used to carry the file's mtime as well, and an unchanged mtime was taken
/// as proof of unchanged content. It is not (DL-3) — every mtime-preserving copy
/// tool breaks that inference — so the timestamp is no longer recorded at all
/// rather than left lying around as a tempting shortcut. A mtime-only touch that
/// leaves the bytes identical is likewise not an external change. See
/// [`file_changed_externally`].
#[derive(Debug, Clone)]
struct Fingerprint {
    hash: u64,
    len: u64,
}

impl Fingerprint {
    fn of_bytes(bytes: &[u8]) -> Self {
        Self {
            hash: fnv1a_64(bytes),
            len: bytes.len() as u64,
        }
    }

    /// Whether two fingerprints describe byte-identical content.
    fn content_matches(&self, other: &Self) -> bool {
        self.len == other.len && self.hash == other.hash
    }
}

/// FNV-1a 64-bit hash — small, dependency-free, and deterministic within a run
/// (all we need: the load-time and pre-save hashes are computed by the same
/// code over the same byte representation).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Read a journal file as UTF-8, naming the file and the encoding when it is
/// not.
///
/// `read_to_string` reports only *"stream did not contain valid UTF-8"* with no
/// path, which for a Latin-1 journal (a `£` or a `ñ` in a payee is enough) reads
/// as an unexplained failure to open the user's books. This fails CLOSED — a
/// lossy decode would write mojibake back over the original — so the fix is to
/// say which file and what to do about it, not to guess the charset.
fn read_journal_text(path: &Path) -> Result<String, EditError> {
    std::fs::read_to_string(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::InvalidData {
            EditError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{} is not valid UTF-8. Ledgeline reads and writes UTF-8 journals only; \
                     convert it first (e.g. `iconv -f latin1 -t utf-8`) so no character is \
                     silently rewritten.",
                    path.display()
                ),
            ))
        } else {
            EditError::Io(err)
        }
    })
}

/// One source file's editable buffer: its rope, the load-time fingerprint used
/// to detect external changes, and whether the rope has been edited since load
/// (so [`JournalEditor::save`] writes only the files that actually changed).
struct FileBuf {
    /// Absolute path to read/write — identical to this buffer's key in
    /// [`JournalEditor::files`].
    path: PathBuf,
    rope: Rope,
    fingerprint: Fingerprint,
    dirty: bool,
}

impl FileBuf {
    fn new(path: PathBuf, text: &str) -> Self {
        Self {
            fingerprint: Fingerprint::of_bytes(text.as_bytes()),
            rope: Rope::from_str(text),
            path,
            dirty: false,
        }
    }
}

/// A format-preserving editor over a journal that may span several files (a main
/// journal plus any `include`d files). Each distinct source file has its own
/// [`FileBuf`], keyed by its resolved absolute path; every edit is applied to the
/// rope of the file the addressed transaction actually lives in.
pub struct JournalEditor {
    /// The main journal's `source_name` (the string passed to the parser),
    /// re-used to reparse the whole journal on validate.
    source_name: String,
    /// The resolved key of the main file within [`Self::files`].
    main_key: PathBuf,
    /// One buffer per distinct source file (main + includes-with-transactions),
    /// keyed by resolved absolute path.
    files: HashMap<PathBuf, FileBuf>,
    /// Behind an [`Arc`] so a consumer that wants to *hold* the parsed journal —
    /// the server's snapshot, above all — can share this one allocation instead
    /// of deep-cloning it (PERF-1b: the clone was 86 ms and 284 MB at 200k
    /// transactions). Replaced wholesale on every committed edit, so no edit is
    /// ever visible through a previously-handed-out `Arc`.
    journal: Arc<Journal>,
}

impl JournalEditor {
    /// Open `path`, reading the main file and every `include`d file a
    /// transaction came from into its own rope, parsing the whole journal, and
    /// capturing a per-file load-time fingerprint.
    ///
    /// # Errors
    /// [`EditError::Io`] if a file cannot be read, or [`EditError::Parse`] if
    /// the journal does not parse.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, EditError> {
        let path = path.into();
        let text = read_journal_text(&path)?;
        let source_name = path.to_string_lossy().into_owned();
        let journal = parse_journal(&text, &source_name)?;
        Self::load(source_name, journal, &text)
    }

    /// Build an editor over in-memory `text` associated with `path`, without
    /// touching the filesystem. This path is single-file: it does not resolve
    /// `include`s (the initial parse would read them from disk).
    ///
    /// The main file's fingerprint is taken from `text`, so a later
    /// [`save`](Self::save) compares against exactly the bytes the caller
    /// supplied (and requires `path` to exist on disk). Useful for in-memory
    /// editing (the server can hold the text already) and for tests.
    ///
    /// # Errors
    /// [`EditError::Parse`] if `text` does not parse.
    pub fn from_text(path: impl Into<PathBuf>, text: &str) -> Result<Self, EditError> {
        let path = path.into();
        let source_name = path.to_string_lossy().into_owned();
        let journal = parse_journal(text, &source_name)?;
        Self::load(source_name, journal, text)
    }

    /// Build an editor from a freshly-parsed `journal`. The main file's text is
    /// `main_text` (already in hand); every other file a transaction came from
    /// is read from disk.
    fn load(source_name: String, journal: Journal, main_text: &str) -> Result<Self, EditError> {
        let main_key = resolve_source_file(&source_name);
        let mut files: HashMap<PathBuf, FileBuf> = HashMap::new();
        files.insert(main_key.clone(), FileBuf::new(main_key.clone(), main_text));
        for txn in &journal.transactions {
            if !files.contains_key(&txn.source_file) {
                let key = txn.source_file.clone();
                let file_text = read_journal_text(&key)?;
                files.insert(key.clone(), FileBuf::new(key, &file_text));
            }
        }
        Ok(Self {
            source_name,
            main_key,
            files,
            journal: Arc::new(journal),
        })
    }

    /// The main journal file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.main_key
    }

    /// The parsed journal, as of the last committed edit.
    ///
    /// Returned as the shared [`Arc`] rather than a plain `&Journal` so a caller
    /// that needs to keep it (the server publishes it in every snapshot) can
    /// clone the pointer instead of the journal. `&Arc<Journal>` deref-coerces to
    /// `&Journal`, so read-only callers are unaffected.
    #[must_use]
    pub fn journal(&self) -> &Arc<Journal> {
        &self.journal
    }

    /// The current text of the MAIN journal file (materialized from its rope).
    /// Edits to `include`d files are reflected in those files' ropes (and on
    /// disk after [`save`](Self::save)), not here.
    #[must_use]
    pub fn text(&self) -> String {
        self.files
            .get(&self.main_key)
            .map_or_else(String::new, |file| file.rope.to_string())
    }

    /// The number of transactions currently in the journal.
    #[must_use]
    pub fn transaction_count(&self) -> usize {
        self.journal.transactions.len()
    }

    /// The exact source text of the transaction with `index` (its
    /// `source_span`, excluding any trailing blank line) from its OWN file, or
    /// `None` if there is no such transaction. Handy for byte-identity
    /// assertions.
    #[must_use]
    pub fn transaction_source(&self, index: Tindex) -> Option<String> {
        let txn = self.find_transaction(index)?;
        let rope = self.rope_for(&txn.source_file).ok()?;
        let (start, end) = txn_char_range(&Lines::new(rope), txn).ok()?;
        Some(rope.slice(start..end).to_string())
    }

    fn find_transaction(&self, index: Tindex) -> Option<&Transaction> {
        self.journal.transactions.iter().find(|t| t.index == index)
    }

    /// The rope of the file keyed by `key`, or an internal error if no buffer is
    /// loaded for it (should not happen for a transaction's own `source_file`).
    fn rope_for(&self, key: &Path) -> Result<&Rope, EditError> {
        self.files.get(key).map(|file| &file.rope).ok_or_else(|| {
            EditError::Internal(format!(
                "no loaded buffer for source file {}",
                key.display()
            ))
        })
    }

    /// Reparse the whole journal against the current ropes with `edited_key`'s
    /// rope replaced by `candidate`, then commit `candidate` to that file's rope
    /// (marking it dirty) and adopt the reparsed journal. `self` is left
    /// untouched on any validation failure.
    fn apply(
        &mut self,
        edited_key: &Path,
        candidate: Rope,
        expected: usize,
    ) -> Result<(), EditError> {
        let reparsed = self.validate_with(edited_key, &candidate, expected)?;
        self.commit(edited_key, candidate, reparsed)
    }

    /// Reparse the whole journal, overriding `edited_key`'s on-disk content with
    /// `candidate` and every other loaded file with its current (possibly
    /// already-edited) rope, then prove the edit did only what was asked.
    ///
    /// # What is checked (DL-4)
    /// The transaction COUNT used to be the whole guard, which is close to no
    /// guard at all: it is satisfied by an edit that deletes the right number of
    /// transactions from the wrong places. That is exactly how DL-1 wrote an
    /// unbalanced, hledger-rejecting journal and reported success. Three checks
    /// now have to pass:
    ///
    /// 1. **The count** is `expected`, as before.
    /// 2. **Every untouched transaction is byte-identical.** The before and
    ///    after transaction texts must agree on a common prefix and a common
    ///    suffix with at most one transaction between them on each side — see
    ///    [`check_single_change`]. A corrupted neighbor fails here even when it
    ///    still parses and still balances.
    /// 3. **No new unbalanced transaction anywhere.** The whole reparsed journal
    ///    goes through [`check_transaction_balances`] (PARSE-1's checker, so
    ///    hledger's exact rule including its two-commodity inferred conversion),
    ///    and any failure that was not already present before the edit rejects
    ///    it. Diffing against the before state matters: a journal that already
    ///    contains an unbalanced transaction must stay editable, or one bad row
    ///    freezes the whole file.
    fn validate_with(
        &self,
        edited_key: &Path,
        candidate: &Rope,
        expected: usize,
    ) -> Result<Journal, EditError> {
        let mut texts: HashMap<PathBuf, String> = self
            .files
            .iter()
            .map(|(key, file)| (key.clone(), file.rope.to_string()))
            .collect();
        // Read the pre-edit sources while the map still holds them, so the
        // before and after states cost one materialization of the journal
        // between them rather than two.
        let before = transaction_sources(&self.journal, &texts)?;

        texts.insert(edited_key.to_path_buf(), candidate.to_string());
        let reparsed = parse_journal_with_overrides(&self.source_name, &texts)
            .map_err(EditError::ParseInvalidAfterEdit)?;
        if reparsed.transactions.len() != expected {
            return Err(EditError::Internal(format!(
                "expected {expected} transactions after the edit, found {}",
                reparsed.transactions.len()
            )));
        }

        let after = transaction_sources(&reparsed, &texts)?;
        check_single_change(&before, &after)?;
        check_no_new_imbalance(&self.journal, &before, &reparsed, &after)?;
        Ok(reparsed)
    }

    /// Replace `edited_key`'s rope with `candidate` (marking it dirty) and adopt
    /// `reparsed` as the new journal.
    fn commit(
        &mut self,
        edited_key: &Path,
        candidate: Rope,
        reparsed: Journal,
    ) -> Result<(), EditError> {
        let file = self.files.get_mut(edited_key).ok_or_else(|| {
            EditError::Internal(format!(
                "no loaded buffer for source file {}",
                edited_key.display()
            ))
        })?;
        file.rope = candidate;
        file.dirty = true;
        self.journal = Arc::new(reparsed);
        Ok(())
    }

    /// Delete the transaction with `index`.
    ///
    /// Removes the transaction's `source_span` from ITS OWN file's rope, plus any
    /// trailing indented comment lines that belong to it, plus **one** following
    /// blank line. That blank-line rule keeps a transaction sitting between two
    /// others from leaving a double blank, without eating a neighbor's separating
    /// blank. When the deletion instead runs to end-of-file (the transaction was
    /// the last content), one *preceding* blank line is dropped so the file does
    /// not end on a dangling separator. Either way only blank lines — owned by no
    /// transaction — are touched beyond the span, so every *other* transaction's
    /// source text (in this and every other file) is left byte-identical.
    ///
    /// # Errors
    /// [`EditError::TransactionNotFound`] if no such transaction exists, or
    /// [`EditError::ParseInvalidAfterEdit`] if (unexpectedly) the result does not
    /// re-parse. On any error `self` is unchanged.
    pub fn delete_transaction(&mut self, index: Tindex) -> Result<(), EditError> {
        let (source_file, start_span, end_span) = {
            let txn = self
                .find_transaction(index)
                .ok_or(EditError::TransactionNotFound(index.0))?;
            (
                txn.source_file.clone(),
                txn.source_span.0.line.saturating_sub(1) as usize,
                txn.source_span.1.line.saturating_sub(1) as usize,
            )
        };
        let rope = self.rope_for(&source_file)?;
        let lines = Lines::new(rope);

        let len_lines = lines.len();
        let start_line0 = start_span.min(len_lines);
        let mut end_line0 = end_span.min(len_lines);
        // (a) trailing indented comment lines are part of this transaction.
        // Since PARSE-7 the parser already carries them inside `source_span`, so
        // this is now a no-op kept as a backstop rather than the load-bearing rule.
        while end_line0 < len_lines && lines.is_indented_content(end_line0) {
            end_line0 += 1;
        }
        // (b) consume one following blank separator line, if present.
        if end_line0 < len_lines && lines.is_blank(end_line0) {
            end_line0 += 1;
        }
        let mut start = lines.line_to_char(start_line0)?;
        let end = lines.line_to_char(end_line0.min(len_lines))?;
        // (c) if the deletion runs to end-of-file, drop one preceding blank so the
        // file does not end on a dangling separator blank.
        if end == rope.len_chars() && start_line0 > 0 && lines.is_blank(start_line0 - 1) {
            start = lines.line_to_char(start_line0 - 1)?;
        }

        let expected = self.journal.transactions.len() - 1;
        let mut candidate = rope.clone();
        candidate.remove(start..end);
        self.apply(&source_file, candidate, expected)
    }

    /// Add `txn` to the journal at `position`.
    ///
    /// The transaction is formatted with [`format_transaction`] and inserted with
    /// exactly one blank line of separation. It must balance (a single posting
    /// may elide its amount — an empty `amounts` vec — to be inferred on
    /// re-parse). After insertion the whole journal is re-parsed and the new
    /// transaction is checked to balance and to round-trip to the intended
    /// value.
    ///
    /// # Placement
    /// [`InsertPosition::Append`] appends to the **main** file. With
    /// [`InsertPosition::DateOrdered`] the new transaction is written into the file
    /// that holds its chronological neighbors — after the latest transaction dated
    /// `<=` the new one (in that file), or before the earliest when the new one
    /// precedes them all — so a per-year/per-month `include`d file receives the row.
    /// An empty journal falls back to appending to the main file. Only that one
    /// file's rope is edited (and, on [`save`](Self::save), written).
    ///
    /// # Errors
    /// [`EditError::Unbalanced`], [`EditError::Unsupported`] (a posting with
    /// multiple commodity amounts), [`EditError::ParseInvalidAfterEdit`],
    /// [`EditError::RoundTripMismatch`], or [`EditError::Internal`]. On any error
    /// `self` is unchanged.
    pub fn add_transaction(
        &mut self,
        txn: &Transaction,
        position: InsertPosition,
    ) -> Result<(), EditError> {
        if txn.postings.iter().any(|p| p.amounts.len() > 1) {
            return Err(EditError::Unsupported(
                "a posting carries multiple commodity amounts".to_string(),
            ));
        }
        if !is_balanced(&txn.postings)? {
            return Err(EditError::Unbalanced);
        }

        let body = format_transaction(txn);
        let Placement {
            file_key,
            insertion,
        } = self.placement_for(&body, txn, position)?;
        let prefix_len = insertion.prefix.chars().count();
        let header_char = insertion.offset + prefix_len;

        let expected = self.journal.transactions.len() + 1;
        let mut candidate = self.rope_for(&file_key)?.clone();
        candidate.insert(insertion.offset, &insertion.prefix);
        candidate.insert(insertion.offset + prefix_len, &insertion.body);
        let reparsed = self.validate_with(&file_key, &candidate, expected)?;

        let added = locate_in_file(&candidate, &reparsed, &file_key, header_char)?;
        if !is_balanced(&added.postings)? {
            return Err(EditError::Unbalanced);
        }
        if !transactions_equivalent(txn, added) {
            return Err(EditError::RoundTripMismatch);
        }

        self.commit(&file_key, candidate, reparsed)
    }

    /// Change **only** the description of the transaction with `index`.
    ///
    /// Rewrites just the transaction's header line (`source_span.0.line`) with a
    /// header rebuilt from the transaction carrying `new_description` — same
    /// date, status, code, and trailing `; comment`. Every posting line below
    /// (accounts, amounts, comments, and whitespace) is left byte-identical, and
    /// the header line's own terminator is preserved. The mutated text is
    /// re-parsed to validate; the edit is refused (with `self` untouched) unless
    /// the re-parsed transaction's description is exactly `new_description`, so a
    /// `;` (or other separator) smuggled into the text cannot silently change the
    /// transaction's meaning.
    ///
    /// # Errors
    /// [`EditError::TransactionNotFound`], [`EditError::ParseInvalidAfterEdit`],
    /// [`EditError::RoundTripMismatch`], or [`EditError::Internal`].
    pub fn set_description(
        &mut self,
        index: Tindex,
        new_description: &str,
    ) -> Result<(), EditError> {
        let (source_file, header_line0, mut rebuilt) = {
            let txn = self
                .find_transaction(index)
                .ok_or(EditError::TransactionNotFound(index.0))?;
            (
                txn.source_file.clone(),
                txn.source_span.0.line.saturating_sub(1) as usize,
                txn.clone(),
            )
        };
        rebuilt.description = new_description.to_string();

        let rope = self.rope_for(&source_file)?;
        let lines = Lines::new(rope);
        let (start, content_end) = lines.content_range(header_line0)?;
        let new_header = rebuild_header(&rebuilt, &lines, header_line0)?;
        let expected = self.journal.transactions.len();
        let mut candidate = rope.clone();
        candidate.remove(start..content_end);
        candidate.insert(start, &new_header);
        let reparsed = self.validate_with(&source_file, &candidate, expected)?;

        // The reparse reassigns file-order indices, but a header-only rewrite adds
        // or removes no lines and moves no transaction, so the target keeps its
        // `tindex`. Guard that its description round-tripped exactly.
        let updated = reparsed
            .transactions
            .iter()
            .find(|t| t.index == index)
            .ok_or_else(|| {
                EditError::Internal("edited transaction not found after reparse".into())
            })?;
        // The dates are carried across as the user's own bytes, so a mismatch
        // here would mean the rewrite landed on the wrong line.
        if updated.description != new_description
            || updated.date != rebuilt.date
            || updated.date2 != rebuilt.date2
        {
            return Err(EditError::RoundTripMismatch);
        }

        self.commit(&source_file, candidate, reparsed)
    }

    /// Change **only** the clearing status of the transaction with `index`.
    ///
    /// Rewrites just the transaction's header line (`source_span.0.line`) with a
    /// header rebuilt from the transaction carrying `status` — same date,
    /// secondary date, code, description, and trailing `; comment`. Every posting
    /// line below (accounts, amounts, comments, and whitespace) is left
    /// byte-identical, and the header line's own terminator is preserved.
    /// [`Status::Unmarked`] removes any `*`/`!` marker; [`Status::Cleared`] /
    /// [`Status::Pending`] add or change it. The mutated text is re-parsed to
    /// validate; the edit is refused (with `self` untouched) unless the re-parsed
    /// transaction's status is exactly `status`.
    ///
    /// # Errors
    /// [`EditError::TransactionNotFound`], [`EditError::ParseInvalidAfterEdit`],
    /// [`EditError::RoundTripMismatch`], or [`EditError::Internal`].
    pub fn set_status(&mut self, index: Tindex, status: Status) -> Result<(), EditError> {
        let (source_file, header_line0, mut rebuilt) = {
            let txn = self
                .find_transaction(index)
                .ok_or(EditError::TransactionNotFound(index.0))?;
            (
                txn.source_file.clone(),
                txn.source_span.0.line.saturating_sub(1) as usize,
                txn.clone(),
            )
        };
        rebuilt.status = status;

        let rope = self.rope_for(&source_file)?;
        let lines = Lines::new(rope);
        let (start, content_end) = lines.content_range(header_line0)?;
        let new_header = rebuild_header(&rebuilt, &lines, header_line0)?;
        let expected = self.journal.transactions.len();
        let mut candidate = rope.clone();
        candidate.remove(start..content_end);
        candidate.insert(start, &new_header);
        let reparsed = self.validate_with(&source_file, &candidate, expected)?;

        // A header-only rewrite adds or removes no lines and moves no transaction,
        // so the target keeps its `tindex`. Guard that its status round-tripped
        // exactly (a marker smuggled into the description can't silently apply).
        let updated = reparsed
            .transactions
            .iter()
            .find(|t| t.index == index)
            .ok_or_else(|| {
                EditError::Internal("edited transaction not found after reparse".into())
            })?;
        if updated.status != status
            || updated.date != rebuilt.date
            || updated.date2 != rebuilt.date2
        {
            return Err(EditError::RoundTripMismatch);
        }

        self.commit(&source_file, candidate, reparsed)
    }

    /// Change **only** the account of the `posting_index`-th posting of the
    /// transaction with `index`.
    ///
    /// Replaces just the account token on that posting's source line, preserving
    /// the line's indentation, any `*`/`!` posting status marker, the amount,
    /// balance assertion, trailing comment, and the exact whitespace between them
    /// (only the account name's characters change, so the amount column may shift
    /// but no other byte moves).
    ///
    /// # Locating the posting line
    /// Postings carry no stored source line, so the line is found by scanning the
    /// transaction's span and taking the `posting_index`-th **posting line** —
    /// an indented, non-blank line whose first non-whitespace character is not
    /// `;` (mirroring the parser, which treats every such line in a transaction
    /// body as a posting and skips `;` comment lines). On that line the current
    /// account name is then located as the first substring after the indentation
    /// and status marker (skipping a leading `(`/`[` virtual bracket), and only
    /// those characters are replaced.
    ///
    /// ## Limitation with duplicate accounts
    /// The account is mapped to its line by **ordinal position** (the Nth posting
    /// line is the Nth posting), which is correct as long as each posting occupies
    /// exactly one line (always true for parsed postings). The current-account
    /// text match is only a corroborating guard: if a transaction has two postings
    /// with the *same* account name, that guard cannot distinguish them, but the
    /// positional mapping still selects the right line.
    ///
    /// # Errors
    /// [`EditError::TransactionNotFound`], [`EditError::PostingNotFound`],
    /// [`EditError::ParseInvalidAfterEdit`], [`EditError::RoundTripMismatch`], or
    /// [`EditError::Internal`]. On any error `self` is unchanged.
    pub fn set_posting_account(
        &mut self,
        index: Tindex,
        posting_index: usize,
        new_account: &str,
    ) -> Result<(), EditError> {
        let (source_file, header_line0, scan_end0, current_account) = {
            let txn = self
                .find_transaction(index)
                .ok_or(EditError::TransactionNotFound(index.0))?;
            let posting = txn
                .postings
                .get(posting_index)
                .ok_or(EditError::PostingNotFound {
                    txn: index.0,
                    posting: posting_index,
                })?;
            (
                txn.source_file.clone(),
                txn.source_span.0.line.saturating_sub(1) as usize,
                txn.source_span.1.line.saturating_sub(1) as usize,
                posting.account.0.clone(),
            )
        };
        let rope = self.rope_for(&source_file)?;
        let lines = Lines::new(rope);
        let line0 =
            nth_posting_line(&lines, header_line0, scan_end0, posting_index).ok_or_else(|| {
                EditError::Internal(format!(
                    "could not locate posting #{posting_index} of transaction #{} in {}",
                    index.0,
                    source_file.display()
                ))
            })?;
        let (start, end) = locate_account_token(&lines, line0, &current_account)?;

        let expected = self.journal.transactions.len();
        let mut candidate = rope.clone();
        candidate.remove(start..end);
        candidate.insert(start, new_account);
        let reparsed = self.validate_with(&source_file, &candidate, expected)?;

        // Same-count, same-order reparse ⇒ the target keeps its `tindex` and
        // posting order; guard that the account round-tripped exactly.
        let updated = reparsed
            .transactions
            .iter()
            .find(|t| t.index == index)
            .and_then(|t| t.postings.get(posting_index))
            .ok_or_else(|| EditError::Internal("edited posting not found after reparse".into()))?;
        if updated.account.0 != new_account {
            return Err(EditError::RoundTripMismatch);
        }

        self.commit(&source_file, candidate, reparsed)
    }

    /// Replace the whole transaction with `index` **in place** with `txn`.
    ///
    /// The transaction's `source_span` (header through last posting, inclusive of
    /// their trailing newlines) is replaced with [`format_transaction`]`(txn)` at
    /// the same file position, so every neighbor's source text stays
    /// byte-identical. Because `format_transaction` emits each posting's `comment`
    /// (and the header comment), a full replace built from a comment-carrying
    /// [`Transaction`] does not drop comments.
    ///
    /// Like [`add_transaction`](Self::add_transaction) this rejects a posting with
    /// multiple commodity amounts, requires the transaction to balance, re-parses
    /// to validate, and guards that the re-parsed transaction round-trips to the
    /// intended value. On any error `self` is unchanged.
    ///
    /// # Errors
    /// [`EditError::TransactionNotFound`], [`EditError::Unbalanced`],
    /// [`EditError::Unsupported`], [`EditError::ParseInvalidAfterEdit`],
    /// [`EditError::RoundTripMismatch`], or [`EditError::Internal`].
    pub fn replace_transaction(
        &mut self,
        index: Tindex,
        txn: &Transaction,
    ) -> Result<(), EditError> {
        if txn.postings.iter().any(|p| p.amounts.len() > 1) {
            return Err(EditError::Unsupported(
                "a posting carries multiple commodity amounts".to_string(),
            ));
        }
        if !is_balanced(&txn.postings)? {
            return Err(EditError::Unbalanced);
        }
        let (source_file, start, end, elided) = {
            let existing = self
                .find_transaction(index)
                .ok_or(EditError::TransactionNotFound(index.0))?;
            let rope = self.rope_for(&existing.source_file)?;
            let lines = Lines::new(rope);
            let (start, end) = txn_char_range(&lines, existing)?;
            (
                existing.source_file.clone(),
                start,
                end,
                elided_postings(&lines, existing),
            )
        };
        // Re-emit a leg the user had left blank as blank, rather than freezing
        // the parser's inferred value into their books.
        let written = restore_elisions(txn, &elided);

        let rope = self.rope_for(&source_file)?;
        let body = with_line_terminator(&format_transaction(&written), dominant_terminator(rope));
        let expected = self.journal.transactions.len();
        let mut candidate = rope.clone();
        candidate.remove(start..end);
        candidate.insert(start, &body);
        let reparsed = self.validate_with(&source_file, &candidate, expected)?;

        // The replacement header starts at `start`; locate the transaction now on
        // that line IN ITS FILE and apply the same balance + round-trip guards as
        // an add.
        let replaced = locate_in_file(&candidate, &reparsed, &source_file, start)?;
        if !is_balanced(&replaced.postings)? {
            return Err(EditError::Unbalanced);
        }
        if !transactions_equivalent(txn, replaced) {
            return Err(EditError::RoundTripMismatch);
        }

        self.commit(&source_file, candidate, reparsed)
    }

    /// Decide which file the new transaction lands in and where within that file's
    /// rope, plus the separating text. [`InsertPosition::Append`] (and an empty
    /// journal) append to the main file; [`InsertPosition::DateOrdered`] places the
    /// row next to its chronological neighbors ACROSS all files (see the enum docs).
    ///
    /// # Choosing between the two neighbors (DL-6)
    /// When the chronological predecessor and successor live in DIFFERENT files,
    /// this choice decides which `include` receives the row. Always following the
    /// predecessor was wrong: with `include 2025.journal` / `include
    /// 2026.journal`, a new `2026-01-05` whose predecessor is `2025-11-01` was
    /// written into **`2025.journal`**, breaking the per-year organisation and
    /// `hledger check ordereddates`.
    ///
    /// Always following the successor is equally wrong in the mirror case: a new
    /// `2025-12-15` between the same two neighbors belongs in `2025.journal`.
    /// Neither neighbor is inherently right, so the tiebreak is the one fact that
    /// distinguishes them — which neighbor's date shares a longer prefix with the
    /// new one. On ISO dates that reads directly as *same year, then same month,
    /// then same day*, which is what a per-year or per-month `include` layout
    /// encodes, and it needs no assumption about file NAMES. A tie keeps the
    /// predecessor, preserving the single-file behaviour.
    fn placement_for(
        &self,
        body: &str,
        txn: &Transaction,
        position: InsertPosition,
    ) -> Result<Placement, EditError> {
        if position == InsertPosition::Append || self.journal.transactions.is_empty() {
            return self.append_to_main(body);
        }
        // PREDECESSOR: the latest existing transaction dated `<=` the new one. On a
        // date tie `max_by` yields the LAST such in file order, so the new row joins
        // the end of its same-date/period group.
        let predecessor = self
            .journal
            .transactions
            .iter()
            .filter(|existing| existing.date.as_str() <= txn.date.as_str())
            .max_by(|a, b| a.date.cmp(&b.date));
        // SUCCESSOR: the earliest existing transaction dated `>` the new one, the
        // FIRST such in file order on a date tie.
        let successor = self
            .journal
            .transactions
            .iter()
            .filter(|existing| existing.date.as_str() > txn.date.as_str())
            .min_by(|a, b| a.date.cmp(&b.date));

        match (predecessor, successor) {
            // Both neighbors in one file (the common single-file case), or no
            // successor at all: follow the predecessor.
            (Some(predecessor), None) => self.insert_after(predecessor, body),
            (Some(predecessor), Some(successor)) => {
                let successor_is_closer_in_period = predecessor.source_file
                    != successor.source_file
                    && shared_date_prefix(&successor.date, &txn.date)
                        > shared_date_prefix(&predecessor.date, &txn.date);
                if successor_is_closer_in_period {
                    self.insert_before(successor, body)
                } else {
                    self.insert_after(predecessor, body)
                }
            }
            // No predecessor ⇒ the new row precedes every existing one: place it
            // before the successor, which is then the earliest transaction.
            (None, Some(successor)) => self.insert_before(successor, body),
            (None, None) => Err(EditError::Internal(
                "a non-empty journal has neither a predecessor nor a successor".into(),
            )),
        }
    }

    /// Append `body` to the MAIN file with exactly one blank separator line.
    fn append_to_main(&self, body: &str) -> Result<Placement, EditError> {
        let main = self.rope_for(&self.main_key)?;
        Ok(Placement {
            file_key: self.main_key.clone(),
            insertion: append_insertion(main, body).with_terminator(dominant_terminator(main)),
        })
    }

    /// Insert `body` immediately after `predecessor`'s span, IN THE PREDECESSOR'S
    /// OWN file. When the predecessor is the last content in that file this uses the
    /// same end-of-file/trailing-newline handling as an append; otherwise it opens a
    /// fresh blank separator line and lets the existing blank below separate the new
    /// row from the next transaction.
    ///
    /// Since PARSE-7 a transaction's `source_span` already runs past its trailing
    /// indented comment lines, so anchoring on the span's end keeps a
    /// `; subscription: false` written under the last posting attached to the
    /// transaction it belongs to.
    fn insert_after(&self, predecessor: &Transaction, body: &str) -> Result<Placement, EditError> {
        let rope = self.rope_for(&predecessor.source_file)?;
        let (_, end) = txn_char_range(&Lines::new(rope), predecessor)?;
        let insertion = if end >= rope.len_chars() {
            append_insertion(rope, body)
        } else {
            Insertion {
                offset: end,
                prefix: "\n".to_string(),
                body: body.to_string(),
            }
        };
        Ok(Placement {
            file_key: predecessor.source_file.clone(),
            insertion: insertion.with_terminator(dominant_terminator(rope)),
        })
    }

    /// Insert `body` immediately before `successor`'s header, IN THE SUCCESSOR's
    /// OWN file, with a trailing blank separator; whatever precedes the anchor
    /// (directives or a blank) stays above the new row.
    ///
    /// The anchor backs up over any comment block written directly above the
    /// header (DL-6): a `; note about this trip` line describes the transaction
    /// under it, and inserting between the two silently re-parents the note to
    /// the new row.
    fn insert_before(&self, successor: &Transaction, body: &str) -> Result<Placement, EditError> {
        let rope = self.rope_for(&successor.source_file)?;
        let lines = Lines::new(rope);
        let (start, _) = txn_char_range(&lines, successor)?;
        let header_line0 = successor.source_span.0.line.saturating_sub(1) as usize;
        let anchor_line0 = comment_block_start(&lines, header_line0);
        let offset = if anchor_line0 == header_line0 {
            start
        } else {
            lines.line_to_char(anchor_line0)?
        };
        Ok(Placement {
            file_key: successor.source_file.clone(),
            insertion: Insertion {
                offset,
                prefix: String::new(),
                body: format!("{body}\n"),
            }
            .with_terminator(dominant_terminator(rope)),
        })
    }

    /// Save every file whose rope changed since load back to disk, atomically,
    /// refusing (and writing NOTHING) if any changed file was modified externally.
    ///
    /// First every dirty file is re-read and its content hash compared to its
    /// load-time fingerprint. If ANY dirty file changed on disk this returns
    /// [`EditError::ExternalChange`] before writing anything. Otherwise each
    /// dirty file is written to a temp file in its own directory, `fsync`ed, and
    /// `rename`d over the target, and its fingerprint is refreshed and its dirty
    /// flag cleared. Unchanged files (including untouched includes) are never
    /// rewritten.
    ///
    /// # The remaining race, and why there is no lock
    /// The two passes are not atomic against another writer, so a write landing
    /// between a file's check and its rename is still lost. Each file is
    /// therefore re-checked immediately before it is written, which narrows the
    /// window from "every check plus every write" to the microseconds between
    /// one file's hash and its own rename.
    ///
    /// It is deliberately not closed with an `flock`/lockfile. An advisory lock
    /// binds only processes that take it, and the writers that actually race
    /// here — Emacs, vim, an editor's autosave, `rsync` — take none, so it would
    /// not prevent the loss. What it *would* add is a way for a crashed or
    /// suspended GUI holding a long-lived editor to wedge every CLI invocation
    /// against the user's own books. The content hash refuses to clobber and
    /// costs nothing; that is the better trade for an irreplaceable record.
    ///
    /// # Errors
    /// [`EditError::ExternalChange`] or [`EditError::Io`].
    pub fn save(&mut self) -> Result<(), EditError> {
        let dirty: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|(_, file)| file.dirty)
            .map(|(key, _)| key.clone())
            .collect();

        // Pass 1: external-change guard for every dirty file — write nothing if
        // any changed.
        for key in &dirty {
            let file = self.files.get(key).ok_or_else(|| {
                EditError::Internal(format!(
                    "no loaded buffer for source file {}",
                    key.display()
                ))
            })?;
            if file_changed_externally(&file.path, &file.fingerprint)? {
                return Err(EditError::ExternalChange);
            }
        }

        // Pass 2: atomically write each dirty file and refresh its fingerprint,
        // re-checking each one immediately before its own write so the TOCTOU
        // window is a single file's hash-to-rename rather than the whole pass.
        for key in &dirty {
            let (path, new_text) = {
                let file = self.files.get(key).ok_or_else(|| {
                    EditError::Internal(format!(
                        "no loaded buffer for source file {}",
                        key.display()
                    ))
                })?;
                if file_changed_externally(&file.path, &file.fingerprint)? {
                    return Err(EditError::ExternalChange);
                }
                (file.path.clone(), file.rope.to_string())
            };
            atomic_write(&path, new_text.as_bytes())?;
            let fingerprint = Fingerprint::of_bytes(new_text.as_bytes());
            let file = self.files.get_mut(key).ok_or_else(|| {
                EditError::Internal(format!(
                    "no loaded buffer for source file {}",
                    key.display()
                ))
            })?;
            file.fingerprint = fingerprint;
            file.dirty = false;
        }
        Ok(())
    }
}

/// Whether the file at `path` differs on disk from its load-time `fingerprint`.
///
/// The file is **always** re-read and its content hashed (DL-3). An unchanged
/// mtime used to short-circuit this, which any mtime-preserving write defeats:
/// `cp -p`, `rsync -t`, `tar -x`, a snapshot or backup restore, an editor that
/// restores mtime, or simply two writes inside one tick on a coarse-granularity
/// filesystem (exFAT, SMB, older NFS). Reproduced with `touch -r`: an externally
/// added transaction was silently wiped by a `save()` that reported success.
///
/// Journals are small and a save is already dominated by a full reparse, so the
/// read costs nothing worth having a guess for. `mtime` is still carried on the
/// [`Fingerprint`], but only as a record — never as a verdict.
fn file_changed_externally(path: &Path, fingerprint: &Fingerprint) -> Result<bool, EditError> {
    let current = std::fs::read(path)?;
    Ok(!Fingerprint::of_bytes(&current).content_matches(fingerprint))
}

// ---------------------------------------------------------------------------
// Line addressing (DL-1)
// ---------------------------------------------------------------------------

/// An **LF-only** line index over a rope: the char offset at which each line
/// starts, where a line ends at `\n` and at nothing else.
///
/// # Why not ropey's own line index
/// The parser numbers lines with [`str::lines`], which ends a line at `\n` only
/// (a paired `\r` is stripped from the line's *content*, not counted as a second
/// break). Ropey follows Unicode instead and **additionally** treats `U+000B`
/// VT, `U+000C` FF, a lone `U+000D` CR, `U+0085` NEL, `U+2028` LS and `U+2029`
/// PS as line breaks. One such character anywhere in a file therefore pushed
/// every `source_span.line` below it out of step with the buffer, and every edit
/// addressed the wrong lines: deleting a transaction destroyed a *posting
/// belonging to a different transaction* and left a journal hledger refuses to
/// load, with no error from the write path (DL-1).
///
/// The triggers are ordinary, not exotic — `U+000C` is the standard Emacs/ledger
/// `^L` page separator, and a lone `\r` is routine CSV-import residue.
///
/// # Why this shape of fix
/// Line numbers stay the addressing scheme; only their *interpretation* moves,
/// from ropey's definition to the parser's. The alternatives were worse:
/// recording byte offsets in `source_span` would change a field the wire layer
/// publishes as hledger's `tsourcepos` and force a model change through five
/// files outside this module, and refusing to open a file containing a non-LF
/// break would make a journal hledger reads perfectly un-openable. Character
/// offsets — all that [`Rope::insert`]/[`Rope::remove`] consume — mean the same
/// thing under either definition, so nothing but the line↔char mapping had to
/// change.
struct Lines<'a> {
    rope: &'a Rope,
    /// Char offset of the start of each line: always begins with `0` and gains
    /// an entry after every `\n`. A text ending in `\n` therefore has a final
    /// empty line, matching both ropey's `len_lines` convention (so
    /// one-past-the-end addressing keeps working) and `str::lines` numbering.
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(rope: &'a Rope) -> Self {
        let mut starts = vec![0usize];
        let mut offset = 0usize;
        for chunk in rope.chunks() {
            for ch in chunk.chars() {
                offset += 1;
                if ch == '\n' {
                    starts.push(offset);
                }
            }
        }
        Self { rope, starts }
    }

    /// The number of lines, on ropey's convention (a trailing `\n` yields a
    /// final empty line; the empty buffer has one line).
    fn len(&self) -> usize {
        self.starts.len()
    }

    /// The char offset at which 0-based `line0` starts. `line0 == len()` is
    /// accepted and yields end-of-buffer, so a transaction that ends at EOF
    /// needs no special case.
    fn line_to_char(&self, line0: usize) -> Result<usize, EditError> {
        match self.starts.get(line0) {
            Some(&start) => Ok(start),
            None if line0 == self.starts.len() => Ok(self.rope.len_chars()),
            None => Err(EditError::Internal(format!(
                "line {line0} is out of range ({} lines)",
                self.starts.len()
            ))),
        }
    }

    /// The 0-based line containing char offset `char_idx`.
    fn char_to_line(&self, char_idx: usize) -> Result<usize, EditError> {
        if char_idx > self.rope.len_chars() {
            return Err(EditError::Internal(format!(
                "char {char_idx} is past the end of the buffer"
            )));
        }
        // `starts[0] == 0 <= char_idx` always holds, so this cannot underflow.
        Ok(self.starts.partition_point(|&start| start <= char_idx) - 1)
    }

    /// The text of 0-based `line0` including its terminator, or `None` past the
    /// end of the buffer.
    fn text(&self, line0: usize) -> Option<String> {
        let start = *self.starts.get(line0)?;
        let end = self
            .starts
            .get(line0 + 1)
            .copied()
            .unwrap_or_else(|| self.rope.len_chars());
        Some(self.rope.slice(start..end).to_string())
    }

    /// Whether `line0` exists and is a real blank line (has content — a newline
    /// and/or whitespace — but trims to empty). The phantom empty line after a
    /// trailing newline (zero chars) is not counted.
    fn is_blank(&self, line0: usize) -> bool {
        self.text(line0)
            .is_some_and(|text| !text.is_empty() && text.trim().is_empty())
    }

    /// Whether `line0` exists, is indented (starts with a space/tab), and is
    /// non-blank — i.e. a trailing in-transaction line (a posting-block comment)
    /// that belongs to the preceding transaction.
    fn is_indented_content(&self, line0: usize) -> bool {
        self.text(line0)
            .is_some_and(|text| text.starts_with([' ', '\t']) && !text.trim().is_empty())
    }

    /// Whether `line0` is a comment line — `;` or `#` after any indentation.
    /// hledger's third comment marker, a column-1 `*` org heading, is
    /// deliberately excluded: an org outline structures the file, and treating
    /// its headings as a transaction's note would move rows under the wrong one.
    fn is_comment(&self, line0: usize) -> bool {
        self.text(line0).is_some_and(|text| {
            let trimmed = text.trim_start();
            trimmed.starts_with([';', '#'])
        })
    }

    /// Whether `line0` is a posting line: indented, non-blank, and not a `;`
    /// comment line (mirrors the parser, which treats every indented non-`;`
    /// line in a transaction body as a posting).
    fn is_posting(&self, line0: usize) -> bool {
        self.text(line0).is_some_and(|text| {
            let trimmed = text.trim_start();
            text.starts_with([' ', '\t']) && !trimmed.is_empty() && !trimmed.starts_with(';')
        })
    }

    /// The char range `[start, content_end)` of line `line0`'s content,
    /// excluding its trailing line terminator (`\r\n`/`\n`, or none at EOF).
    /// Used to rewrite a line's text while preserving its exact terminator.
    fn content_range(&self, line0: usize) -> Result<(usize, usize), EditError> {
        let start = self.line_to_char(line0)?;
        let text = self
            .text(line0)
            .ok_or_else(|| EditError::Internal(format!("line {line0} is out of range")))?;
        Ok((start, start + strip_terminator(&text).chars().count()))
    }
}

/// `text` without its final line terminator: one `\n`, and the `\r` of a `\r\n`
/// pair. Exactly what `str::lines` removes, so a line's content here is the
/// content the parser saw.
fn strip_terminator(text: &str) -> &str {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.strip_suffix('\r').unwrap_or(text)
}

/// The half-open rope char range `[start, end)` covering a transaction's
/// `source_span` — the header line through the line after its last body line.
/// [`Lines::line_to_char`] accepts a one-past-the-end line index, so a final
/// transaction that ends at EOF is handled without special-casing.
fn txn_char_range(lines: &Lines, txn: &Transaction) -> Result<(usize, usize), EditError> {
    let len_lines = lines.len();
    let start_line0 = txn.source_span.0.line.saturating_sub(1) as usize;
    let end_line0 = txn.source_span.1.line.saturating_sub(1) as usize;
    let start = lines.line_to_char(start_line0.min(len_lines))?;
    let end = lines.line_to_char(end_line0.min(len_lines))?;
    Ok((start, end))
}

/// The 0-based line of the `posting_index`-th posting of the transaction whose
/// header is on line `header_line0`, scanning posting lines in the half-open line
/// range `(header_line0, scan_end0)`. Blank and `;` comment lines are skipped, so
/// postings map to source lines by ordinal position.
fn nth_posting_line(
    lines: &Lines,
    header_line0: usize,
    scan_end0: usize,
    posting_index: usize,
) -> Option<usize> {
    let end = scan_end0.min(lines.len());
    (header_line0 + 1..end)
        .filter(|line0| lines.is_posting(*line0))
        .nth(posting_index)
}

/// The rope char range `[start, end)` of the account token on posting line
/// `line0`, matched as the first occurrence of `current_account` after the line's
/// indentation and any `*`/`!` status marker (skipping a leading `(`/`[` virtual
/// bracket). Only the account name is spanned, so replacing it leaves the marker,
/// amount, assertion, comment, and whitespace untouched.
fn locate_account_token(
    lines: &Lines,
    line0: usize,
    current_account: &str,
) -> Result<(usize, usize), EditError> {
    let text = lines
        .text(line0)
        .ok_or_else(|| EditError::Internal(format!("posting line {line0} is out of range")))?;
    let content = strip_terminator(&text);

    let indent_end = content
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(content.len());
    let after_indent = &content[indent_end..];
    let field_start = if after_indent.starts_with(['*', '!']) {
        let marker_end = indent_end + 1;
        let rest = &content[marker_end..];
        marker_end + (rest.len() - rest.trim_start_matches([' ', '\t']).len())
    } else {
        indent_end
    };

    let region = content.get(field_start..).unwrap_or("");
    let rel = region.find(current_account).ok_or_else(|| {
        EditError::Internal(format!(
            "account '{current_account}' not found on posting line {}",
            line0 + 1
        ))
    })?;
    let byte_start = field_start + rel;
    let byte_end = byte_start + current_account.len();
    let start_chars = content[..byte_start].chars().count();
    let end_chars = content[..byte_end].chars().count();
    let line_start = lines.line_to_char(line0)?;
    Ok((line_start + start_chars, line_start + end_chars))
}

fn count_trailing_newlines(rope: &Rope) -> usize {
    let len = rope.len_chars();
    let mut count = 0;
    while count < len && rope.get_char(len - 1 - count) == Some('\n') {
        count += 1;
    }
    count
}

// ---------------------------------------------------------------------------
// Elided amounts
// ---------------------------------------------------------------------------

/// Which of `txn`'s postings were written **without an amount** in the source,
/// by posting index.
///
/// Read back from the source line rather than from the model: by the time a
/// [`Posting`] exists the parser has already filled its `amounts` in, and adding
/// an `elided` flag to the model would force a change through five files outside
/// this module (`wire.rs`, `reports/`, `holdings/`, and the server's edit API)
/// for a fact the source text states plainly.
fn elided_postings(lines: &Lines, txn: &Transaction) -> Vec<bool> {
    let header_line0 = txn.source_span.0.line.saturating_sub(1) as usize;
    let scan_end0 = txn.source_span.1.line.saturating_sub(1) as usize;
    (0..txn.postings.len())
        .map(|posting_index| {
            nth_posting_line(lines, header_line0, scan_end0, posting_index)
                .and_then(|line0| lines.text(line0))
                .is_some_and(|text| {
                    crate::parse::posting_line_elides_amount(strip_terminator(&text))
                })
        })
        .collect()
}

/// `txn` with the amount cleared on any posting whose ORIGINAL source line
/// elided it, so a full replace writes the leg back blank instead of freezing
/// the parser's inferred value into the file.
///
/// # Why this is safe
/// A caller reaches here only after [`is_balanced`], which requires every
/// commodity in the group to sum to exactly zero. Blanking one leg of such a
/// group therefore re-infers the identical value on the next parse, and the
/// round-trip guard still compares the reparsed amounts against the caller's
/// explicit ones.
///
/// # When it declines
/// Nothing is cleared unless the posting count is unchanged (otherwise index `i`
/// no longer names the same leg), the group has no elided posting already, and
/// exactly one posting in the group was elided before. Any ambiguity leaves the
/// amounts explicit — the status quo, never a guess.
fn restore_elisions<'a>(txn: &'a Transaction, elided: &[bool]) -> Cow<'a, Transaction> {
    if elided.len() != txn.postings.len() {
        return Cow::Borrowed(txn);
    }
    let targets: Vec<usize> = [PostingType::Regular, PostingType::BalancedVirtual]
        .into_iter()
        .filter_map(|ptype| {
            let group = || {
                txn.postings
                    .iter()
                    .enumerate()
                    .filter(move |(_, p)| p.ptype == ptype)
            };
            if group().any(|(_, p)| p.amounts.is_empty()) {
                return None;
            }
            let mut was_elided = group().filter(|(i, _)| elided[*i]).map(|(i, _)| i);
            match (was_elided.next(), was_elided.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        })
        .collect();
    if targets.is_empty() {
        return Cow::Borrowed(txn);
    }
    let mut restored = txn.clone();
    for index in targets {
        restored.postings[index].amounts.clear();
    }
    Cow::Owned(restored)
}

/// How many leading characters two ISO dates share — 10 for the same day, 7 for
/// the same month, 4 for the same year, 0 for different millennia.
///
/// Both dates are normalized `YYYY-MM-DD` ASCII by the time they reach here, so
/// byte comparison is character comparison.
fn shared_date_prefix(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// The first line of the comment block written directly above the transaction
/// header on `header_line0`, or `header_line0` itself when there is none.
///
/// A `;`/`#` line sitting immediately above a header is a note about the
/// transaction below it, so a date-ordered insert has to go ABOVE the whole
/// block rather than between the note and what it describes (DL-6).
///
/// A block that runs unbroken to the top of the file is left alone: that is the
/// file's own preamble (`; My journal`), not a note about whichever transaction
/// happens to come first, and pushing a new row above it would be the worse of
/// the two rewrites.
fn comment_block_start(lines: &Lines, header_line0: usize) -> usize {
    let first = (0..header_line0)
        .rev()
        .take_while(|line0| lines.is_comment(*line0))
        .last()
        .unwrap_or(header_line0);
    if first == 0 { header_line0 } else { first }
}

/// The line terminator to write into `rope`: `\r\n` when CRLF lines outnumber
/// bare-LF ones, else `\n`.
///
/// Inserting an LF-only transaction into a CRLF journal left a mixed-terminator
/// file, which the next whitespace-normalising tool rewrites wholesale — turning
/// a one-transaction edit into a whole-file diff (DL-6).
fn dominant_terminator(rope: &Rope) -> &'static str {
    let (crlf, lf, _) = rope.chunks().flat_map(str::chars).fold(
        (0usize, 0usize, false),
        |(crlf, lf, prev_cr), ch| match ch {
            '\n' if prev_cr => (crlf + 1, lf, false),
            '\n' => (crlf, lf + 1, false),
            _ => (crlf, lf, ch == '\r'),
        },
    );
    if crlf > lf { "\r\n" } else { "\n" }
}

/// `text` with every `\n` rewritten to `terminator`. The formatter emits `\n`,
/// so this is a no-op for an LF file.
fn with_line_terminator(text: &str, terminator: &str) -> String {
    if terminator == "\n" {
        text.to_string()
    } else {
        text.replace('\n', terminator)
    }
}

/// The pieces of a rope insertion: insert `prefix` then `body` at `offset`.
struct Insertion {
    offset: usize,
    prefix: String,
    body: String,
}

impl Insertion {
    /// This insertion with its line terminators rewritten to `terminator`, so a
    /// CRLF journal keeps CRLF endings throughout.
    fn with_terminator(self, terminator: &str) -> Self {
        Self {
            offset: self.offset,
            prefix: with_line_terminator(&self.prefix, terminator),
            body: with_line_terminator(&self.body, terminator),
        }
    }
}

/// A resolved add placement: which file's rope to edit, and the [`Insertion`]
/// within it. Lets [`JournalEditor::add_transaction`] write a date-ordered row
/// into the `include`d file that holds its neighbors, not just the main file.
struct Placement {
    file_key: PathBuf,
    insertion: Insertion,
}

/// The [`Insertion`] that appends `body` at the end of `rope` with exactly one
/// blank separator line, matching whatever trailing-newline shape is already
/// present (no newline ⇒ close the last line and add a blank; one ⇒ add a blank;
/// already blank-terminated ⇒ none).
fn append_insertion(rope: &Rope, body: &str) -> Insertion {
    let len = rope.len_chars();
    let prefix = if len == 0 {
        String::new()
    } else {
        match count_trailing_newlines(rope) {
            0 => "\n\n".to_string(),
            1 => "\n".to_string(),
            _ => String::new(),
        }
    };
    Insertion {
        offset: len,
        prefix,
        body: body.to_string(),
    }
}

/// Find the transaction that was just inserted/replaced in the file keyed by
/// `file_key`, by the char offset of its header within that file's candidate
/// rope. Filtering on `file_key` disambiguates transactions that share a
/// `source_span` line number across different files.
fn locate_in_file<'a>(
    candidate: &Rope,
    reparsed: &'a Journal,
    file_key: &Path,
    header_char: usize,
) -> Result<&'a Transaction, EditError> {
    let line0 = Lines::new(candidate).char_to_line(header_char)?;
    let line1 =
        u32::try_from(line0 + 1).map_err(|_| EditError::Internal("line index overflow".into()))?;
    reparsed
        .transactions
        .iter()
        .find(|t| t.source_file == file_key && t.source_span.0.line == line1)
        .ok_or_else(|| EditError::Internal("could not locate the added transaction".into()))
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Format a [`Transaction`] as clean, valid hledger journal text ending in a
/// newline.
///
/// The header is `DATE[=DATE2] [*|!] [(CODE)] [DESCRIPTION] [ ; COMMENT]`; each
/// posting is `    ACCOUNT<pad>AMOUNT`, with a 4-space indent, amounts aligned
/// with at least a 2-space account/amount gap. Amounts render via the amount's
/// own style: `$1234.56` for a left symbol, `1234.56 EUR` for a right one, using
/// the style's decimal mark (so a comma-decimal commodity round-trips), and
/// append `@`/`@@` costs. Digit-group separators are omitted (the plain form
/// always re-parses to the same value). A posting with an empty `amounts` vec is
/// rendered account-only (an elided/inferred posting). Each posting's `comment`
/// is emitted as a trailing `  ; comment` (like the header comment), so a
/// full-replace edit ([`JournalEditor::replace_transaction`]) preserves comments.
///
/// # Example
/// ```
/// use ledgeline_core::edit::format_transaction;
/// use ledgeline_core::model::*;
/// use ledgeline_core::decimal::Dec;
///
/// let dollars = AmountStyle {
///     side: CommoditySide::Left,
///     spaced: false,
///     decimal_mark: Some('.'),
///     digit_groups: None,
///     precision: 2,
/// };
/// let amount = Amount {
///     commodity: Commodity("$".into()),
///     quantity: Dec::new(180_000, 2),
///     style: dollars,
///     cost: None,
/// };
/// let posting = |account: &str, amt: Option<Amount>| Posting {
///     status: Status::Unmarked,
///     ptype: PostingType::Regular,
///     account: AccountName(account.into()),
///     amounts: amt.into_iter().collect(),
///     balance_assertion: None,
///     date: None,
///     date2: None,
///     comment: String::new(),
///     tags: vec![],
/// };
/// let txn = Transaction {
///     index: Tindex(1),
///     date: "2026-07-01".into(),
///     date2: None,
///     status: Status::Cleared,
///     code: String::new(),
///     description: "Landlord | rent".into(),
///     comment: String::new(),
///     preceding_comment: String::new(),
///     tags: vec![],
///     postings: vec![
///         posting("expenses:housing:rent", Some(amount)),
///         posting("assets:bank:checking", None),
///     ],
///     source_span: (SourcePos { line: 1, column: 1 }, SourcePos { line: 3, column: 1 }),
///     source_file: std::path::PathBuf::new(),
/// };
/// assert_eq!(
///     format_transaction(&txn),
///     "2026-07-01 * Landlord | rent\n    \
///      expenses:housing:rent  $1800.00\n    assets:bank:checking\n"
/// );
/// ```
#[must_use]
pub fn format_transaction(txn: &Transaction) -> String {
    let mut out = format_header(txn);
    out.push('\n');
    let amount_col = txn
        .postings
        .iter()
        .filter(|p| !p.amounts.is_empty())
        .map(|p| account_field(p).chars().count())
        .max()
        .unwrap_or(0);
    for posting in &txn.postings {
        for line in format_posting_lines(posting, amount_col) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// A header line rebuilt from `txn`, but keeping the ORIGINAL line's own date
/// token instead of the normalized ISO form (DL-6).
///
/// [`format_header`] renders `txn.date`, which the parser has already normalized
/// to `YYYY-MM-DD`. Asking for a status change on `2026/01/01 * (42) Payee`
/// therefore rewrote the date as well, giving `2026-01-01 ! (42) Payee` — valid
/// hledger, but an unrequested restyling of a line the user wanted exactly one
/// character changed on. That matters beyond taste: it makes a one-character
/// edit show up in `git diff`, and it silently converts a journal written in one
/// date style into a mixed one.
///
/// The date token is the header's first whitespace-delimited token (`DATE` or
/// `DATE=DATE2`) in the original and in the rebuild alike, so carrying the
/// original's across is a pure substitution — the dates cannot change here,
/// because their bytes are never regenerated.
fn rebuild_header(
    txn: &Transaction,
    lines: &Lines,
    header_line0: usize,
) -> Result<String, EditError> {
    let original = lines.text(header_line0).ok_or_else(|| {
        EditError::Internal(format!("header line {header_line0} is out of range"))
    })?;
    let rebuilt = format_header(txn);
    Ok(
        match (
            strip_terminator(&original).split_whitespace().next(),
            rebuilt.split_whitespace().next(),
        ) {
            (Some(original_date), Some(rebuilt_date)) if original_date != rebuilt_date => {
                format!("{original_date}{}", &rebuilt[rebuilt_date.len()..])
            }
            _ => rebuilt,
        },
    )
}

fn format_header(txn: &Transaction) -> String {
    let mut header = txn.date.clone();
    if let Some(date2) = &txn.date2 {
        header.push('=');
        header.push_str(date2);
    }
    match txn.status {
        Status::Cleared => header.push_str(" *"),
        Status::Pending => header.push_str(" !"),
        Status::Unmarked => {}
    }
    if !txn.code.is_empty() {
        header.push_str(" (");
        header.push_str(&txn.code);
        header.push(')');
    }
    if !txn.description.is_empty() {
        header.push(' ');
        header.push_str(&txn.description);
    }
    let comment = txn.comment.trim();
    if !comment.is_empty() {
        header.push_str("  ; ");
        header.push_str(comment);
    }
    header
}

/// The account field of a posting: the (bracketed, for virtuals) account name,
/// prefixed with a `*`/`!` posting status marker when present.
fn account_field(posting: &Posting) -> String {
    let name = match posting.ptype {
        PostingType::Regular => posting.account.0.clone(),
        PostingType::Virtual => format!("({})", posting.account.0),
        PostingType::BalancedVirtual => format!("[{}]", posting.account.0),
    };
    match posting.status {
        Status::Cleared => format!("* {name}"),
        Status::Pending => format!("! {name}"),
        Status::Unmarked => name,
    }
}

fn format_posting_lines(posting: &Posting, amount_col: usize) -> Vec<String> {
    let field = account_field(posting);
    let comment = posting.comment.trim();
    if posting.amounts.is_empty() {
        let mut line = format!("    {field}");
        // hledger accepts an assertion on an amount-less posting
        // (`assets:cash    = $99.00`), and dropping it here would delete the
        // user's reconciliation anchor precisely when the leg is elided — the
        // one case where nothing else on the line records the balance.
        if let Some(assertion) = &posting.balance_assertion {
            line.push_str(&render_assertion(assertion));
        }
        push_comment(&mut line, comment);
        return vec![line];
    }
    posting
        .amounts
        .iter()
        .enumerate()
        .map(|(idx, amount)| {
            let pad = amount_col.saturating_sub(field.chars().count()) + 2;
            let mut line = format!("    {field}{}{}", " ".repeat(pad), render_amount(amount));
            if idx == 0
                && let Some(assertion) = &posting.balance_assertion
            {
                line.push_str(&render_assertion(assertion));
            }
            // A posting is a single source line in hledger, so its comment rides
            // the first amount line (after any balance assertion).
            if idx == 0 {
                push_comment(&mut line, comment);
            }
            line
        })
        .collect()
}

/// Append `  ; comment` to `line` when `comment` is non-empty, matching the
/// header comment's two-space separator. A no-op for an empty comment.
fn push_comment(line: &mut String, comment: &str) {
    if !comment.is_empty() {
        line.push_str("  ; ");
        line.push_str(comment);
    }
}

fn render_amount(amount: &Amount) -> String {
    let mut rendered = render_priced(&amount.commodity, amount.quantity, &amount.style);
    if let Some(cost) = &amount.cost {
        let op = match cost.kind {
            CostKind::Unit => " @ ",
            CostKind::Total => " @@ ",
        };
        rendered.push_str(op);
        rendered.push_str(&render_priced(
            &cost.amount.commodity,
            cost.amount.quantity,
            &cost.amount.style,
        ));
    }
    rendered
}

fn render_assertion(assertion: &BalanceAssertion) -> String {
    let op = match (assertion.total, assertion.inclusive) {
        (true, true) => "==*",
        (true, false) => "==",
        (false, true) => "=*",
        (false, false) => "=",
    };
    format!(
        "  {op} {}",
        render_priced(
            &assertion.amount.commodity,
            assertion.amount.quantity,
            &assertion.amount.style,
        )
    )
}

/// Render `commodity` + `quantity` per `style`'s side/spacing/decimal-mark.
/// Digit grouping is intentionally omitted.
fn render_priced(commodity: &Commodity, quantity: Dec, style: &AmountStyle) -> String {
    let number = render_dec(quantity, style.decimal_mark.unwrap_or('.'));
    let symbol = &commodity.0;
    if symbol.is_empty() {
        return number;
    }
    match (style.side, style.spaced) {
        (CommoditySide::Left, false) => format!("{symbol}{number}"),
        (CommoditySide::Left, true) => format!("{symbol} {number}"),
        (CommoditySide::Right, false) => format!("{number}{symbol}"),
        (CommoditySide::Right, true) => format!("{number} {symbol}"),
    }
}

/// The widest fractional precision [`render_dec`] will lay out.
///
/// Nothing the engine itself produces comes close: [`Dec::parse`] caps at 10
/// places and [`Dec::mul`] at most sums its operands' scales. Only a [`Dec`]
/// built directly from unvalidated wire input can exceed this. 255 is hledger's
/// own maximum displayed precision, so the clamp cannot truncate a value hledger
/// could have written.
const MAX_RENDER_PLACES: u32 = 255;

/// Render a [`Dec`] using `mark` as the decimal separator, exactly (no rounding,
/// no grouping): `Dec::new(180_000, 2)` → `1800.00`, `Dec::new(5, 3)` → `0.005`.
///
/// # Total by construction
/// `Dec::places` is a `u32` but Rust's format width is a `u16`, so the obvious
/// `format!("{digits:0>width$}", width = places + 1)` panics with *"Formatting
/// argument out of range"* once `places >= 65535`. Since [`format_transaction`]
/// returns a `String` and cannot signal failure, this function must be total.
/// Two changes make it so:
///
/// - the zero padding is built with [`str::repeat`], which takes no width
///   argument and so has no such limit;
/// - `places` is clamped to [`MAX_RENDER_PLACES`], bounding the output to a few
///   hundred bytes rather than the ~64 KB single amount line a `places = 65534`
///   request would otherwise commit to the user's books.
///
/// A clamped render is a *different number* from the one passed in, and it is
/// never silently written: it differs from the input by at least a factor of ten,
/// so the reparse-and-compare guard in [`JournalEditor::add_transaction`] /
/// [`JournalEditor::replace_transaction`] rejects it with
/// [`EditError::RoundTripMismatch`]. Validating `places` at the wire boundary
/// remains the primary fix; this is the backstop that keeps the core from
/// panicking or allocating unboundedly if that boundary is ever bypassed.
fn render_dec(value: Dec, mark: char) -> String {
    let negative = value.mantissa < 0;
    let digits = value.mantissa.unsigned_abs().to_string();
    let places = value.places.min(MAX_RENDER_PLACES) as usize;
    let body = if places == 0 {
        digits
    } else {
        // Ensure there is at least one integer digit before the mark.
        let padded = match (places + 1).checked_sub(digits.len()) {
            Some(zeros) if zeros > 0 => "0".repeat(zeros) + &digits,
            _ => digits,
        };
        // `padded.len() > places` holds in both branches, so this cannot wrap,
        // and every byte is ASCII, so the split is on a char boundary.
        let split = padded.len() - places;
        format!("{}{mark}{}", &padded[..split], &padded[split..])
    };
    if negative { format!("-{body}") } else { body }
}

// ---------------------------------------------------------------------------
// Post-edit verification (DL-4)
// ---------------------------------------------------------------------------

/// One transaction's identity for the before/after comparison: the file it lives
/// in, and its exact source text.
type TxnSource = (PathBuf, String);

/// Every transaction's exact source text, in journal order, read out of `texts`
/// (resolved absolute path → whole file contents) using the LF-only line
/// numbering the parser assigned.
///
/// Each file is split into lines ONCE: pulling every transaction's span by
/// rescanning its file from the top would be quadratic in a large journal, and
/// this runs on every edit.
fn transaction_sources<'a>(
    journal: &Journal,
    texts: &'a HashMap<PathBuf, String>,
) -> Result<Vec<TxnSource>, EditError> {
    let mut by_file: HashMap<&PathBuf, Vec<&'a str>> = HashMap::new();
    for txn in &journal.transactions {
        if !by_file.contains_key(&txn.source_file) {
            // Every file a transaction came from is loaded by `load`, so a miss
            // means the edit introduced a source we have no buffer for. Refusing
            // is the safe direction: the guard cannot vouch for what it cannot read.
            let text = texts.get(&txn.source_file).ok_or_else(|| {
                EditError::Internal(format!(
                    "no loaded text for source file {}",
                    txn.source_file.display()
                ))
            })?;
            by_file.insert(&txn.source_file, text.split_inclusive('\n').collect());
        }
    }
    Ok(journal
        .transactions
        .iter()
        .map(|txn| {
            let lines = by_file.get(&txn.source_file).map_or(&[][..], Vec::as_slice);
            let from = (txn.source_span.0.line.saturating_sub(1) as usize).min(lines.len());
            let to = (txn.source_span.1.line.saturating_sub(1) as usize).clamp(from, lines.len());
            (txn.source_file.clone(), lines[from..to].concat())
        })
        .collect())
}

/// Require that AT MOST ONE transaction differs between the before and after
/// states: the two sequences must agree on a common prefix and a common suffix,
/// leaving no more than one transaction between them on either side.
///
/// That is the shape of every operation here — a delete removes one, an add
/// inserts one, an in-place edit rewrites one — so anything wider means the edit
/// reached a transaction it was never asked to touch. Aligning by prefix and
/// suffix rather than by index is what makes this work across the renumbering an
/// insert or delete causes.
///
/// Comparing the text with its final line terminator removed is the one
/// deliberate tolerance: appending to a file that does not end in a newline
/// legitimately gives the last transaction the terminator it lacked. Every other
/// byte must match.
fn check_single_change(before: &[TxnSource], after: &[TxnSource]) -> Result<(), EditError> {
    let same = |a: &TxnSource, b: &TxnSource| {
        a.0 == b.0 && strip_terminator(&a.1) == strip_terminator(&b.1)
    };
    let prefix = before
        .iter()
        .zip(after)
        .take_while(|(a, b)| same(a, b))
        .count();
    let overlap = before.len().min(after.len()) - prefix;
    let suffix = (1..=overlap)
        .take_while(|k| same(&before[before.len() - k], &after[after.len() - k]))
        .count();

    let changed_before = before.len() - prefix - suffix;
    let changed_after = after.len() - prefix - suffix;
    if changed_before <= 1 && changed_after <= 1 {
        return Ok(());
    }
    Err(EditError::Internal(format!(
        "the edit rewrote {changed_before} transaction(s) into {changed_after}; \
         only the addressed transaction may change"
    )))
}

/// Reject an edit that leaves any transaction unbalanced which was not already
/// unbalanced before it.
///
/// Diffing against the before state rather than demanding a wholly balanced
/// journal is the point: a journal that already contains an unbalanced
/// transaction has to stay editable, or one bad row anywhere freezes the entire
/// file — including the edit that would fix it.
fn check_no_new_imbalance(
    before_journal: &Journal,
    before: &[TxnSource],
    after_journal: &Journal,
    after: &[TxnSource],
) -> Result<(), EditError> {
    // If the journal could not be balance-checked BEFORE the edit — an
    // arithmetic overflow summing an amount too wide for `i128` — then this edit
    // is not what broke it, and refusing would leave the user unable to make the
    // very edit that would. The byte-identity guard above still stands.
    let Ok(mut existing) = imbalance_keys(before_journal, before) else {
        return Ok(());
    };
    // Reaching here means the edit itself introduced an unsummable amount:
    // refuse it, as unbalanced-or-unprovable.
    let introduced = imbalance_keys(after_journal, after).map_err(|_| EditError::Unbalanced)?;
    for key in introduced {
        match existing.iter().position(|known| *known == key) {
            Some(at) => {
                existing.swap_remove(at);
            }
            // `Unbalanced` rather than a new variant: the server matches
            // `EditError` exhaustively, and this is precisely what it means.
            None => return Err(EditError::Unbalanced),
        }
    }
    Ok(())
}

/// One key per unbalanced transaction — file, balancing group, and exact source
/// text. Keying on the text rather than the index is what survives the
/// renumbering an insert or delete causes, so a pre-existing failure is still
/// recognised as the same one afterwards.
fn imbalance_keys(journal: &Journal, sources: &[TxnSource]) -> Result<Vec<String>, EditError> {
    check_transaction_balances(journal)?
        .into_iter()
        .map(|failure| {
            let (file, text) = sources.get(failure.transaction_index).ok_or_else(|| {
                EditError::Internal("a balance failure named an unknown transaction".into())
            })?;
            Ok(format!(
                "{}\u{0}{:?}\u{0}{}",
                file.display(),
                failure.group,
                strip_terminator(text)
            ))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Balance + round-trip validation
// ---------------------------------------------------------------------------

/// Whether a set of postings balances.
///
/// Real and balanced-virtual postings balance within their own groups; virtual
/// (`(a)`) postings are excluded. A group with exactly one amount-less posting
/// balances by construction (that leg is inferred on re-parse); two or more
/// amount-less postings in a group cannot be inferred and do not balance. A
/// group where every posting has an amount balances iff every commodity's
/// cost-adjusted total is zero.
fn is_balanced(postings: &[Posting]) -> Result<bool, DecError> {
    for ptype in [PostingType::Regular, PostingType::BalancedVirtual] {
        let group: Vec<&Posting> = postings.iter().filter(|p| p.ptype == ptype).collect();
        let elided = group.iter().filter(|p| p.amounts.is_empty()).count();
        if elided > 1 {
            return Ok(false);
        }
        if elided == 1 {
            continue;
        }
        let mut sums: Vec<(Commodity, Dec)> = Vec::new();
        for posting in &group {
            for amount in &posting.amounts {
                let (commodity, quantity) = amount_contribution(amount)?;
                match sums.iter_mut().find(|(c, _)| *c == commodity) {
                    Some((_, total)) => *total = total.add(quantity)?,
                    None => sums.push((commodity, quantity)),
                }
            }
        }
        if sums.iter().any(|(_, total)| !total.is_zero()) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A posting amount's contribution to the transaction balance: its cost value in
/// the cost commodity when priced, otherwise the amount itself. Mirrors the
/// parser's balancing semantics (`@` multiplies, `@@` is the signed total).
fn amount_contribution(amount: &Amount) -> Result<(Commodity, Dec), DecError> {
    match &amount.cost {
        None => Ok((amount.commodity.clone(), amount.quantity)),
        Some(cost) => {
            let quantity = match cost.kind {
                CostKind::Unit => amount.quantity.mul(cost.amount.quantity)?,
                CostKind::Total => {
                    let magnitude = cost.amount.quantity.abs()?;
                    if amount.quantity.mantissa < 0 {
                        magnitude.neg()?
                    } else {
                        magnitude
                    }
                }
            };
            Ok((cost.amount.commodity.clone(), quantity))
        }
    }
}

/// Whether a re-parsed transaction is semantically the one we intended to add:
/// same header fields and, for each posting, the same account, posting TYPE,
/// balance ASSERTION and (for explicit postings) the same amount values and
/// costs. An elided input posting (empty `amounts`) skips the amount check — its
/// value is inferred on re-parse.
///
/// # The two fields this used to wave through (DL-2)
/// A balance assertion was never compared at all, so a formatter or wire model
/// that dropped `= $99.00` produced a transaction this function called
/// equivalent — and the user's reconciliation anchor was gone for good. Posting
/// type is compared (and always has been), but note what that can and cannot
/// catch: it compares the caller's INTENT against the reparse, so it catches the
/// formatter turning `[budget:env]` into `budget:env`; it cannot catch a request
/// model that never carried the brackets in the first place, because by then
/// `Regular` *is* the intent. That half has to be fixed at the wire boundary.
fn transactions_equivalent(input: &Transaction, parsed: &Transaction) -> bool {
    if input.date != parsed.date
        || input.date2 != parsed.date2
        || input.status != parsed.status
        || input.code != parsed.code
        || input.description != parsed.description
        || input.postings.len() != parsed.postings.len()
    {
        return false;
    }
    input.postings.iter().zip(&parsed.postings).all(|(a, b)| {
        a.account == b.account
            && a.ptype == b.ptype
            && assertions_equivalent(a.balance_assertion.as_ref(), b.balance_assertion.as_ref())
            && (a.amounts.is_empty() || amounts_equivalent(&a.amounts, &b.amounts))
    })
}

/// Whether a re-parsed balance assertion preserves the intended one: same
/// presence, same `=`/`==`/`=*` flavour, and the same asserted amount by value.
///
/// `position` is deliberately excluded — the input's is whatever the caller
/// happened to build, while the parsed one records where the `=` actually landed
/// in the file, so they differ on every successful edit.
fn assertions_equivalent(
    input: Option<&BalanceAssertion>,
    parsed: Option<&BalanceAssertion>,
) -> bool {
    match (input, parsed) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.inclusive == b.inclusive
                && a.total == b.total
                && amount_value_equivalent(&a.amount, &b.amount)
        }
        _ => false,
    }
}

fn amounts_equivalent(input: &[Amount], parsed: &[Amount]) -> bool {
    input.len() == parsed.len()
        && input
            .iter()
            .zip(parsed)
            .all(|(x, y)| amount_value_equivalent(x, y) && costs_equivalent(x, y))
}

/// Whether a re-parsed amount preserves the intended commodity **and value** —
/// the round-trip guard's core comparison, deliberately tolerant of resolutions
/// the parser performs legitimately while still catching real corruption.
///
/// - **Quantity** is compared by numeric VALUE. [`Dec`] equality already ignores
///   scale (`-50` equals `-50.00`), so a value formatted as a bare number and
///   re-parsed at a `D`-directive's precision does not trip the guard — whereas a
///   decimal-mark corruption (e.g. `1234.50` becoming `123450`) is a genuinely
///   different value and is still rejected.
/// - **Commodity** must match exactly, UNLESS the input commodity is EMPTY. A
///   blank commodity formats as a bare number that the parser legitimately
///   resolves from a `D` directive (or literal inference), so the resolved
///   commodity is accepted. A non-empty input commodity is still matched
///   strictly, so a corrupted symbol is caught.
fn amount_value_equivalent(input: &Amount, parsed: &Amount) -> bool {
    let commodity_ok = input.commodity.0.is_empty() || input.commodity == parsed.commodity;
    commodity_ok && input.quantity == parsed.quantity
}

fn costs_equivalent(input: &Amount, parsed: &Amount) -> bool {
    match (&input.cost, &parsed.cost) {
        (None, None) => true,
        (Some(a), Some(b)) => a.kind == b.kind && amount_value_equivalent(&a.amount, &b.amount),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Atomic write
// ---------------------------------------------------------------------------

/// Mode for a temp file at creation, and for a target that does not yet exist:
/// owner-only. A financial record defaults closed rather than umask-derived.
#[cfg(unix)]
const NEW_FILE_MODE: u32 = 0o600;

/// Distinct temp names tried before giving up. `create_new` refuses to open an
/// existing path, so a collision — accidental or hostile — costs a retry.
const TEMP_NAME_ATTEMPTS: usize = 8;

/// Symlink hops followed when resolving the write target, bounding a link loop.
const MAX_SYMLINK_HOPS: usize = 32;

/// Write `bytes` to `path` atomically: temp file in the same directory, `fsync`,
/// then `rename` over the target (best-effort directory `fsync`).
///
/// # What is carried forward
/// The rename installs a brand-new inode, so nothing about the old file survives
/// unless it is copied forward explicitly. This function copies:
///
/// - **Permission bits** (Unix). The target's mode is applied to the temp file
///   before the rename, so a journal kept at `0600` stays at `0600` instead of
///   being silently recreated at `0666 & ~umask` — world-readable under the
///   common `umask 022`. A target that does not exist yet is created `0600`,
///   deliberately ignoring a permissive umask.
/// - **Owner and group** (Unix, best effort). `fchown` is attempted and its
///   failure ignored, since an unprivileged process cannot give a file away.
///   This matters only when a privileged process edits another user's journal;
///   in the ordinary case the temp file is already owned by the right user.
/// - **The symlink at `path`, if any.** The write is redirected to the file the
///   link resolves to, so the link itself survives. A plain rename over `path`
///   would have replaced the link with a regular file.
///
/// # What is NOT carried forward
/// - **ACLs and extended attributes.** The new inode gets none. A journal
///   protected by a POSIX ACL, or carrying xattrs (macOS quarantine/Finder
///   metadata, SELinux labels), loses them on the first save. Preserving these
///   needs platform-specific APIs this crate does not link.
/// - **Hard links.** `rename` rebinds only this name; a second hard link to the
///   journal keeps pointing at the OLD inode and silently retains the pre-edit
///   content. This is inherent to rename-based atomic writes. The alternative —
///   truncating and rewriting in place — would preserve links, ACLs and xattrs
///   but opens a window in which a crash leaves a half-written journal, which is
///   the worse failure for an irreplaceable primary record.
/// - **Inode number, birth time, and open file descriptors** on the old inode,
///   which continue to observe the pre-edit content.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let target = resolve_symlinks(path);
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target.file_name().map_or_else(
        || "journal".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );

    let (tmp_path, file) = create_temp_file(dir, &file_name)?;
    if let Err(err) = fill_temp_file(file, bytes, &target) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    if let Err(err) = std::fs::rename(&tmp_path, &target) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    // Best-effort: durably record the rename in the directory.
    if let Ok(dir_file) = std::fs::File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

/// Write `bytes` into the open temp `file`, give it `target`'s ownership and
/// mode, and only then flush both to stable storage — so a crash between the
/// `fsync` and the `rename` cannot leave a file whose contents are durable but
/// whose permissions are not. Consumes `file` so the handle is closed before the
/// caller renames it (required on Windows, harmless elsewhere).
fn fill_temp_file(mut file: std::fs::File, bytes: &[u8], target: &Path) -> std::io::Result<()> {
    file.write_all(bytes)?;
    carry_forward_metadata(&file, target);
    file.sync_all()
}

/// Follow `path` through up to [`MAX_SYMLINK_HOPS`] symlinks and return the path
/// whose contents the rename should replace.
///
/// Renaming straight over a symlink replaces the *link* with a regular file,
/// silently detaching the journal from wherever the user pointed it. Renaming
/// over its resolved target leaves the link intact. A dangling link resolves to
/// its (non-existent) target, so the write creates the file the link already
/// names.
///
/// The hop limit exists purely to guarantee termination on a link *loop*. A loop
/// names no real file, so resolution stops on one of the links and the rename
/// replaces it with a regular file — the same outcome as before this function
/// existed, and the only one available.
fn resolve_symlinks(path: &Path) -> PathBuf {
    std::iter::successors(Some(path.to_path_buf()), |current| {
        let link = std::fs::symlink_metadata(current)
            .ok()
            .filter(|meta| meta.file_type().is_symlink())
            .and_then(|_| std::fs::read_link(current).ok())?;
        Some(match current.parent() {
            Some(dir) if link.is_relative() => dir.join(link),
            _ => link,
        })
    })
    .take(MAX_SYMLINK_HOPS)
    .last()
    .unwrap_or_else(|| path.to_path_buf())
}

/// Create a fresh temp file next to the target, returning its path and an open
/// handle, retrying on a name collision.
///
/// Uses `create_new` (`O_CREAT | O_EXCL`), which fails rather than opening an
/// existing path and — the point — refuses to follow a symlink planted at that
/// name. A local attacker who guesses the temp name therefore cannot redirect
/// the journal's contents into a file of their choosing; they can only force a
/// retry.
fn create_temp_file(dir: &Path, file_name: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    (0..TEMP_NAME_ATTEMPTS)
        .find_map(|_| {
            let tmp_path = dir.join(format!(".{file_name}.ledgeline-{}.tmp", unique_suffix()));
            match open_new_exclusive(&tmp_path) {
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => None,
                result => Some(result.map(|file| (tmp_path, file))),
            }
        })
        .unwrap_or_else(|| {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "could not create a temp file in {} after {TEMP_NAME_ATTEMPTS} attempts",
                    dir.display()
                ),
            ))
        })
}

/// Open `path` for writing, failing if anything — file, directory or symlink —
/// already exists there.
fn open_new_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Never wider than owner-only while the journal's contents are in
        // flight; the target's real mode is applied just before the rename.
        options.mode(NEW_FILE_MODE);
    }
    options.open(path)
}

/// Give the temp file `target`'s ownership and permission bits so the rename
/// does not relax them.
///
/// Best effort by design: on failure the temp file keeps [`NEW_FILE_MODE`],
/// which is *narrower* than any target it might replace, and failing the save
/// outright would cost the user an edit to their books to avoid a strictly safe
/// outcome. Ownership is set before the mode because `chown` clears the
/// set-user-ID and set-group-ID bits.
#[cfg(unix)]
fn carry_forward_metadata(file: &std::fs::File, target: &Path) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    // No target yet: keep the owner-only default rather than a umask-derived one.
    let Ok(existing) = std::fs::metadata(target) else {
        return;
    };
    let _ = std::os::unix::fs::fchown(file, Some(existing.uid()), Some(existing.gid()));
    let _ = file.set_permissions(std::fs::Permissions::from_mode(existing.mode() & 0o7777));
}

/// Non-Unix platforms expose no portable mode/ownership model here; the temp
/// file keeps whatever the OS gave it, as before.
#[cfg(not(unix))]
fn carry_forward_metadata(_file: &std::fs::File, _target: &Path) {}

/// A per-process-unique suffix for the temp file name.
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}-{seq}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountName, Cost, SourcePos};

    fn dollar_style() -> AmountStyle {
        AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: 2,
        }
    }

    fn eur_style() -> AmountStyle {
        AmountStyle {
            side: CommoditySide::Right,
            spaced: true,
            decimal_mark: Some(','),
            digit_groups: None,
            precision: 2,
        }
    }

    fn dollars(mantissa: i128, places: u32) -> Amount {
        Amount {
            commodity: Commodity("$".into()),
            quantity: Dec::new(mantissa, places),
            style: dollar_style(),
            cost: None,
        }
    }

    fn posting(account: &str, amounts: Vec<Amount>) -> Posting {
        Posting {
            status: Status::Unmarked,
            ptype: PostingType::Regular,
            account: AccountName(account.into()),
            amounts,
            balance_assertion: None,
            date: None,
            date2: None,
            comment: String::new(),
            tags: vec![],
        }
    }

    fn txn(date: &str, description: &str, postings: Vec<Posting>) -> Transaction {
        Transaction {
            index: Tindex(1),
            date: date.into(),
            date2: None,
            status: Status::Cleared,
            code: String::new(),
            description: description.into(),
            comment: String::new(),
            preceding_comment: String::new(),
            tags: vec![],
            postings,
            source_span: (
                SourcePos { line: 1, column: 1 },
                SourcePos { line: 3, column: 1 },
            ),
            source_file: PathBuf::new(),
        }
    }

    #[test]
    fn render_dec_shapes() {
        assert_eq!(render_dec(Dec::new(180_000, 2), '.'), "1800.00");
        assert_eq!(render_dec(Dec::new(5, 3), '.'), "0.005");
        assert_eq!(render_dec(Dec::new(-165_891, 2), '.'), "-1658.91");
        assert_eq!(render_dec(Dec::new(1000, 0), '.'), "1000");
        assert_eq!(render_dec(Dec::new(64500, 2), ','), "645,00");
        assert_eq!(render_dec(Dec::new(0, 0), '.'), "0");
    }

    /// SEC-2 item 4: `render_dec` must be total. Rust's format width is a `u16`,
    /// so the old `format!("{digits:0>width$}")` panicked with "Formatting
    /// argument out of range" once `places >= 65535` — reachable straight from
    /// the edit wire, where it returned no HTTP response at all.
    #[test]
    fn render_dec_is_total_for_extreme_places() {
        let expected = |places: u32| {
            let places = places.min(MAX_RENDER_PLACES) as usize;
            if places == 0 {
                "5".to_string()
            } else {
                format!("0.{}5", "0".repeat(places - 1))
            }
        };

        for places in [
            0,
            1,
            10,
            MAX_RENDER_PLACES - 1,
            MAX_RENDER_PLACES,
            MAX_RENDER_PLACES + 1,
            65_534, // the last value the old code survived
            65_535, // the first value the old code panicked on
            u32::MAX,
        ] {
            let rendered = render_dec(Dec::new(5, places), '.');
            assert_eq!(rendered, expected(places), "at places = {places}");
            assert!(
                rendered.len() <= MAX_RENDER_PLACES as usize + 2,
                "output must stay bounded at places = {places}, got {} bytes",
                rendered.len()
            );
        }
    }

    /// The widest mantissa at the widest scale: no panic, no unbounded string,
    /// and `i128::MIN` still negates safely via `unsigned_abs`.
    #[test]
    fn render_dec_handles_the_extreme_mantissa() {
        let rendered = render_dec(Dec::new(i128::MIN, u32::MAX), '.');
        assert!(rendered.starts_with("-0."));
        // sign + leading zero + mark + MAX_RENDER_PLACES fractional digits.
        assert_eq!(rendered.len(), MAX_RENDER_PLACES as usize + 3);
        assert!(rendered.ends_with(&i128::MIN.unsigned_abs().to_string()));
    }

    /// SEC-8: the temp file is opened with `create_new`, so a symlink planted at
    /// the temp path cannot capture the journal's contents. `O_CREAT | O_EXCL`
    /// refuses to follow a final symlink, which is what makes this safe.
    #[cfg(unix)]
    #[test]
    fn temp_file_creation_refuses_a_planted_symlink() {
        let dir = std::env::temp_dir().join(format!("ledgeline-sec8-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");

        let decoy = dir.join("decoy.txt");
        std::fs::write(&decoy, "untouched").expect("write decoy");
        let planted = dir.join("planted.tmp");
        std::os::unix::fs::symlink(&decoy, &planted).expect("plant symlink");

        let err = open_new_exclusive(&planted).expect_err("must refuse a planted symlink");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&decoy).expect("read decoy"),
            "untouched",
            "the symlink target must not have been opened or truncated"
        );

        // A fresh name succeeds, and is never wider than owner-only from birth.
        // Asserting a SUBSET of NEW_FILE_MODE rather than equality keeps this
        // independent of the runner's umask, which can only narrow it further.
        let fresh = dir.join("fresh.tmp");
        let file = open_new_exclusive(&fresh).expect("a fresh name opens");
        use std::os::unix::fs::PermissionsExt;
        let mode = file.metadata().expect("stat").permissions().mode() & 0o777;
        assert_eq!(
            mode & !NEW_FILE_MODE,
            0,
            "temp file mode {mode:o} is wider than {NEW_FILE_MODE:o}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_simple_cleared_txn_with_elided_leg() {
        let formatted = format_transaction(&txn(
            "2026-07-01",
            "Landlord | rent",
            vec![
                posting("expenses:housing:rent", vec![dollars(180_000, 2)]),
                posting("assets:bank:checking", vec![]),
            ],
        ));
        assert_eq!(
            formatted,
            "2026-07-01 * Landlord | rent\n    \
             expenses:housing:rent  $1800.00\n    assets:bank:checking\n"
        );
    }

    #[test]
    fn format_reparses_to_equivalent_transaction() {
        // A cost + comma-decimal EUR + code + comment all round-trip.
        let mut t = txn(
            "2025-09-12",
            "Hotel Adlon | lodging",
            vec![
                posting(
                    "expenses:travel:lodging",
                    vec![Amount {
                        commodity: Commodity("EUR".into()),
                        quantity: Dec::new(64500, 2),
                        style: eur_style(),
                        cost: None,
                    }],
                ),
                posting("assets:bank:wise:eur", vec![]),
            ],
        );
        t.code = "INV-9".into();
        t.comment = "trip: berlin\n".into();

        let text = format_transaction(&t);
        // Declare EUR's comma-decimal style so the reparse reads it correctly.
        let journal_text = format!("commodity 1.000,00 EUR\n\n{text}");
        let journal = parse_journal(&journal_text, "t.journal").unwrap();
        let parsed = &journal.transactions[0];
        assert!(transactions_equivalent(&t, parsed), "got: {text}");
        assert_eq!(parsed.code, "INV-9");
        assert_eq!(parsed.postings[0].amounts[0].quantity, Dec::new(64500, 2));
    }

    #[test]
    fn format_unit_cost_and_status_and_assertion() {
        let mut buy = txn(
            "2024-09-16",
            "Fidelity | buy AAPL",
            vec![
                posting(
                    "assets:broker:taxable:aapl",
                    vec![Amount {
                        commodity: Commodity("AAPL".into()),
                        quantity: Dec::new(10, 0),
                        style: AmountStyle {
                            side: CommoditySide::Right,
                            spaced: true,
                            decimal_mark: Some('.'),
                            digit_groups: None,
                            precision: 0,
                        },
                        cost: Some(Box::new(Cost {
                            kind: CostKind::Unit,
                            amount: dollars(22000, 2),
                        })),
                    }],
                ),
                posting("assets:broker:taxable:cash", vec![]),
            ],
        );
        // posting-level status + balance assertion on the first posting.
        buy.postings[0].status = Status::Cleared;
        buy.postings[0].balance_assertion = Some(BalanceAssertion {
            amount: dollars(500_000, 2),
            inclusive: false,
            total: false,
            position: SourcePos { line: 2, column: 1 },
        });

        let text = format_transaction(&buy);
        assert!(text.contains("* assets:broker:taxable:aapl  10 AAPL @ $220.00  = $5000.00"));
        // Re-parses cleanly and balances.
        let journal = parse_journal(&text, "t.journal").unwrap();
        assert!(is_balanced(&journal.transactions[0].postings).unwrap());
    }

    #[test]
    fn is_balanced_detects_imbalance() {
        // Two explicit legs that do not sum to zero.
        let unbalanced = vec![
            posting("a", vec![dollars(100, 0)]),
            posting("b", vec![dollars(-99, 0)]),
        ];
        assert!(!is_balanced(&unbalanced).unwrap());

        // Same legs, now balanced.
        let balanced = vec![
            posting("a", vec![dollars(100, 0)]),
            posting("b", vec![dollars(-100, 0)]),
        ];
        assert!(is_balanced(&balanced).unwrap());

        // Two elided legs cannot be inferred.
        let two_elided = vec![posting("a", vec![]), posting("b", vec![])];
        assert!(!is_balanced(&two_elided).unwrap());
    }

    #[test]
    fn unit_cost_balances_in_cost_commodity() {
        let postings = vec![
            posting(
                "assets:broker:aapl",
                vec![Amount {
                    commodity: Commodity("AAPL".into()),
                    quantity: Dec::new(10, 0),
                    style: dollar_style(),
                    cost: Some(Box::new(Cost {
                        kind: CostKind::Unit,
                        amount: dollars(22000, 2),
                    })),
                }],
            ),
            posting("assets:broker:cash", vec![dollars(-220_000, 2)]),
        ];
        assert!(is_balanced(&postings).unwrap());
    }

    #[test]
    fn add_with_empty_commodity_resolves_via_d_directive() {
        // A `D` default-commodity directive is in effect; the new transaction's
        // explicit leg carries an EMPTY commodity, which `format_transaction`
        // renders as a bare number and the parser resolves to `$`. The round-trip
        // guard must accept the resolved commodity and the value (scale-tolerant),
        // rather than tripping `RoundTripMismatch`.
        let text =
            "D $1,000.00\n\n2026-07-01 * X\n    expenses:a  $50.00\n    assets:bank:checking\n";
        let mut editor = JournalEditor::from_text("/tmp/d.journal", text).unwrap();

        let new_txn = txn(
            "2026-07-03",
            "t",
            vec![
                posting(
                    "assets:bank:checking",
                    vec![Amount {
                        commodity: Commodity(String::new()),
                        quantity: Dec::new(-50, 0),
                        style: dollar_style(),
                        cost: None,
                    }],
                ),
                posting("expenses:food", vec![]),
            ],
        );

        editor
            .add_transaction(&new_txn, InsertPosition::Append)
            .expect("empty-commodity add should resolve via the D directive");

        // The value lands on disk as a bare number under the `D` directive...
        assert!(
            editor.text().contains("assets:bank:checking  -50"),
            "got:\n{}",
            editor.text()
        );
        // ...and reads back with the RESOLVED commodity (`$`) and intended value.
        let added = editor
            .journal()
            .transactions
            .iter()
            .find(|t| t.description == "t")
            .expect("added transaction present after reparse");
        let checking = added
            .postings
            .iter()
            .find(|p| p.account.0 == "assets:bank:checking")
            .expect("checking posting present");
        assert_eq!(checking.amounts[0].commodity, Commodity("$".into()));
        assert_eq!(checking.amounts[0].quantity, Dec::new(-50, 0));
    }

    #[test]
    fn round_trip_guard_tolerates_scale_but_catches_corruption() {
        // Same value at a different scale (a `D`-directive precision): -50 vs
        // -50.00, same commodity — accepted (value equality ignores scale).
        let intended = txn(
            "2026-07-03",
            "t",
            vec![
                posting("a", vec![dollars(-50, 0)]),
                posting("b", vec![dollars(50, 0)]),
            ],
        );
        let rescaled = txn(
            "2026-07-03",
            "t",
            vec![
                posting("a", vec![dollars(-5000, 2)]),
                posting("b", vec![dollars(5000, 2)]),
            ],
        );
        assert!(transactions_equivalent(&intended, &rescaled));

        // A decimal-mark corruption turning 1234.50 into 123450 is a DIFFERENT
        // value and is still rejected.
        let good = txn(
            "2026-07-03",
            "t",
            vec![posting("a", vec![dollars(123_450, 2)])],
        );
        let corrupted = txn(
            "2026-07-03",
            "t",
            vec![posting("a", vec![dollars(123_450, 0)])],
        );
        assert!(!transactions_equivalent(&good, &corrupted));

        // A non-empty commodity that reparses to a DIFFERENT symbol is still
        // rejected — strict commodity match is preserved for explicit commodities.
        let usd = txn("2026-07-03", "t", vec![posting("a", vec![dollars(50, 0)])]);
        let eur = txn(
            "2026-07-03",
            "t",
            vec![posting(
                "a",
                vec![Amount {
                    commodity: Commodity("EUR".into()),
                    quantity: Dec::new(50, 0),
                    style: eur_style(),
                    cost: None,
                }],
            )],
        );
        assert!(!transactions_equivalent(&usd, &eur));
    }

    #[test]
    fn fingerprint_content_authoritative() {
        let a = Fingerprint::of_bytes(b"hello world\n");
        let b = Fingerprint::of_bytes(b"hello world\n");
        let c = Fingerprint::of_bytes(b"hello wor1d\n");
        assert!(a.content_matches(&b)); // identical content
        assert!(!a.content_matches(&c)); // content differs
    }

    /// DL-1: the line index must agree with `str::lines()`, not with ropey's
    /// Unicode line breaks, for every character the two disagree on.
    #[test]
    fn line_index_is_lf_only() {
        for break_char in [
            '\u{000B}', '\u{000C}', '\r', '\u{0085}', '\u{2028}', '\u{2029}',
        ] {
            let text = format!("a{break_char}b\nc\n");
            let rope = Rope::from_str(&text);
            let lines = Lines::new(&rope);
            assert_eq!(
                lines.len(),
                text.lines().count() + 1,
                "{break_char:?}: one line per `\\n`, plus the phantom final line"
            );
            assert_eq!(lines.line_to_char(0).unwrap(), 0);
            // Line 1 starts after the ONLY real break, the `\n` at index 3.
            assert_eq!(lines.line_to_char(1).unwrap(), 4, "{break_char:?}");
            assert_eq!(lines.char_to_line(2).unwrap(), 0, "{break_char:?}");
            assert_eq!(lines.char_to_line(4).unwrap(), 1, "{break_char:?}");
        }
    }

    /// `line_to_char` accepts one past the end (a transaction ending at EOF),
    /// and the empty buffer has exactly one line.
    #[test]
    fn line_index_edges() {
        let empty = Rope::from_str("");
        assert_eq!(Lines::new(&empty).len(), 1);
        assert_eq!(Lines::new(&empty).line_to_char(0).unwrap(), 0);

        let rope = Rope::from_str("a\nb\n");
        let lines = Lines::new(&rope);
        assert_eq!(lines.len(), 3, "a trailing newline yields a phantom line");
        assert_eq!(lines.line_to_char(3).unwrap(), 4, "one past the end is EOF");
        assert!(lines.line_to_char(4).is_err());

        let unterminated = Rope::from_str("a\nb");
        let lines = Lines::new(&unterminated);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.line_to_char(2).unwrap(), 3);
    }

    fn sources(texts: &[&str]) -> Vec<TxnSource> {
        texts
            .iter()
            .map(|t| (PathBuf::from("/j"), (*t).to_string()))
            .collect()
    }

    /// DL-4: the shapes every operation legitimately produces are accepted, and
    /// the shape DL-1's corruption produced is not.
    #[test]
    fn single_change_guard_accepts_only_one_edited_transaction() {
        let before = sources(&["A\n", "B\n", "C\n"]);

        // Delete the middle: one before, none after.
        assert!(check_single_change(&before, &sources(&["A\n", "C\n"])).is_ok());
        // Insert in the middle: none before, one after.
        assert!(check_single_change(&before, &sources(&["A\n", "X\n", "B\n", "C\n"])).is_ok());
        // Rewrite one in place.
        assert!(check_single_change(&before, &sources(&["A\n", "B2\n", "C\n"])).is_ok());
        // A no-op is trivially fine.
        assert!(check_single_change(&before, &before).is_ok());
        // A last transaction that gains the newline it lacked at EOF.
        assert!(check_single_change(&sources(&["A\n", "C"]), &sources(&["A\n", "C\n"])).is_ok());

        // The DL-1 shape: deleting B also destroyed a posting of A. The count
        // check saw 3 -> 2 and was satisfied; this must not be.
        let corrupted = sources(&["A-mutilated\n", "C\n"]);
        assert!(
            check_single_change(&before, &corrupted).is_err(),
            "a delete that also rewrote a neighbour must be rejected"
        );
        // Two transactions rewritten at once.
        assert!(check_single_change(&before, &sources(&["A2\n", "B2\n", "C\n"])).is_err());
    }

    /// DL-2's guard half: a dropped or altered balance assertion must fail the
    /// round-trip comparison.
    #[test]
    fn round_trip_guard_compares_balance_assertions() {
        let with_assertion = |assertion: Option<BalanceAssertion>| {
            let mut t = txn(
                "2026-07-03",
                "t",
                vec![
                    posting("assets:cash", vec![dollars(-50, 0)]),
                    posting("expenses:x", vec![dollars(50, 0)]),
                ],
            );
            t.postings[0].balance_assertion = assertion;
            t
        };
        let assertion = |mantissa: i128, total: bool, inclusive: bool| {
            Some(BalanceAssertion {
                amount: dollars(mantissa, 2),
                inclusive,
                total,
                position: SourcePos { line: 2, column: 1 },
            })
        };

        let intended = with_assertion(assertion(9900, false, false));
        // Dropped entirely — the DL-2 loss.
        assert!(!transactions_equivalent(&intended, &with_assertion(None)));
        // A different asserted amount.
        assert!(!transactions_equivalent(
            &intended,
            &with_assertion(assertion(9800, false, false))
        ));
        // `=` silently promoted to `==`.
        assert!(!transactions_equivalent(
            &intended,
            &with_assertion(assertion(9900, true, false))
        ));
        // `=` silently promoted to `=*`.
        assert!(!transactions_equivalent(
            &intended,
            &with_assertion(assertion(9900, false, true))
        ));
        // Preserved, at a different scale and a different source position: fine.
        let mut same = with_assertion(assertion(990_000, false, false));
        same.postings[0]
            .balance_assertion
            .as_mut()
            .expect("assertion")
            .amount
            .quantity = Dec::new(9900, 2);
        same.postings[0]
            .balance_assertion
            .as_mut()
            .expect("assertion")
            .position = SourcePos {
            line: 99,
            column: 7,
        };
        assert!(transactions_equivalent(&intended, &same));
    }

    /// DL-6: the header rebuild keeps the user's own date token.
    #[test]
    fn header_rebuild_preserves_the_original_date_token() {
        let mut t = txn("2026-01-01", "Payee", vec![]);
        t.code = "42".into();
        t.status = Status::Pending;

        let rope = Rope::from_str("2026/01/01 * (42) Payee\n");
        let lines = Lines::new(&rope);
        assert_eq!(
            rebuild_header(&t, &lines, 0).unwrap(),
            "2026/01/01 ! (42) Payee"
        );

        // An ISO original is left exactly as `format_header` renders it.
        let rope = Rope::from_str("2026-01-01 * (42) Payee\n");
        let lines = Lines::new(&rope);
        assert_eq!(
            rebuild_header(&t, &lines, 0).unwrap(),
            "2026-01-01 ! (42) Payee"
        );
    }

    /// DL-6: the terminator actually in use wins, and a tie favours `\n`.
    #[test]
    fn dominant_terminator_follows_the_file() {
        assert_eq!(dominant_terminator(&Rope::from_str("a\nb\n")), "\n");
        assert_eq!(dominant_terminator(&Rope::from_str("a\r\nb\r\n")), "\r\n");
        assert_eq!(dominant_terminator(&Rope::from_str("")), "\n");
        // Mixed, LF in the majority.
        assert_eq!(dominant_terminator(&Rope::from_str("a\r\nb\nc\n")), "\n");
    }
}
