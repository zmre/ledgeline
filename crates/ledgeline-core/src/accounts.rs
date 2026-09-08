//! `account NAME  ; tags...` declarations: the format-preserving editor over
//! them.
//!
//! `rules.rs` states the discipline this follows, and `aliases.rs` states it
//! for the construct closest in shape to this one:
//!
//! > An edit rewrites bytes only inside the spans it names, and every other
//! > byte of the file comes out the `&str` slice it went in as.
//!
//! # Why there is no `AliasLock`-shaped lock here
//!
//! `aliases.rs` needs locks because an edit there only splices the pattern and
//! replacement *inside* otherwise-preserved formatting, so a line whose
//! existing shape is ambiguous (a stray `;`, an unescaped `/`) cannot be
//! touched at all without guessing what its author meant.
//!
//! Here, an edit replaces the **entire** comment — everything from the
//! separator that ends the account name to the end of the line — with this
//! module's own deterministic rendering of `{tags, note}`: every tag as
//! `key: value`, comma-joined, then the free-text note as the trailing
//! segment. There is no existing shape to preserve, only a candidate new
//! string to validate, so every declared account is editable. The risk moves
//! from "can this line be read safely" to "would the rendered replacement
//! read back as something other than what was asked" — a write-time check
//! ([`check_tags`], [`check_note`]), not a per-line lock. [`AccountDoc::verify`]
//! still re-renders, byte-compares, and re-parses the result, exactly as
//! [`crate::aliases::AliasDoc::verify`] does.
//!
//! # Reading tags and note out of one comment
//!
//! An `account` directive's comment is not, at the model level, two separate
//! fields — [`crate::model::AccountDeclaration::tags`] is *derived* from
//! [`crate::model::AccountDeclaration::comment`] by
//! [`crate::parse::parse_tags`], which classifies each comma-separated
//! segment as a tag (`name:value`) or not. This module reuses that exact
//! predicate ([`crate::parse::tag_in_segment`]) to do the other half of the
//! same job: the segments that are *not* tags, rejoined with `", "`, are the
//! note. Using a second, hand-rolled definition of "is this a tag" here would
//! be the precise silent-divergence bug `parse_account_directive`'s own doc
//! comment already records one instance of.
//!
//! # Declaring a previously-undeclared account
//!
//! [`AccountEdit::Declare`] is [`crate::aliases::AliasEdit::Append`]'s
//! analogue: inserted immediately after the file's last `account` line, or at
//! EOF when it has none. Unlike an alias, an `account` directive's *order*
//! never changes what it means (no scoping, no first-match cascade), so this
//! insertion point is simpler than [`crate::aliases::AliasDoc`]'s — it is an
//! imitation of the aliases module's placement convention (new declarations
//! sit together, at the bottom of the block that already has them), not a
//! second instance of the reasoning that convention exists for.

use crate::model::Journal;
use crate::parse::{split_account_name, tag_in_segment};
use crate::rules::Newline;
use std::ops::Range;
use thiserror::Error;

/// A byte range into [`AccountDoc::text`]. Byte offsets, like `rules::Span`
/// and `aliases::Span`, for the same reason: every one lands on a boundary
/// the scan found in the text, so slicing can never split a code point.
pub type Span = Range<usize>;

/// Longest tag name this module will write, in bytes.
pub const MAX_TAG_NAME_BYTES: usize = 64;
/// Longest tag value this module will write, in bytes.
pub const MAX_TAG_VALUE_BYTES: usize = 256;
/// Longest note this module will write, in bytes.
pub const MAX_NOTE_BYTES: usize = 512;
/// Longest account name this module will write, in bytes.
pub const MAX_NAME_BYTES: usize = 300;
/// How many tags one edit may set. A bound on the request, not on the user:
/// nobody hand-types sixty tags onto one account in a single save.
pub const MAX_TAGS_PER_EDIT: usize = 64;

// ---------------------------------------------------------------------------
// The document model
// ---------------------------------------------------------------------------

