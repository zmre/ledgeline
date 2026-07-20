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
//! header line through the last posting line, inclusive of their trailing
//! newlines. Because `source_span` lines are relative to the transaction's own
//! file, editing a transaction that lives in an `include`d file touches only
//! that file's rope and leaves the main journal (and every other include)
//! byte-identical.
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
//! - **Reparse-to-validate.** After any mutation the WHOLE journal is re-parsed
//!   with [`parse_journal_with_overrides`], resolving `include`s from the EDITED
//!   in-memory ropes (not the stale on-disk copies); the edit is only committed
//!   if it parses cleanly (and, for an add, the new transaction balances and
//!   round-trips). On failure `self` is left untouched.
//! - **External-change guard.** [`JournalEditor::save`] re-reads each file it is
//!   about to write and compares its content hash to that file's load-time
//!   fingerprint, refusing (with [`EditError::ExternalChange`]) — and writing
//!   nothing — rather than clobbering a file that changed underneath it.
//! - **Atomic write.** `save` writes each changed file to a temp file in the
//!   same directory, `fsync`s it, and `rename`s it over the target.
//! - **Single writer.** Mutations take `&mut self`; the server will wrap the
//!   editor in a `Mutex` in the next increment (no OS-level lock yet).

use crate::decimal::{Dec, DecError};
use crate::model::{
    Amount, AmountStyle, BalanceAssertion, Commodity, CommoditySide, CostKind, Journal, Posting,
    PostingType, Status, Tindex, Transaction,
};
use crate::parse::{ParseError, parse_journal, parse_journal_with_overrides, resolve_source_file};
use ropey::Rope;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
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

/// A load-time fingerprint used to detect external changes before saving.
///
/// The content `hash` (plus `len`) is authoritative: a mtime-only touch that
/// leaves the bytes identical is deliberately *not* treated as an external
/// change. `mtime` is captured for completeness and refreshed after a write.
#[derive(Debug, Clone)]
struct Fingerprint {
    mtime: Option<SystemTime>,
    hash: u64,
    len: u64,
}