/// One `account` line, located in a file's bytes, with its comment already
/// classified into tags and a free-text note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLine {
    /// 0-based position among **this file's account lines** — the handle an
    /// edit names. A parse-time index, not durable across saves; the caller's
    /// revision check is what makes that safe (same convention as
    /// [`crate::aliases::AliasLine::index`]).
    pub index: usize,
    /// 1-based file line, numbered LF-only exactly as [`str::lines`] does.
    pub line: u32,
    /// The declared account name.
    pub name: String,
    /// Tags parsed from the comment, in comma order.
    pub tags: Vec<(String, String)>,
    /// The comment's free-text segments (everything that did not classify as
    /// a tag), rejoined with `", "`. Empty when the comment is only tags, or
    /// there is no comment.
    pub note: String,
    /// The line's content, without its terminator — what [`AccountDoc::verify`]
    /// compares an unedited line's bytes against.
    span: Span,
    /// Where a [`AccountEdit::Replace`] splices: from the separator that ends
    /// the account name (or the end of the line, when there is no separator)
    /// to the end of the line's content. Replacing it with `""` clears the
    /// comment entirely, leaving a bare `account NAME`.
    tail: Span,
    /// The line's content **with** its terminator — what an
    /// [`AccountEdit::Delete`] removes, and whose end is where a new
    /// declaration is inserted when it lands after this line. Equal to `span`
    /// at EOF with no trailing newline.
    full: Span,
}

/// One change to one file's `account` declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountEdit {
    /// Rewrite one line's tags and note, leaving its name and everything
    /// before the comment exactly as it is.
    Replace {
        /// Which line, by [`AccountLine::index`].
        index: usize,
        /// The new tags, in the order they should be written.
        tags: Vec<(String, String)>,
        /// The new free-text note.
        note: String,
    },
    /// Declare a previously-undeclared account. See the module docs for why
    /// the insertion point needs no ordering rule beyond "after the others".
    Declare {
        /// The account name to declare.
        name: String,
        /// Its initial tags.
        tags: Vec<(String, String)>,
        /// Its initial note.
        note: String,
    },
    /// Remove one declaration, terminator and all. This only removes the
    /// chart-of-accounts entry — it does not touch, and cannot touch, any
    /// posting that still names the account; hledger infers what it needs
    /// from the name or from nothing at all.
    Delete {
        /// Which line, by [`AccountLine::index`].
        index: usize,
    },
}

/// A complete set of changes to one file's `account` declarations.
///
/// Like [`crate::aliases::AliasPlan`]: no `order`, and omission is not an
/// error — a client that sends one edit changes one line. At most one
/// [`AccountEdit::Replace`] or [`AccountEdit::Delete`] per existing line (never
/// both — naming the same line twice is refused, same as two `Replace`s
/// would be); any number of [`AccountEdit::Declare`]s, applied in the order
/// given (there is no specificity re-ordering here, because unlike an alias an
/// account declaration's order carries no meaning to reorder around).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountPlan {
    /// The changes, in any order for `Replace`; `Declare`s land in the order
    /// listed.
    pub edits: Vec<AccountEdit>,
}

impl AccountPlan {
    /// The [`AccountEdit::Replace`] naming `index`, if any.
    fn edit_for(&self, index: usize) -> Option<&AccountEdit> {
        self.edits
            .iter()
            .find(|edit| matches!(edit, AccountEdit::Replace { index: at, .. } if *at == index))
    }
}

/// Errors from planning or checking an account-declaration rewrite.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AccountError {
    /// An edit named a line this file does not have — almost always a stale
    /// index from a client that planned against an older parse.
    #[error("this file has no account number {0}")]
    UnknownAccount(usize),
    /// Two edits named the same line.
    #[error("account number {0} was named by more than one change")]
    DuplicateAccount(usize),
    /// A client-supplied value is not something this module will write into a
    /// journal.
    #[error("{0}")]
    Invalid(String),
    /// The rewritten text could not be proved to be the requested edit and
    /// nothing else, so the caller must write nothing.
    #[error("the rewritten journal failed its round-trip check; nothing was written")]
    RoundTripMismatch,
}

/// One journal file's `account` declarations, over its original text.
///
/// Immutable. An edit is an [`AccountPlan`] rendered by [`AccountDoc::apply`],
/// which returns a new `String` and never mutates `self`, so a refused edit
/// cannot leave half a document behind.
#[derive(Debug, Clone)]
pub struct AccountDoc {
    text: String,
    newline: Newline,
    lines: Vec<AccountLine>,
}

impl AccountDoc {
    /// Parse one journal file's `account` lines. **Infallible** — a file
    /// always opens, and every declared account is editable (see the module
    /// docs for why there is no locked case).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self {
            text: text.to_string(),
            newline: Newline::detect(text),
            lines: scan(text),
        }
    }

    /// The original text, byte for byte.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The account lines, in file order. Index equals [`AccountLine::index`].
    #[must_use]
    pub fn lines(&self) -> &[AccountLine] {
        &self.lines
    }

    /// Render the file under `plan`. Pure; no I/O; `self` is untouched.
    ///
    /// # Errors
    /// Validation runs to completion before a byte is rendered, so a rejected
    /// plan never produces partial text: [`AccountError::UnknownAccount`],
    /// [`AccountError::DuplicateAccount`] or [`AccountError::Invalid`].
    pub fn apply(&self, plan: &AccountPlan) -> Result<String, AccountError> {
        let splices = self.splices(plan)?;
        // Every byte outside a splice is copied from `self.text` verbatim —
        // the isolation guarantee, held by construction rather than by
        // inspection, exactly as `AliasDoc::apply` holds it.
        let mut out = String::with_capacity(self.text.len());
        let mut cursor = 0;
        for (span, replacement) in &splices {
            out.push_str(&self.text[cursor..span.start]);
            out.push_str(replacement);
            cursor = span.end;
        }
        out.push_str(&self.text[cursor..]);
        Ok(out)
    }

    /// Prove `new_text` is exactly the edit `plan` asked for, and nothing else.
    ///
    /// 1. re-render the plan and require `new_text` to match byte for byte;
    /// 2. require the re-parse to hold exactly the surviving original lines
    ///    (every one not named by an [`AccountEdit::Delete`]) plus every
    ///    declared one, in order;
    /// 3. require every **unedited** surviving line to come back
    ///    byte-identical to its original span content;
    /// 4. require every **replaced** line to read back with exactly the
    ///    tags/note that were asked for, and every **declared** one to read
    ///    back with exactly the name/tags/note that were asked for — the
    ///    render→parse fixpoint, which is what catches a note that would be
    ///    written and then read back as an extra tag.
    ///
    /// What is deliberately *not* here: whether the file still parses as part
    /// of its journal. Only the caller knows which journal this file belongs
    /// to, so that check is the caller's ([`crate::parse::parse_journal_with_overrides`]),
    /// exactly as [`crate::aliases::AliasDoc::verify`] leaves it to the server.
    ///
    /// # Errors
    /// [`AccountError::RoundTripMismatch`] if any step fails, plus whatever
    /// [`AccountDoc::apply`] reports for an invalid plan.
    pub fn verify(&self, plan: &AccountPlan, new_text: &str) -> Result<(), AccountError> {
        if self.apply(plan)? != new_text {
            return Err(AccountError::RoundTripMismatch);
        }
        let reparsed = Self::parse(new_text);

        let deleted: Vec<usize> = plan
            .edits
            .iter()
            .filter_map(|edit| match edit {
                AccountEdit::Delete { index } => Some(*index),
                _ => None,
            })
            .collect();
        let survivors: Vec<&AccountLine> = self
            .lines
            .iter()
            .filter(|line| !deleted.contains(&line.index))
            .collect();
        let declared = plan
            .edits
            .iter()
            .filter(|edit| matches!(edit, AccountEdit::Declare { .. }))
            .count();
        if reparsed.lines.len() != survivors.len() + declared {
            return Err(AccountError::RoundTripMismatch);
        }

        // Every SURVIVING original line, in order, lines up with the first
        // `survivors.len()` reparsed lines — deletes remove from this list,
        // never shift what remains.
        for (before, after) in survivors.iter().zip(reparsed.lines.iter()) {
            match plan.edit_for(before.index) {
                Some(AccountEdit::Replace { tags, note, .. }) => {
                    if &after.tags != tags || &after.note != note {
                        return Err(AccountError::RoundTripMismatch);
                    }
                }
                _ => {
                    if new_text[after.span.clone()] != self.text[before.span.clone()] {
                        return Err(AccountError::RoundTripMismatch);
                    }
                }
            }
        }

        // Then the declared ones, in `plan.edits`'s own order — the order
        // `splices` actually wrote them in, since (unlike aliases) this
        // module never reorders them.
        let mut tail = reparsed.lines[survivors.len()..].iter();
        for edit in &plan.edits {
            if let AccountEdit::Declare { name, tags, note } = edit {
                let Some(after) = tail.next() else {
                    return Err(AccountError::RoundTripMismatch);
                };
                if &after.name != name || &after.tags != tags || &after.note != note {
                    return Err(AccountError::RoundTripMismatch);
                }
            }
        }
        Ok(())
    }

    /// The ordered, non-overlapping `(span, text)` list one plan splices.
    fn splices(&self, plan: &AccountPlan) -> Result<Vec<(Span, String)>, AccountError> {
        self.check_plan(plan)?;
        let mut splices: Vec<(Span, String)> = Vec::new();
        for edit in &plan.edits {
            match edit {
                AccountEdit::Replace { index, tags, note } => {
                    let line = self.line(*index)?;
                    let suffix = render_suffix(tags, note);
                    let replacement = if suffix.is_empty() {
                        String::new()
                    } else {
                        format!("  ; {suffix}")
                    };
                    splices.push((line.tail.clone(), replacement));
                }
                AccountEdit::Declare { name, tags, note } => {
                    let at = self.insertion_point();
                    splices.push((at..at, self.appended(at, name, tags, note)));
                }
                AccountEdit::Delete { index } => {
                    let line = self.line(*index)?;
                    splices.push((line.full.clone(), String::new()));
                }
            }
        }
        // Stable, so multiple `Declare`s sharing the same insertion point
        // land in `plan.edits`'s own order rather than being reshuffled.
        splices.sort_by_key(|(span, _)| (span.start, span.end));
        Ok(splices)
    }

    /// Where an [`AccountEdit::Declare`] puts its line: immediately after the
    /// file's last `account` line, or at EOF when it has none.
    fn insertion_point(&self) -> usize {
        self.lines
            .last()
            .map_or(self.text.len(), |last| last.full.end)
    }

    /// The text an [`AccountEdit::Declare`] inserts at `at`.
    ///
    /// A file whose last line has no terminator gets one first, and no blank
    /// line is added — the same two rules `aliases::AliasDoc::appended`
    /// states at length.
    fn appended(&self, at: usize, name: &str, tags: &[(String, String)], note: &str) -> String {
        let newline = self.newline.as_str();
        let lead = if at == 0 || self.text[..at].ends_with('\n') {
            ""
        } else {
            newline
        };
        let suffix = render_suffix(tags, note);
        let sep = if suffix.is_empty() {
            String::new()
        } else {
            format!("  ; {suffix}")
        };
        format!("{lead}account {name}{sep}{newline}")
    }

    /// Reject a plan before a byte of it is rendered.
    fn check_plan(&self, plan: &AccountPlan) -> Result<(), AccountError> {
        let mut named: Vec<usize> = Vec::new();
        for edit in &plan.edits {
            match edit {
                AccountEdit::Replace { index, tags, note } => {
                    if named.contains(index) {
                        return Err(AccountError::DuplicateAccount(*index));
                    }
                    named.push(*index);
                    self.line(*index)?;
                    check_tags(tags)?;
                    check_note(note)?;
                }
                AccountEdit::Declare { name, tags, note } => {
                    check_name(name)?;
                    check_tags(tags)?;
                    check_note(note)?;
                }
                AccountEdit::Delete { index } => {
                    if named.contains(index) {
                        return Err(AccountError::DuplicateAccount(*index));
                    }
                    named.push(*index);
                    self.line(*index)?;
                }
            }
        }
        Ok(())
    }

    /// The line with this index.
    fn line(&self, index: usize) -> Result<&AccountLine, AccountError> {
        self.lines
            .get(index)
            .ok_or(AccountError::UnknownAccount(index))
    }
}