impl Fingerprint {
    fn of_bytes(bytes: &[u8], mtime: Option<SystemTime>) -> Self {
        Self {
            mtime,
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

/// The file's last-modified time, if the platform/file provides one.
fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
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
    fn new(path: PathBuf, text: &str, mtime: Option<SystemTime>) -> Self {
        Self {
            fingerprint: Fingerprint::of_bytes(text.as_bytes(), mtime),
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
    journal: Journal,
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
        let text = std::fs::read_to_string(&path)?;
        let source_name = path.to_string_lossy().into_owned();
        let journal = parse_journal(&text, &source_name)?;
        Self::load(source_name, journal, &text, true)
    }

    /// Build an editor over in-memory `text` associated with `path`, without
    /// touching the filesystem. This path is single-file: it does not resolve
    /// `include`s (the initial parse would read them from disk).
    ///
    /// The main file's fingerprint is taken from `text` with no mtime, so a
    /// later [`save`](Self::save) falls back to the authoritative content-hash
    /// guard (and requires `path` to exist on disk). Useful for in-memory
    /// editing (the server can hold the text already) and for tests.
    ///
    /// # Errors
    /// [`EditError::Parse`] if `text` does not parse.
    pub fn from_text(path: impl Into<PathBuf>, text: &str) -> Result<Self, EditError> {
        let path = path.into();
        let source_name = path.to_string_lossy().into_owned();
        let journal = parse_journal(text, &source_name)?;
        Self::load(source_name, journal, text, false)
    }

    /// Build an editor from a freshly-parsed `journal`. The main file's text is
    /// `main_text` (already in hand); every other file a transaction came from
    /// is read from disk. `on_disk` captures the main file's mtime for the
    /// fast-path external-change check (false for [`from_text`](Self::from_text)).
    fn load(
        source_name: String,
        journal: Journal,
        main_text: &str,
        on_disk: bool,
    ) -> Result<Self, EditError> {
        let main_key = resolve_source_file(&source_name);
        let main_mtime = if on_disk { file_mtime(&main_key) } else { None };
        let mut files: HashMap<PathBuf, FileBuf> = HashMap::new();
        files.insert(
            main_key.clone(),
            FileBuf::new(main_key.clone(), main_text, main_mtime),
        );
        for txn in &journal.transactions {
            if !files.contains_key(&txn.source_file) {
                let key = txn.source_file.clone();
                let file_text = std::fs::read_to_string(&key)?;
                let mtime = file_mtime(&key);
                files.insert(key.clone(), FileBuf::new(key, &file_text, mtime));
            }
        }
        Ok(Self {
            source_name,
            main_key,
            files,
            journal,
        })
    }

    /// The main journal file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.main_key
    }

    /// The parsed journal, as of the last committed edit.
    #[must_use]
    pub fn journal(&self) -> &Journal {
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
        let (start, end) = txn_char_range(rope, txn).ok()?;
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
    /// already-edited) rope. Requires exactly `expected` transactions.
    fn validate_with(
        &self,
        edited_key: &Path,
        candidate: &Rope,
        expected: usize,
    ) -> Result<Journal, EditError> {
        let mut overrides: HashMap<PathBuf, String> = self
            .files
            .iter()
            .map(|(key, file)| (key.clone(), file.rope.to_string()))
            .collect();
        overrides.insert(edited_key.to_path_buf(), candidate.to_string());
        let reparsed = parse_journal_with_overrides(&self.source_name, &overrides)
            .map_err(EditError::ParseInvalidAfterEdit)?;
        if reparsed.transactions.len() != expected {
            return Err(EditError::Internal(format!(
                "expected {expected} transactions after the edit, found {}",
                reparsed.transactions.len()
            )));
        }
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
        self.journal = reparsed;
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

        let len_lines = rope.len_lines();
        let start_line0 = start_span.min(len_lines);
        let mut end_line0 = end_span.min(len_lines);
        // (a) trailing indented comment lines are part of this transaction.
        while end_line0 < len_lines && line_is_indented_content(rope, end_line0) {
            end_line0 += 1;
        }
        // (b) consume one following blank separator line, if present.
        if end_line0 < len_lines && line_is_blank(rope, end_line0) {
            end_line0 += 1;
        }
        let mut start = line_start_char(rope, start_line0)?;
        let end = line_start_char(rope, end_line0.min(len_lines))?;
        // (c) if the deletion runs to end-of-file, drop one preceding blank so the
        // file does not end on a dangling separator blank.
        if end == rope.len_chars() && start_line0 > 0 && line_is_blank(rope, start_line0 - 1) {
            start = line_start_char(rope, start_line0 - 1)?;
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
        let new_header = format_header(&rebuilt);

        let rope = self.rope_for(&source_file)?;
        let (start, content_end) = line_content_range(rope, header_line0)?;
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
        if updated.description != new_description {
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
        let new_header = format_header(&rebuilt);

        let rope = self.rope_for(&source_file)?;
        let (start, content_end) = line_content_range(rope, header_line0)?;
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
        if updated.status != status {
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
        let line0 =
            nth_posting_line(rope, header_line0, scan_end0, posting_index).ok_or_else(|| {
                EditError::Internal(format!(
                    "could not locate posting #{posting_index} of transaction #{} in {}",
                    index.0,
                    source_file.display()
                ))
            })?;
        let (start, end) = locate_account_token(rope, line0, &current_account)?;

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
        let (source_file, start, end) = {
            let existing = self
                .find_transaction(index)
                .ok_or(EditError::TransactionNotFound(index.0))?;
            let rope = self.rope_for(&existing.source_file)?;
            let (start, end) = txn_char_range(rope, existing)?;
            (existing.source_file.clone(), start, end)
        };
        let body = format_transaction(txn);

        let rope = self.rope_for(&source_file)?;
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
        match self
            .journal
            .transactions
            .iter()
            .filter(|existing| existing.date.as_str() <= txn.date.as_str())
            .max_by(|a, b| a.date.cmp(&b.date))
        {
            Some(predecessor) => self.insert_after(predecessor, body),
            // No predecessor ⇒ the new row precedes every existing one: place it
            // before the earliest (first such in file order on a date tie).
            None => {
                let earliest = self
                    .journal
                    .transactions
                    .iter()
                    .min_by(|a, b| a.date.cmp(&b.date))
                    .ok_or_else(|| {
                        EditError::Internal("no earliest transaction to insert before".into())
                    })?;
                self.insert_before(earliest, body)
            }
        }
    }

    /// Append `body` to the MAIN file with exactly one blank separator line.
    fn append_to_main(&self, body: &str) -> Result<Placement, EditError> {
        let main = self.rope_for(&self.main_key)?;
        Ok(Placement {
            file_key: self.main_key.clone(),
            insertion: append_insertion(main, body),
        })
    }

    /// Insert `body` immediately after `predecessor`'s span, IN THE PREDECESSOR'S
    /// OWN file. When the predecessor is the last content in that file this uses the
    /// same end-of-file/trailing-newline handling as an append; otherwise it opens a
    /// fresh blank separator line and lets the existing blank below separate the new
    /// row from the next transaction.
    fn insert_after(&self, predecessor: &Transaction, body: &str) -> Result<Placement, EditError> {
        let rope = self.rope_for(&predecessor.source_file)?;
        let (_, end) = txn_char_range(rope, predecessor)?;
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
            insertion,
        })
    }

    /// Insert `body` immediately before `earliest`'s header, IN THE EARLIEST
    /// transaction's OWN file, with a trailing blank separator; whatever precedes
    /// the header (directives or a blank) stays above the new row.
    fn insert_before(&self, earliest: &Transaction, body: &str) -> Result<Placement, EditError> {
        let rope = self.rope_for(&earliest.source_file)?;
        let (start, _) = txn_char_range(rope, earliest)?;
        Ok(Placement {
            file_key: earliest.source_file.clone(),
            insertion: Insertion {
                offset: start,
                prefix: String::new(),
                body: format!("{body}\n"),
            },
        })
    }

    /// Save every file whose rope changed since load back to disk, atomically,
    /// refusing (and writing NOTHING) if any changed file was modified externally.
    ///
    /// First every dirty file is checked against its load-time fingerprint: an
    /// unchanged `mtime` is a fast path taken as unchanged content; otherwise the
    /// file is re-read and its content hash compared (authoritative). If ANY dirty
    /// file changed on disk this returns [`EditError::ExternalChange`] before
    /// writing anything. Otherwise each dirty file is written to a temp file in
    /// its own directory, `fsync`ed, and `rename`d over the target, and its
    /// fingerprint is refreshed and its dirty flag cleared. Unchanged files
    /// (including untouched includes) are never rewritten.
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

        // Pass 2: atomically write each dirty file and refresh its fingerprint.
        for key in &dirty {
            let (path, new_text) = {
                let file = self.files.get(key).ok_or_else(|| {
                    EditError::Internal(format!(
                        "no loaded buffer for source file {}",
                        key.display()
                    ))
                })?;
                (file.path.clone(), file.rope.to_string())
            };
            atomic_write(&path, new_text.as_bytes())?;
            let fingerprint = Fingerprint::of_bytes(new_text.as_bytes(), file_mtime(&path));
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
/// An unchanged `mtime` is a fast path (content taken as unchanged); otherwise
/// the file is re-read and its content hash compared, which is authoritative.
fn file_changed_externally(path: &Path, fingerprint: &Fingerprint) -> Result<bool, EditError> {
    let current_mtime = file_mtime(path);
    let unchanged = match (fingerprint.mtime, current_mtime) {
        (Some(loaded), Some(now)) if loaded == now => true,
        _ => {
            let current = std::fs::read(path)?;
            Fingerprint::of_bytes(&current, current_mtime).content_matches(fingerprint)
        }
    };
    Ok(!unchanged)
}

/// The half-open rope char range `[start, end)` covering a transaction's
/// `source_span` within `rope` — the header line through the line after its last
/// posting. `line_to_char` accepts a one-past-the-end line index, so a final
/// transaction that ends at EOF is handled without special-casing.
fn txn_char_range(rope: &Rope, txn: &Transaction) -> Result<(usize, usize), EditError> {
    let len_lines = rope.len_lines();
    let start_line0 = txn.source_span.0.line.saturating_sub(1) as usize;
    let end_line0 = txn.source_span.1.line.saturating_sub(1) as usize;
    let start = line_start_char(rope, start_line0.min(len_lines))?;
    let end = line_start_char(rope, end_line0.min(len_lines))?;
    Ok((start, end))
}

fn line_start_char(rope: &Rope, line0: usize) -> Result<usize, EditError> {
    rope.try_line_to_char(line0)
        .map_err(|e| EditError::Internal(format!("line_to_char({line0}): {e}")))
}

/// Whether `rope` line `line0` exists and is a real blank line (has content — a
/// newline and/or whitespace — but trims to empty). The phantom empty line after
/// a trailing newline (zero chars) is not counted.
fn line_is_blank(rope: &Rope, line0: usize) -> bool {
    match rope.get_line(line0) {
        Some(line) => line.len_chars() > 0 && line.to_string().trim().is_empty(),
        None => false,
    }
}

/// Whether `rope` line `line0` exists, is indented (starts with a space/tab), and
/// is non-blank — i.e. a trailing in-transaction line (a posting-block comment)
/// that belongs to the preceding transaction.
fn line_is_indented_content(rope: &Rope, line0: usize) -> bool {
    match rope.get_line(line0) {
        Some(line) if line.len_chars() > 0 => {
            let text = line.to_string();
            text.starts_with([' ', '\t']) && !text.trim().is_empty()
        }
        _ => false,
    }
}

/// The char range `[start, content_end)` of `rope` line `line0`'s content,
/// excluding its trailing line terminator (`\r\n`/`\n`, or none at EOF). Used to
/// rewrite a line's text while preserving its exact terminator.
fn line_content_range(rope: &Rope, line0: usize) -> Result<(usize, usize), EditError> {
    let start = line_start_char(rope, line0)?;
    let line = rope
        .get_line(line0)
        .ok_or_else(|| EditError::Internal(format!("line {line0} is out of range")))?;
    let text = line.to_string();
    let content = text.trim_end_matches('\n').trim_end_matches('\r');
    Ok((start, start + content.chars().count()))
}

/// Whether `rope` line `line0` is a posting line: indented, non-blank, and not a
/// `;` comment line (mirrors the parser, which treats every indented non-`;` line
/// in a transaction body as a posting).
fn line_is_posting(rope: &Rope, line0: usize) -> bool {
    match rope.get_line(line0) {
        Some(line) if line.len_chars() > 0 => {
            let text = line.to_string();
            let trimmed = text.trim_start();
            text.starts_with([' ', '\t']) && !trimmed.is_empty() && !trimmed.starts_with(';')
        }
        _ => false,
    }
}

/// The 0-based line of the `posting_index`-th posting of the transaction whose
/// header is on line `header_line0`, scanning posting lines in the half-open line
/// range `(header_line0, scan_end0)` of `rope`. Blank and `;` comment lines are
/// skipped, so postings map to source lines by ordinal position.
fn nth_posting_line(
    rope: &Rope,
    header_line0: usize,
    scan_end0: usize,
    posting_index: usize,
) -> Option<usize> {
    let end = scan_end0.min(rope.len_lines());
    let mut seen = 0;
    for line0 in (header_line0 + 1)..end {
        if line_is_posting(rope, line0) {
            if seen == posting_index {
                return Some(line0);
            }
            seen += 1;
        }
    }
    None
}

/// The `rope` char range `[start, end)` of the account token on posting line
/// `line0`, matched as the first occurrence of `current_account` after the line's
/// indentation and any `*`/`!` status marker (skipping a leading `(`/`[` virtual
/// bracket). Only the account name is spanned, so replacing it leaves the marker,
/// amount, assertion, comment, and whitespace untouched.
fn locate_account_token(
    rope: &Rope,
    line0: usize,
    current_account: &str,
) -> Result<(usize, usize), EditError> {
    let line = rope
        .get_line(line0)
        .ok_or_else(|| EditError::Internal(format!("posting line {line0} is out of range")))?;
    let text = line.to_string();
    let content = text.trim_end_matches('\n').trim_end_matches('\r');

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
    let line_start = line_start_char(rope, line0)?;
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

/// The pieces of a rope insertion: insert `prefix` then `body` at `offset`.
struct Insertion {
    offset: usize,
    prefix: String,
    body: String,
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
    let line0 = candidate
        .try_char_to_line(header_char)
        .map_err(|e| EditError::Internal(format!("char_to_line({header_char}): {e}")))?;
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

/// Render a [`Dec`] using `mark` as the decimal separator, exactly (no rounding,
/// no grouping): `Dec::new(180_000, 2)` → `1800.00`, `Dec::new(5, 3)` → `0.005`.
fn render_dec(value: Dec, mark: char) -> String {
    let negative = value.mantissa < 0;
    let digits = value.mantissa.unsigned_abs().to_string();
    let body = if value.places == 0 {
        digits
    } else {
        let places = value.places as usize;
        // Ensure there is at least one integer digit before the mark.
        let padded = if digits.len() <= places {
            format!("{digits:0>width$}", width = places + 1)
        } else {
            digits
        };
        let split = padded.len() - places;
        format!("{}{mark}{}", &padded[..split], &padded[split..])
    };
    if negative { format!("-{body}") } else { body }
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
/// same header fields and, for each posting, the same account/type and (for
/// explicit postings) the same amount values and costs. An elided input posting
/// (empty `amounts`) skips the amount check — its value is inferred on re-parse.
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
            && (a.amounts.is_empty() || amounts_equivalent(&a.amounts, &b.amounts))
    })
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

/// Write `bytes` to `path` atomically: temp file in the same directory,
/// `fsync`, then `rename` over the target (best-effort directory `fsync`).
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().map_or_else(
        || "journal".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let tmp_path = dir.join(format!(".{file_name}.ledgeline-{}.tmp", unique_suffix()));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    // Best-effort: durably record the rename in the directory.
    if let Ok(dir_file) = std::fs::File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

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
        let a = Fingerprint::of_bytes(b"hello world\n", None);
        let b = Fingerprint::of_bytes(b"hello world\n", Some(SystemTime::now()));
        let c = Fingerprint::of_bytes(b"hello wor1d\n", None);
        assert!(a.content_matches(&b)); // mtime differs, content identical
        assert!(!a.content_matches(&c)); // content differs
    }
}