/// Render `{tags, note}` into a comment's content (without the leading `; `):
/// every tag as `key: value`, comma-joined, then the note as the trailing
/// segment. Empty when both are empty.
fn render_suffix(tags: &[(String, String)], note: &str) -> String {
    let mut parts: Vec<String> = tags
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect();
    if !note.is_empty() {
        parts.push(note.to_string());
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// Value validation
// ---------------------------------------------------------------------------

fn has_control(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_control())
}

/// Whether `text` contains two-or-more consecutive spaces/tabs — the run
/// [`split_account_name`] reads as ending an account name early.
fn contains_whitespace_run(text: &str) -> bool {
    let mut run = false;
    for ch in text.chars() {
        if ch == ' ' || ch == '\t' {
            if run {
                return true;
            }
            run = true;
        } else {
            run = false;
        }
    }
    false
}

fn check_tags(tags: &[(String, String)]) -> Result<(), AccountError> {
    if tags.len() > MAX_TAGS_PER_EDIT {
        return Err(AccountError::Invalid(format!(
            "an edit may set at most {MAX_TAGS_PER_EDIT} tags; this one named {}",
            tags.len()
        )));
    }
    for (name, value) in tags {
        check_tag_name(name)?;
        check_tag_value(value)?;
    }
    Ok(())
}

fn check_tag_name(name: &str) -> Result<(), AccountError> {
    let invalid = |why: &str| Err(AccountError::Invalid(format!("a tag name {why}")));
    if name.is_empty() {
        return invalid("may not be empty");
    }
    if name.len() > MAX_TAG_NAME_BYTES {
        return invalid(&format!(
            "is {} bytes; the limit is {MAX_TAG_NAME_BYTES}",
            name.len()
        ));
    }
    if name.trim() != name {
        return invalid("may not begin or end with whitespace");
    }
    if name.split_whitespace().count() > 1 {
        return invalid(
            "may not contain whitespace: hledger reads only the last word before the `:` as the \
             tag name, so anything before it would be silently dropped",
        );
    }
    if name.contains(',') || name.contains(':') {
        return invalid("may not contain `,` or `:`");
    }
    if has_control(name) {
        return invalid("may not contain a control character");
    }
    Ok(())
}

fn check_tag_value(value: &str) -> Result<(), AccountError> {
    let invalid = |why: &str| Err(AccountError::Invalid(format!("a tag value {why}")));
    if value.len() > MAX_TAG_VALUE_BYTES {
        return invalid(&format!(
            "is {} bytes; the limit is {MAX_TAG_VALUE_BYTES}",
            value.len()
        ));
    }
    if value.trim() != value {
        return invalid("may not begin or end with whitespace");
    }
    if value.contains(',') {
        return invalid(
            "may not contain `,`: hledger would read whatever follows it as a separate tag",
        );
    }
    if has_control(value) {
        return invalid("may not contain a control character");
    }
    Ok(())
}

/// Reject a note this module will not write: one whose own comma segments
/// would themselves classify as a tag under [`tag_in_segment`] — the exact
/// predicate a reparse applies, not an approximation of it.
fn check_note(note: &str) -> Result<(), AccountError> {
    if note.is_empty() {
        return Ok(());
    }
    if note.len() > MAX_NOTE_BYTES {
        return Err(AccountError::Invalid(format!(
            "a note is {} bytes; the limit is {MAX_NOTE_BYTES}",
            note.len()
        )));
    }
    if note.trim() != note {
        return Err(AccountError::Invalid(
            "a note may not begin or end with whitespace".to_string(),
        ));
    }
    if has_control(note) {
        return Err(AccountError::Invalid(
            "a note may not contain a control character".to_string(),
        ));
    }
    for segment in note.split(',') {
        if let Some((name, _)) = tag_in_segment(segment) {
            return Err(AccountError::Invalid(format!(
                "a note containing {:?} would be read back as a tag named \"{name}\": remove the \
                 comma or the colon",
                segment.trim()
            )));
        }
    }
    Ok(())
}

fn check_name(name: &str) -> Result<(), AccountError> {
    let invalid = |why: &str| Err(AccountError::Invalid(format!("an account name {why}")));
    if name.is_empty() {
        return invalid("may not be empty");
    }
    if name.len() > MAX_NAME_BYTES {
        return invalid(&format!(
            "is {} bytes; the limit is {MAX_NAME_BYTES}",
            name.len()
        ));
    }
    if name.trim() != name {
        return invalid("may not begin or end with whitespace");
    }
    if has_control(name) {
        return invalid("may not contain a control character");
    }
    if name.contains(';') || name.contains('#') {
        return invalid("may not contain `;` or `#`");
    }
    if contains_whitespace_run(name) {
        return invalid(
            "may not contain two consecutive spaces or tabs: hledger would read that as the end \
             of the name",
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Every top-level `account` line in `text`.
///
/// Skips a `comment`/`end comment` block exactly as
/// [`crate::parse::skip_comment_block`] does — an `account` line inside one is
/// not a directive at all, and counting it here would shift the index of
/// every declaration below it, landing an edit on the wrong line. Matches
/// [`crate::aliases::AliasDoc`]'s `scan` in shape, for the same reason.
fn scan(text: &str) -> Vec<AccountLine> {
    let mut lines: Vec<AccountLine> = Vec::new();
    let mut start = 0;
    let mut number = 0u32;
    let mut commented = false;
    while start < text.len() {
        let rest = &text[start..];
        let (content_len, full_len) = match rest.find('\n') {
            Some(at) => (at - usize::from(rest[..at].ends_with('\r')), at + 1),
            None => (rest.len(), rest.len()),
        };
        number = number.saturating_add(1);
        let content = &rest[..content_len];
        if commented {
            commented = content.trim() != "end comment";
        } else if content.split_whitespace().next() == Some("comment") {
            commented = true;
        } else if let Some(line) =
            account_line(start, content, content_len, full_len, lines.len(), number)
        {
            lines.push(line);
        }
        start += full_len;
    }
    lines
}

/// One line, if it is an `account` directive.
fn account_line(
    base: usize,
    content: &str,
    content_len: usize,
    full_len: usize,
    index: usize,
    number: u32,
) -> Option<AccountLine> {
    let after = content.strip_prefix("account")?;
    if !after.starts_with([' ', '\t']) {
        return None;
    }
    let leading_ws = after.len() - after.trim_start().len();
    let body = after.trim_start();
    let body_base = base + "account".len() + leading_ws;

    let (name, comment, tail) = split_account_name(body)?;
    let (tags, note) = classify(comment.unwrap_or(""));

    Some(AccountLine {
        index,
        line: number,
        name,
        tags,
        note,
        span: base..base + content_len,
        tail: (body_base + tail)..base + content_len,
        full: base..base + full_len,
    })
}

/// Split one account declaration's raw comment (everything after the `;`,
/// untrimmed at the front) into its tags and free-text note, using the exact
/// predicate [`crate::parse::parse_tags`] uses — so this is always in
/// agreement with [`crate::model::AccountDeclaration::tags`].
fn classify(comment: &str) -> (Vec<(String, String)>, String) {
    let content = comment.trim();
    if content.is_empty() {
        return (Vec::new(), String::new());
    }
    let mut tags = Vec::new();
    let mut note_segments: Vec<&str> = Vec::new();
    for segment in content.split(',') {
        if let Some(tag) = tag_in_segment(segment) {
            tags.push(tag);
        } else {
            let trimmed = segment.trim();
            if !trimmed.is_empty() {
                note_segments.push(trimmed);
            }
        }
    }
    (tags, note_segments.join(", "))
}

/// The `account` lines a journal file declares, read from disk.
///
/// A convenience for a caller that has a path rather than the text — exists so
/// the read and the parse cannot drift apart between callers, same as
/// [`crate::aliases::read_doc`].
///
/// # Errors
/// Whatever [`std::fs::read_to_string`] reports.
pub fn read_doc(path: &std::path::Path) -> std::io::Result<AccountDoc> {
    Ok(AccountDoc::parse(&std::fs::read_to_string(path)?))
}

/// Every account name [`Journal::account_tags`] would answer for, and whether
/// it carries an explicit declaration.
///
/// Not used by the document model itself — a small convenience so a caller
/// building the "which accounts are undeclared" view has one place to ask,
/// rather than re-deriving set membership over `journal.accounts` each time.
#[must_use]
pub fn is_declared(journal: &Journal, account: &str) -> bool {
    journal.accounts.iter().any(|decl| decl.name.0 == account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    fn doc(text: &str) -> AccountDoc {
        AccountDoc::parse(text)
    }

    #[test]
    fn scans_name_tags_and_note_with_the_right_tail_span() {
        let text = "account assets:cash  ; type: A, my note\n";
        let parsed = doc(text);
        assert_eq!(parsed.lines().len(), 1);
        let line = &parsed.lines()[0];
        assert_eq!(line.name, "assets:cash");
        assert_eq!(line.tags, vec![("type".to_string(), "A".to_string())]);
        assert_eq!(line.note, "my note");
        assert_eq!(&text[line.tail.clone()], "  ; type: A, my note");
        assert_eq!(line.line, 1);
    }

    #[test]
    fn a_bare_name_has_no_tags_or_note_and_an_empty_tail_at_line_end() {
        let text = "account assets:cash\n";
        let parsed = doc(text);
        let line = &parsed.lines()[0];
        assert!(line.tags.is_empty());
        assert_eq!(line.note, "");
        assert_eq!(
            line.tail,
            "account assets:cash".len().."account assets:cash".len()
        );
    }

    #[test]
    fn an_unedited_document_round_trips_byte_for_byte() {
        for text in [
            "",
            "account a:b\n",
            "account a:b",
            "account a:b  ; type: A\n",
            "\u{feff}account a\r\naccount b\r\n",
            "; note\naccount  a:b   ; type: A   \n\n2026-01-01 x\n    a:b  $1\n    c\n",
        ] {
            let parsed = doc(text);
            assert_eq!(
                parsed.apply(&AccountPlan::default()).unwrap(),
                text,
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_isolated_edit_leaves_every_other_byte_alone() {
        let text = "account a\naccount b  ; type: A\naccount c\n";
        let parsed = doc(text);
        let plan = AccountPlan {
            edits: vec![AccountEdit::Replace {
                index: 1,
                tags: vec![("type".to_string(), "L".to_string())],
                note: String::new(),
            }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "account a\naccount b  ; type: L\naccount c\n");
        parsed.verify(&plan, &out).unwrap();
    }

    #[test]
    fn a_delete_removes_exactly_one_line_and_its_terminator() {
        let text = "account a\naccount b  ; type: A\naccount c\n";
        let parsed = doc(text);
        let plan = AccountPlan {
            edits: vec![AccountEdit::Delete { index: 1 }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "account a\naccount c\n");
        parsed.verify(&plan, &out).unwrap();

        let journal = parse_journal(&out, "t.journal").unwrap();
        assert_eq!(journal.accounts.len(), 2);
    }

    #[test]
    fn deleting_the_last_line_still_lets_a_declare_land_in_the_same_batch() {
        let text = "account a\naccount b  ; type: A\n";
        let parsed = doc(text);
        let plan = AccountPlan {
            edits: vec![
                AccountEdit::Delete { index: 1 },
                AccountEdit::Declare {
                    name: "c".to_string(),
                    tags: Vec::new(),
                    note: String::new(),
                },
            ],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "account a\naccount c\n");
        parsed.verify(&plan, &out).unwrap();
    }

    #[test]
    fn a_locked_index_may_not_be_named_by_both_a_replace_and_a_delete() {
        let parsed = doc("account a\n");
        let error = parsed
            .apply(&AccountPlan {
                edits: vec![
                    AccountEdit::Delete { index: 0 },
                    AccountEdit::Replace {
                        index: 0,
                        tags: Vec::new(),
                        note: "x".to_string(),
                    },
                ],
            })
            .unwrap_err();
        assert_eq!(error, AccountError::DuplicateAccount(0));
    }

    #[test]
    fn clearing_tags_and_note_leaves_a_bare_name() {
        let parsed = doc("account a  ; type: A, old note\n");
        let plan = AccountPlan {
            edits: vec![AccountEdit::Replace {
                index: 0,
                tags: Vec::new(),
                note: String::new(),
            }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "account a\n");
        parsed.verify(&plan, &out).unwrap();
    }

    #[test]
    fn adding_a_comment_to_a_bare_name_uses_two_spaces_before_the_semicolon() {
        let parsed = doc("account a\n");
        let plan = AccountPlan {
            edits: vec![AccountEdit::Replace {
                index: 0,
                tags: vec![("type".to_string(), "A".to_string())],
                note: String::new(),
            }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "account a  ; type: A\n");
        parsed.verify(&plan, &out).unwrap();
        assert!(parse_journal(&out, "t.journal").is_ok());
    }

    #[test]
    fn declare_lands_after_the_last_account_line_not_at_eof() {
        let parsed = doc("account a\n2026-01-01 x\n    a  $1\n    b\n");
        let plan = AccountPlan {
            edits: vec![AccountEdit::Declare {
                name: "b".to_string(),
                tags: vec![("type".to_string(), "L".to_string())],
                note: String::new(),
            }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(
            out,
            "account a\naccount b  ; type: L\n2026-01-01 x\n    a  $1\n    b\n"
        );
        parsed.verify(&plan, &out).unwrap();
    }

    #[test]
    fn declare_lands_at_eof_when_there_is_no_account_line_yet() {
        let parsed = doc("2026-01-01 x\n    a  $1\n    b\n");
        let plan = AccountPlan {
            edits: vec![AccountEdit::Declare {
                name: "a".to_string(),
                tags: Vec::new(),
                note: "first declaration".to_string(),
            }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(
            out,
            "2026-01-01 x\n    a  $1\n    b\naccount a  ; first declaration\n"
        );
        parsed.verify(&plan, &out).unwrap();
        let journal = parse_journal(&out, "t.journal").unwrap();
        assert_eq!(journal.accounts.len(), 1);
    }

    #[test]
    fn declare_at_eof_supplies_a_missing_terminator() {
        // No account line to anchor to, and no final newline — the case that
        // would otherwise glue the directive onto whatever the last line was.
        let parsed = doc("2026-01-01 x\n    a  $1\n    b");
        let plan = AccountPlan {
            edits: vec![AccountEdit::Declare {
                name: "b".to_string(),
                tags: Vec::new(),
                note: String::new(),
            }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "2026-01-01 x\n    a  $1\n    b\naccount b\n");
        parsed.verify(&plan, &out).unwrap();
        assert!(parse_journal(&out, "t.journal").is_ok());
    }

    #[test]
    fn a_batch_of_declares_lands_in_the_order_given() {
        let parsed = doc("");
        let plan = AccountPlan {
            edits: vec![
                AccountEdit::Declare {
                    name: "assets:cash".to_string(),
                    tags: Vec::new(),
                    note: String::new(),
                },
                AccountEdit::Declare {
                    name: "expenses:food".to_string(),
                    tags: Vec::new(),
                    note: String::new(),
                },
            ],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "account assets:cash\naccount expenses:food\n");
        parsed.verify(&plan, &out).unwrap();
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        let parsed = doc("account a\r\n");
        let plan = AccountPlan {
            edits: vec![AccountEdit::Declare {
                name: "b".to_string(),
                tags: Vec::new(),
                note: String::new(),
            }],
        };
        assert_eq!(parsed.apply(&plan).unwrap(), "account a\r\naccount b\r\n");
    }

    #[test]
    fn an_account_line_inside_a_comment_block_is_not_one() {
        let text = "comment\naccount hidden\nend comment\naccount real\n";
        let parsed = doc(text);
        let names: Vec<&str> = parsed.lines().iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["real"]);
        assert_eq!(parsed.lines()[0].index, 0);
        assert_eq!(parsed.lines()[0].line, 4);

        let journal = parse_journal(text, "t.journal").unwrap();
        assert_eq!(journal.accounts.len(), 1);
        assert_eq!(journal.accounts[0].position.line, 4);
    }

    #[test]
    fn tag_and_note_classification_matches_the_parsers_own() {
        let text = "account a  ; type: A, bsgroup: Cash, some free text, holdings: none\n";
        let doc = doc(text);
        let journal = parse_journal(text, "t.journal").unwrap();
        assert_eq!(doc.lines()[0].tags, journal.accounts[0].tags);
    }

    #[test]
    fn values_that_would_not_read_back_are_refused() {
        let parsed = doc("account a\n");
        let replace = |tags: Vec<(&str, &str)>, note: &str| {
            parsed
                .apply(&AccountPlan {
                    edits: vec![AccountEdit::Replace {
                        index: 0,
                        tags: tags
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                        note: note.to_string(),
                    }],
                })
                .unwrap_err()
        };
        for (tags, note, needle) in [
            (vec![("two words", "A")], "", "may not contain whitespace"),
            (vec![("type", "A,B")], "", "separate tag"),
            (vec![], "ping bob, re: taxes", "read back as a tag"),
            (vec![], " leading space", "whitespace"),
            (vec![("", "A")], "", "may not be empty"),
        ] {
            let error = replace(tags.clone(), note);
            assert!(
                error.to_string().contains(needle),
                "{tags:?}/{note:?}: {error}"
            );
        }
    }

    #[test]
    fn a_name_with_two_spaces_is_refused_at_declare_time() {
        let parsed = doc("");
        let error = parsed
            .apply(&AccountPlan {
                edits: vec![AccountEdit::Declare {
                    name: "a  b".to_string(),
                    tags: Vec::new(),
                    note: String::new(),
                }],
            })
            .unwrap_err();
        assert!(
            error.to_string().contains("two consecutive spaces"),
            "{error}"
        );
    }

    #[test]
    fn a_stale_index_is_refused_rather_than_applied_elsewhere() {
        let parsed = doc("account a\n");
        let error = parsed
            .apply(&AccountPlan {
                edits: vec![AccountEdit::Replace {
                    index: 7,
                    tags: Vec::new(),
                    note: String::new(),
                }],
            })
            .unwrap_err();
        assert_eq!(error, AccountError::UnknownAccount(7));
    }

    #[test]
    fn naming_one_line_twice_is_refused() {
        let parsed = doc("account a\n");
        let error = parsed
            .apply(&AccountPlan {
                edits: vec![
                    AccountEdit::Replace {
                        index: 0,
                        tags: Vec::new(),
                        note: "one".to_string(),
                    },
                    AccountEdit::Replace {
                        index: 0,
                        tags: Vec::new(),
                        note: "two".to_string(),
                    },
                ],
            })
            .unwrap_err();
        assert_eq!(error, AccountError::DuplicateAccount(0));
    }

    #[test]
    fn verify_refuses_text_that_is_not_the_plan() {
        let parsed = doc("account a\n");
        let plan = AccountPlan {
            edits: vec![AccountEdit::Replace {
                index: 0,
                tags: vec![("type".to_string(), "A".to_string())],
                note: String::new(),
            }],
        };
        assert_eq!(
            parsed.verify(&plan, "account a  ; type: L\n").unwrap_err(),
            AccountError::RoundTripMismatch
        );
    }
}
