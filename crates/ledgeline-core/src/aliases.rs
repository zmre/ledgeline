//! `alias` directives: what the import pipeline forwards, and how one line of a
//! journal is edited without touching another byte.
//!
//! Two jobs, and they are separate on purpose.
//!
//! # 1. Forwarding — the whole reason this module exists
//!
//! An `alias` directive sitting in a journal **does not reach the CSV during an
//! import**. Verified against hledger 1.52: with
//! `alias PW Roth IRA - 3077 = assets:morganstanley:pw-roth-ira` in the target
//! journal, `hledger import --dry-run` proposed postings to the account
//! `PW Roth IRA - 3077`, unmapped. The `--alias` *option* does reach it, in both
//! `OLD=NEW` and `/REGEX/=REPL` forms, and several compose.
//!
//! So [`forward`] turns the journal's own alias directives into `--alias`
//! arguments, and the server puts them on every hledger invocation that reads a
//! statement. That is the entire feature: a mapping the user already wrote down,
//! delivered to the one place hledger would not deliver it itself.
//!
//! Two rules bound it, and both are refusals rather than guesses:
//!
//! - **An alias closed by `end aliases` is never forwarded.** `--alias` is
//!   global and the user wrote down where that one stops. See
//!   [`Journal::aliases_in_force`](crate::model::Journal::aliases_in_force).
//! - **An alias this module will not put on a command line is not forwarded,
//!   and says why** ([`AliasRefusal`]). It is reported, not dropped: a mapping
//!   that silently did not apply is the failure mode the whole preview exists to
//!   prevent.
//!
//! # 2. Editing — the span document model, one line wide
//!
//! `rules.rs` states the discipline this follows, and states it for a file
//! Ledgeline may rewrite wholesale. A journal is the riskiest write path in the
//! app, so the same discipline is applied *more* narrowly rather than less:
//!
//! > **An edit rewrites bytes only inside the spans it names, and every other
//! > byte of the file comes out the `&str` slice it went in as.**
//!
//! Here that is enforced by construction. [`AliasDoc::apply`] copies the
//! original text verbatim between the spans it splices, and the only spans it
//! will splice are one alias line's **pattern** and **replacement** extents —
//! never the `alias` keyword, never the `=`, never the whitespace between them.
//! A column-aligned block of aliases therefore stays aligned with no alignment
//! code anywhere, for the same reason `rules.rs` needs none.
//!
//! [`AliasDoc::verify`] then refuses rather than trusting that: it re-renders
//! the plan and requires the bytes to match, re-parses the result, and requires
//! every *unedited* alias line to come back byte-identical and every edited one
//! to read back as exactly what was asked for. A caller writes only if `verify`
//! agreed. The whole-journal re-parse — the check that the file still means
//! something — is the server's, because only the server knows which journal this
//! file belongs to (`parse_journal_with_overrides`).
//!
//! # What is deliberately NOT modeled
//!
//! Following `rules.rs`'s "when in doubt, opaque": a line this module cannot
//! promise to rewrite safely is presented **read-only**, with an [`AliasLock`]
//! naming what stopped it. It is still parsed, still listed, still forwarded if
//! it is forwardable. It simply cannot be edited here.
//!
//! The locks, each earned against the hledger binary:
//!
//! - [`AliasLock::CommentLike`] — a `;` or `#` on the line. `alias a = b ; note`
//!   declares the account **literally named** `b ; note`; hledger does not treat
//!   it as a comment (verified). Editing such a line would cement a reading its
//!   author almost certainly did not intend, so it is shown as it is. This is
//!   the same decision `rules.rs` makes for a `#`-led matcher.
//! - [`AliasLock::Empty`] — an empty pattern or replacement. There is no
//!   whitespace to re-emit verbatim on one side of the splice, so an edit would
//!   have to invent separator bytes; and an alias mapping to nothing is not a
//!   mapping.
//! - [`AliasLock::Delimiter`] — a plain pattern containing `=` (hledger splits
//!   at the first one, so the line does not say what it looks like it says), or
//!   a regex pattern containing an escaped `/`. Rewriting either means deciding
//!   how to re-escape it, and a wrong answer silently re-points the mapping.
//! - [`AliasLock::Control`] / [`AliasLock::TooLong`] — a byte this module will
//!   not write, or more of them than it will write.
//!
//! **An inserted alias goes immediately after the file's last alias line, or at
//! EOF when it has none** — and nowhere else. That is not a limitation dressed
//! up as a rule; it is the furthest-forward position that is provably still in
//! force where an import appends *and* provably unable to change what anything
//! already in the file means. See [`AliasDoc::insertion_point`].

use crate::model::{AliasDirective, Journal};
use crate::rules::Newline;
use std::cmp::Reverse;
use std::ops::Range;
use std::path::Path;
use thiserror::Error;

/// A byte range into [`AliasDoc::text`]. Byte offsets, like `rules::Span`, and
/// for the same reason: every one lands on a boundary the parser found in the
/// text, so slicing can never split a code point.
pub type Span = Range<usize>;

/// Longest alias pattern this module will write, in bytes. The same number
/// `rules.rs` uses for a matcher pattern, and for the same reason.
pub const MAX_PATTERN_BYTES: usize = 256;

/// Longest replacement this module will write, in bytes. An account name; the
/// same cap `rules.rs` puts on a directive value.
pub const MAX_REPLACEMENT_BYTES: usize = 512;

/// How many aliases are forwarded to one hledger invocation.
///
/// A bound on `argv`, not on the user: every real journal is orders of magnitude
/// under it. Aliases past the limit are reported as
/// [`AliasRefusal::Limit`] rather than quietly dropped, because the difference
/// between "hledger saw this mapping" and "hledger did not" is the whole
/// preview.
pub const MAX_FORWARDED: usize = 200;

// ---------------------------------------------------------------------------
// Forwarding
// ---------------------------------------------------------------------------

/// Why an alias is not put on an hledger command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasRefusal {
    /// An `end aliases` in the same file closed this alias's scope. `--alias` is
    /// global and has no way to say "only until line 40".
    Scoped,
    /// The pattern or the replacement is empty, so there is no mapping to make.
    Empty,
    /// A control character. Alias text becomes an `argv` entry and is echoed
    /// back in the UI; neither is a place for a `\n` or a `\0`.
    Control,
    /// Longer than [`MAX_PATTERN_BYTES`] / [`MAX_REPLACEMENT_BYTES`].
    TooLong,
    /// Past [`MAX_FORWARDED`].
    Limit,
}

impl AliasRefusal {
    /// A sentence completing "this alias is not used for imports because …",
    /// written for the person reading the screen.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Scoped => {
                "an `end aliases` line closes it before the end of its file, and hledger's \
                 --alias option has no way to express a scope that stops"
            }
            Self::Empty => "one side of it is empty, so it maps nothing to nothing",
            Self::Control => "it contains a control character",
            Self::TooLong => "it is longer than Ledgeline will put on a command line",
            Self::Limit => {
                "this journal declares more aliases than Ledgeline forwards to one import"
            }
        }
    }
}

/// One alias, and either the `--alias` argument it becomes or why it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forwarded {
    /// The pattern as written, without a regex's slashes.
    pub pattern: String,
    /// The replacement as written.
    pub replacement: String,
    /// Whether the pattern is the `/REGEX/` form.
    pub regex: bool,
    /// The `--alias` argument (`OLD=NEW` or `/REGEX/=REPL`), or the refusal.
    pub argument: Result<String, AliasRefusal>,
}

impl Forwarded {
    /// The `--alias` argument, when there is one.
    #[must_use]
    pub fn argument(&self) -> Option<&str> {
        self.argument.as_deref().ok()
    }
}

/// Every alias the journal declares, in file order, each with its `--alias`
/// argument or the reason it has none.
///
/// The list is complete — refusals included — because the caller's job is to
/// *show* it. A UI that only ever saw the forwarded ones could not tell a user
/// why the mapping they wrote did not happen.
#[must_use]
pub fn forward(journal: &Journal) -> Vec<Forwarded> {
    journal
        .aliases
        .iter()
        .scan(0usize, |taken, alias| {
            let argument = argument_for(alias).and_then(|argument| {
                *taken += 1;
                if *taken > MAX_FORWARDED {
                    Err(AliasRefusal::Limit)
                } else {
                    Ok(argument)
                }
            });
            Some(Forwarded {
                pattern: alias.pattern.clone(),
                replacement: alias.replacement.clone(),
                regex: alias.regex,
                argument,
            })
        })
        .collect()
}

/// Just the arguments, in order — what the server hands to `--alias`.
#[must_use]
pub fn arguments(journal: &Journal) -> Vec<String> {
    forward(journal)
        .into_iter()
        .filter_map(|forwarded| forwarded.argument.ok())
        .collect()
}

/// The `--alias` argument for one alias, or why it has none.
///
/// The content refusals are checked **before** the scope one, so an alias that
/// is both empty and scoped-closed is reported as empty: "you mapped nothing" is
/// the answer the person reading it can act on, and it stays true if they later
/// delete the `end aliases`.
fn argument_for(alias: &AliasDirective) -> Result<String, AliasRefusal> {
    if alias.pattern.is_empty() || alias.replacement.is_empty() {
        return Err(AliasRefusal::Empty);
    }
    if has_control(&alias.pattern) || has_control(&alias.replacement) {
        return Err(AliasRefusal::Control);
    }
    if alias.pattern.len() > MAX_PATTERN_BYTES || alias.replacement.len() > MAX_REPLACEMENT_BYTES {
        return Err(AliasRefusal::TooLong);
    }
    if alias.ended {
        return Err(AliasRefusal::Scoped);
    }
    Ok(format!(
        "{}={}",
        render_pattern(&alias.pattern, alias.regex),
        alias.replacement
    ))
}

/// A pattern as it is written on a line and on a command line: bare, or wrapped
/// in the slashes that make hledger read it as a regex.
fn render_pattern(pattern: &str, regex: bool) -> String {
    if regex {
        format!("/{pattern}/")
    } else {
        pattern.to_string()
    }
}

/// Any ASCII control character. Same test, and same reasoning, as
/// `rules::check_text`.
fn has_control(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_control())
}

// ---------------------------------------------------------------------------
// The document model
// ---------------------------------------------------------------------------

/// Why one alias line is presented read-only. See the module docs for the
/// argument behind each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasLock {
    /// A `;` or `#` on the line, which hledger reads as part of the account
    /// name rather than as a comment.
    CommentLike,
    /// An empty pattern or replacement.
    Empty,
    /// A delimiter this module will not re-derive: `=` inside a plain pattern,
    /// or `\/` inside a regex one.
    Delimiter,
    /// A control character on the line.
    Control,
    /// Past [`MAX_PATTERN_BYTES`] / [`MAX_REPLACEMENT_BYTES`].
    TooLong,
}

impl AliasLock {
    /// A sentence completing "this alias cannot be edited here because …".
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::CommentLike => {
                "it carries a `;` or `#`, which hledger reads as part of the account name and not \
                 as a comment; rewriting the line would cement a reading its author probably did \
                 not intend"
            }
            Self::Empty => "one side of it is empty, so there is nothing to rewrite",
            Self::Delimiter => {
                "its pattern contains a delimiter (`=`, or an escaped `/`) whose re-escaping \
                 Ledgeline will not guess at"
            }
            Self::Control => "it contains a control character",
            Self::TooLong => "it is longer than Ledgeline will rewrite",
        }
    }
}

/// One `alias` line, located in a file's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasLine {
    /// 0-based position among **this file's alias lines** — the handle an edit
    /// names. Like `rules::ItemId`, it is a parse-time index and is not durable
    /// across saves; the revision is what makes that safe.
    pub index: usize,
    /// 1-based file line, numbered LF-only exactly as [`str::lines`] does.
    pub line: u32,
    /// The line's content, without its terminator.
    pub span: Span,
    /// The line's content **with** its terminator — what a delete removes.
    pub full: Span,
    /// The pattern's extent, **including** a regex's slashes.
    pub pattern_span: Span,
    /// The replacement's extent, trimmed.
    pub replacement_span: Span,
    /// The pattern as written, without a regex's slashes.
    pub pattern: String,
    /// The replacement as written.
    pub replacement: String,
    /// Whether the pattern is the `/REGEX/` form.
    pub regex: bool,
    /// `Some` when the line is read-only.
    pub lock: Option<AliasLock>,
}

impl AliasLine {
    /// Whether this line may be rewritten. A locked line can still be deleted —
    /// removing a whole top-level line cannot inject anything, which is the same
    /// narrowing `rules::replaceable` makes.
    #[must_use]
    pub fn editable(&self) -> bool {
        self.lock.is_none()
    }
}

/// One journal file's alias lines, over its original text.
///
/// Immutable. An edit is an [`AliasPlan`] rendered by [`AliasDoc::apply`], which
/// returns a new `String` and never mutates `self`, so a refused edit cannot
/// leave half a document behind.
#[derive(Debug, Clone)]
pub struct AliasDoc {
    text: String,
    newline: Newline,
    lines: Vec<AliasLine>,
}

/// One change to one alias line.
///
/// Note what is absent: no variant carries a rendered line. Every byte
/// [`AliasDoc::apply`] writes is either a byte read from the file moments
/// earlier or this module's own rendering of a validated pattern/replacement
/// pair — the structural guarantee `rules::ItemBody` makes, for the same reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasEdit {
    /// Rewrite one line's pattern and replacement, leaving its keyword,
    /// separator and terminator exactly as they are.
    Replace {
        /// Which line, by [`AliasLine::index`].
        index: usize,
        /// The new pattern, without a regex's slashes.
        pattern: String,
        /// The new replacement.
        replacement: String,
        /// Whether to write it as `/REGEX/`.
        regex: bool,
    },
    /// Remove one line, terminator and all.
    Delete {
        /// Which line, by [`AliasLine::index`].
        index: usize,
    },
    /// Append a new alias at the end of the file. See the module docs for why
    /// that is the only insertion point offered.
    Append {
        /// The pattern, without a regex's slashes.
        pattern: String,
        /// The replacement.
        replacement: String,
        /// Whether to write it as `/REGEX/`.
        regex: bool,
    },
}

/// A complete set of changes to one file's aliases.
///
/// Unlike `rules::EditPlan` there is no `order`, and omission is *not* an error:
/// this plan cannot reorder or delete by omission, so a client that sends one
/// edit changes one line. Reordering EXISTING aliases is deliberately not
/// offered — aliases are positional, so a reorder is a semantic change
/// wearing a cosmetic's clothes.
///
/// The one exception: multiple [`AliasEdit::Append`]s in the same plan are
/// free to arrive in any order, but are always WRITTEN in descending pattern
/// length (see [`AliasDoc::specificity_ordered`]) — not the client's array
/// order — so a batch that appends both a parent account's alias and one of
/// its subaccounts' never lets the shorter, broader pattern shadow the
/// longer, more specific one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AliasPlan {
    /// The changes, in any order. At most one per existing line.
    pub edits: Vec<AliasEdit>,
}

/// Errors from planning or checking an alias rewrite.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AliasError {
    /// An edit named a line this file does not have — almost always a stale
    /// index from a client that planned against an older parse.
    #[error("this file has no alias number {0}")]
    UnknownAlias(usize),
    /// Two edits named the same line.
    #[error("alias number {0} was named by more than one change")]
    DuplicateAlias(usize),
    /// An edit asked to rewrite a line this module presents read-only.
    #[error("alias number {index} cannot be edited here because {why}")]
    Locked {
        /// Which line.
        index: usize,
        /// The lock's own sentence.
        why: &'static str,
    },
    /// A client-supplied string is not something this module will write into a
    /// journal.
    #[error("{0}")]
    Invalid(String),
    /// The rewritten text could not be proved to be the requested edit and
    /// nothing else, so the caller must write nothing.
    #[error("the rewritten journal failed its round-trip check; nothing was written")]
    RoundTripMismatch,
}

impl AliasDoc {
    /// Parse one journal file's alias lines. **Infallible** — a file always
    /// opens, and a line this module will not rewrite becomes a locked line
    /// rather than a refusal, exactly as `RulesDoc::parse` never fails.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let lines = scan(text);
        Self {
            text: text.to_string(),
            newline: Newline::detect(text),
            lines,
        }
    }

    /// The original text, byte for byte.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The alias lines, in file order. Index equals [`AliasLine::index`].
    #[must_use]
    pub fn lines(&self) -> &[AliasLine] {
        &self.lines
    }

    /// Render the file under `plan`. Pure; no I/O; `self` is untouched.
    ///
    /// # Errors
    /// Validation runs to completion before a byte is rendered, so a rejected
    /// plan never produces partial text: [`AliasError::UnknownAlias`],
    /// [`AliasError::DuplicateAlias`], [`AliasError::Locked`] or
    /// [`AliasError::Invalid`].
    pub fn apply(&self, plan: &AliasPlan) -> Result<String, AliasError> {
        let splices = self.splices(plan)?;
        // Every byte outside a splice is copied from `self.text` verbatim. That
        // is the isolation guarantee, and it holds by construction rather than
        // by inspection.
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
    /// Byte preservation alone is not enough — the same argument `rules::verify`
    /// makes — so this re-parses and checks meaning:
    ///
    /// 1. re-render the plan and require `new_text` to match byte for byte;
    /// 2. require the re-parse to hold exactly the alias lines the plan implies;
    /// 3. require every **unedited** surviving line to come back byte-identical
    ///    to its original span content;
    /// 4. require every edited or appended line to read back as exactly the
    ///    pattern/replacement/regex that was requested — the render→parse
    ///    fixpoint, which is what catches a value that would be written and then
    ///    read back as something else.
    ///
    /// What is deliberately *not* here: whether the file still parses as part of
    /// its journal. Only the caller knows which journal this file belongs to, so
    /// that check is the caller's (`parse_journal_with_overrides`), and the
    /// server does it before writing.
    ///
    /// # Errors
    /// [`AliasError::RoundTripMismatch`] if any step fails, plus whatever
    /// [`AliasDoc::apply`] reports for an invalid plan.
    pub fn verify(&self, plan: &AliasPlan, new_text: &str) -> Result<(), AliasError> {
        if self.apply(plan)? != new_text {
            return Err(AliasError::RoundTripMismatch);
        }
        let reparsed = Self::parse(new_text);

        let deleted: Vec<usize> = plan
            .edits
            .iter()
            .filter_map(|edit| match edit {
                AliasEdit::Delete { index } => Some(*index),
                _ => None,
            })
            .collect();
        let survivors: Vec<&AliasLine> = self
            .lines
            .iter()
            .filter(|line| !deleted.contains(&line.index))
            .collect();
        let appended = plan
            .edits
            .iter()
            .filter(|edit| matches!(edit, AliasEdit::Append { .. }))
            .count();
        if reparsed.lines.len() != survivors.len() + appended {
            return Err(AliasError::RoundTripMismatch);
        }

        // Each surviving original line, in order, against its counterpart.
        for (before, after) in survivors.iter().zip(reparsed.lines.iter()) {
            match plan.edit_for(before.index) {
                Some(AliasEdit::Replace {
                    pattern,
                    replacement,
                    regex,
                    ..
                }) => {
                    if &after.pattern != pattern
                        || &after.replacement != replacement
                        || after.regex != *regex
                    {
                        return Err(AliasError::RoundTripMismatch);
                    }
                }
                // Untouched: byte-identical, which is the isolation property
                // stated as a check rather than as a hope.
                _ => {
                    if new_text[after.span.clone()] != self.text[before.span.clone()] {
                        return Err(AliasError::RoundTripMismatch);
                    }
                }
            }
        }

        // Then the appended ones, which sit after every survivor, in the same
        // specificity order `splices` actually wrote them in — not
        // `plan.edits`'s own order.
        let mut tail = reparsed.lines[survivors.len()..].iter();
        for edit in &Self::specificity_ordered(&plan.edits) {
            if let AliasEdit::Append {
                pattern,
                replacement,
                regex,
            } = edit
            {
                let Some(after) = tail.next() else {
                    return Err(AliasError::RoundTripMismatch);
                };
                if &after.pattern != pattern
                    || &after.replacement != replacement
                    || after.regex != *regex
                {
                    return Err(AliasError::RoundTripMismatch);
                }
            }
        }
        Ok(())
    }

    /// The ordered, non-overlapping `(span, text)` list one plan splices.
    fn splices(&self, plan: &AliasPlan) -> Result<Vec<(Span, String)>, AliasError> {
        self.check_plan(plan)?;
        let ordered = Self::specificity_ordered(&plan.edits);
        let mut splices: Vec<(Span, String)> = Vec::new();
        for edit in &ordered {
            match edit {
                AliasEdit::Replace {
                    index,
                    pattern,
                    replacement,
                    regex,
                } => {
                    let line = self.line(*index)?;
                    // Two splices, not one: the `=` and every space around it
                    // are re-emitted verbatim, so a column-aligned block stays
                    // aligned without a line of alignment code.
                    splices.push((line.pattern_span.clone(), render_pattern(pattern, *regex)));
                    splices.push((line.replacement_span.clone(), replacement.clone()));
                }
                AliasEdit::Delete { index } => {
                    let line = self.line(*index)?;
                    splices.push((line.full.clone(), String::new()));
                }
                AliasEdit::Append {
                    pattern,
                    replacement,
                    regex,
                } => {
                    let at = self.insertion_point();
                    splices.push((at..at, self.appended(at, pattern, replacement, *regex)));
                }
            }
        }
        splices.sort_by_key(|(span, _)| (span.start, span.end));
        Ok(splices)
    }

    /// Reorder one plan's [`AliasEdit::Append`]s by descending pattern length,
    /// leaving every other edit exactly where it was.
    ///
    /// All appends in one plan land at the same [`Self::insertion_point`], so
    /// their relative order among themselves is otherwise whatever order the
    /// client happened to send — and both `resolve_account`
    /// (`crate::qb_import`) and hledger's own `--alias` composition are
    /// first-match-wins in file order. A parent account's alias
    /// (`4000 Sales Revenue = revenue:sales`) is a colon-segment PREFIX of one
    /// of its subaccounts' raw name (`4000 Sales Revenue:4021 Enterprise
    /// Subscription`), so if the parent's line lands first, its own
    /// prefix-cascade rule silently resolves the subaccount before the
    /// subaccount's own, more specific alias is ever reached — even though
    /// both were typed in the same batch. A colon-segment prefix is always
    /// shorter than what it prefixes, so sorting by descending length puts
    /// every subaccount's own alias ahead of any ancestor's, without needing
    /// to parse the colon structure at all. Ties (unrelated patterns of equal
    /// length) keep their original relative order — the sort is stable, and
    /// two patterns that don't prefix one another are never order-dependent
    /// in the first place.
    fn specificity_ordered(edits: &[AliasEdit]) -> Vec<AliasEdit> {
        let mut appends: Vec<AliasEdit> = edits
            .iter()
            .filter(|edit| matches!(edit, AliasEdit::Append { .. }))
            .cloned()
            .collect();
        appends.sort_by_key(|edit| match edit {
            AliasEdit::Append { pattern, .. } => Reverse(pattern.len()),
            AliasEdit::Replace { .. } | AliasEdit::Delete { .. } => {
                unreachable!("filtered to Append above")
            }
        });
        let mut appends = appends.into_iter();
        edits
            .iter()
            .map(|edit| {
                if matches!(edit, AliasEdit::Append { .. }) {
                    appends.next().expect("as many appends as were filtered in")
                } else {
                    edit.clone()
                }
            })
            .collect()
    }

    /// Where an [`AliasEdit::Append`] puts its line.
    ///
    /// **Immediately after the file's last alias line, or at EOF when it has
    /// none** — and the two cases are one rule, not a special case and a
    /// fallback. The rule is: the furthest-forward position that is provably
    /// (a) still in force where an import appends, and (b) unable to change the
    /// meaning of anything already in the file.
    ///
    /// Sitting after the last alias satisfies both. An `alias` is a single-line
    /// column-1 directive whose extent depends on nothing around it, so inserting
    /// between two lines cannot merge with either; and if the last alias is
    /// itself in force at EOF then so is a line placed after it. It also keeps
    /// the user's mapping table together, which the EOF-only rule did not — a new
    /// alias landing at the bottom of a file whose others are at the top is
    /// technically correct and practically a mess.
    ///
    /// The one thing that breaks (a) is an `end aliases` after the last alias:
    /// the new line would be inside the closed scope, in force nowhere. Then the
    /// answer is EOF, which is past every `end aliases` by definition.
    fn insertion_point(&self) -> usize {
        let Some(last) = self.lines.last() else {
            return self.text.len();
        };
        if self.text[last.full.end..]
            .lines()
            .any(|line| line.split_whitespace().collect::<Vec<_>>() == ["end", "aliases"])
        {
            return self.text.len();
        }
        last.full.end
    }

    /// The text an [`AliasEdit::Append`] inserts at `at`.
    ///
    /// A file whose last line has no terminator gets one first — the case
    /// `rules.rs` calls out as the one that bites, and it bites harder here:
    /// without it the new directive would be glued onto whatever the last line
    /// was, turning two constructs into one.
    ///
    /// No blank line is added, deliberately. `rules.rs` supplies one only where a
    /// construct's *extent* needs it to end, and no journal construct works that
    /// way — a column-1 directive already ends whatever precedes it. Adding one
    /// for looks would be writing bytes nobody asked for into the file this
    /// module exists to leave alone.
    fn appended(&self, at: usize, pattern: &str, replacement: &str, regex: bool) -> String {
        let newline = self.newline.as_str();
        let lead = if at == 0 || self.text[..at].ends_with('\n') {
            ""
        } else {
            newline
        };
        format!(
            "{lead}alias {} = {replacement}{newline}",
            render_pattern(pattern, regex)
        )
    }

    /// Reject a plan before a byte of it is rendered.
    fn check_plan(&self, plan: &AliasPlan) -> Result<(), AliasError> {
        let mut named: Vec<usize> = Vec::new();
        for edit in &plan.edits {
            if let Some(index) = edit.index() {
                let line = self.line(index)?;
                if named.contains(&index) {
                    return Err(AliasError::DuplicateAlias(index));
                }
                named.push(index);
                if let (Some(lock), AliasEdit::Replace { .. }) = (line.lock, edit) {
                    return Err(AliasError::Locked {
                        index,
                        why: lock.message(),
                    });
                }
            }
            if let Some((pattern, replacement, regex)) = edit.content() {
                check_pattern(pattern, regex)?;
                check_replacement(replacement)?;
            }
        }
        Ok(())
    }

    /// The line with this index.
    fn line(&self, index: usize) -> Result<&AliasLine, AliasError> {
        self.lines.get(index).ok_or(AliasError::UnknownAlias(index))
    }
}

impl AliasEdit {
    /// The existing line this edit names, or `None` for an append.
    #[must_use]
    pub fn index(&self) -> Option<usize> {
        match self {
            Self::Replace { index, .. } | Self::Delete { index } => Some(*index),
            Self::Append { .. } => None,
        }
    }

    /// The client-supplied content, or `None` for a delete.
    fn content(&self) -> Option<(&str, &str, bool)> {
        match self {
            Self::Replace {
                pattern,
                replacement,
                regex,
                ..
            }
            | Self::Append {
                pattern,
                replacement,
                regex,
            } => Some((pattern, replacement, *regex)),
            Self::Delete { .. } => None,
        }
    }
}

impl AliasPlan {
    /// The edit naming `index`, if any.
    fn edit_for(&self, index: usize) -> Option<&AliasEdit> {
        self.edits.iter().find(|edit| edit.index() == Some(index))
    }
}

// ---------------------------------------------------------------------------
// Value validation
// ---------------------------------------------------------------------------

/// Reject a pattern this module will not write.
///
/// Every rule here is one that would make the written line read back as
/// something other than what was asked for — the failure `verify` would catch
/// afterwards, reported up front with a sentence instead.
fn check_pattern(pattern: &str, regex: bool) -> Result<(), AliasError> {
    let invalid = |why: &str| Err(AliasError::Invalid(format!("an alias pattern {why}")));
    if pattern.is_empty() {
        return invalid("may not be empty");
    }
    if pattern.len() > MAX_PATTERN_BYTES {
        return invalid(&format!(
            "is {} bytes; the limit is {MAX_PATTERN_BYTES}",
            pattern.len()
        ));
    }
    if has_control(pattern) {
        return invalid("may not contain a control character");
    }
    if pattern.trim() != pattern {
        return invalid(
            "may not begin or end with whitespace: hledger trims it, so the value \
                        would be written and then read back without it",
        );
    }
    if pattern.contains(';') || pattern.contains('#') {
        return invalid(
            "may not contain `;` or `#`: hledger reads those as part of the account name rather \
             than as a comment",
        );
    }
    if regex {
        if crate::parse::unescaped_slash(pattern).is_some() {
            return invalid(
                "written as a regular expression may not contain an unescaped `/`, which would \
                 close the pattern early",
            );
        }
        if pattern.contains('\\') {
            return invalid(
                "written as a regular expression may not contain a backslash escape here: \
                 Ledgeline will not re-derive one it did not write",
            );
        }
    } else {
        if pattern.starts_with('/') {
            return invalid("may not begin with `/` unless it is a regular expression");
        }
        if pattern.contains('=') {
            return invalid(
                "may not contain `=`: hledger splits the line at the first one, so the mapping \
                 would not be the one asked for",
            );
        }
    }
    Ok(())
}

/// Reject a replacement this module will not write.
fn check_replacement(replacement: &str) -> Result<(), AliasError> {
    let invalid = |why: &str| Err(AliasError::Invalid(format!("an alias replacement {why}")));
    if replacement.is_empty() {
        return invalid("may not be empty");
    }
    if replacement.len() > MAX_REPLACEMENT_BYTES {
        return invalid(&format!(
            "is {} bytes; the limit is {MAX_REPLACEMENT_BYTES}",
            replacement.len()
        ));
    }
    if has_control(replacement) {
        return invalid("may not contain a control character");
    }
    if replacement.trim() != replacement {
        return invalid(
            "may not begin or end with whitespace: hledger trims it, so the value \
                        would be written and then read back without it",
        );
    }
    if replacement.contains(';') || replacement.contains('#') {
        return invalid(
            "may not contain `;` or `#`: hledger reads those as part of the account name rather \
             than as a comment, so `b ; note` is an account called `b ; note`",
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Every `alias` line in `text`, with the spans of its parts.
///
/// A directive is top-level, so a line qualifies only when `alias` starts at
/// column 1 and is followed by whitespace — which is exactly the dispatch
/// `parse.rs` performs.
///
/// The two must agree on the *set*, not merely the shape, because an
/// [`AliasLine::index`] is an ordinal and the server joins it against the
/// parser's own list. `comment`/`end comment` is the one construct that would
/// break that: hledger swallows everything between them, so an `alias` line in
/// a commented-out block is not a directive at all, and counting it here would
/// shift the index of every alias below it — an edit landing on the wrong line.
/// So the block is skipped, with the parser's exact (deliberately hazardous)
/// unterminated-block behaviour: it runs to EOF.
fn scan(text: &str) -> Vec<AliasLine> {
    let mut lines: Vec<AliasLine> = Vec::new();
    let mut start = 0;
    let mut number = 0u32;
    let mut commented = false;
    while start < text.len() {
        let rest = &text[start..];
        let (content_len, full_len) = match rest.find('\n') {
            // A `\r\n` file keeps its `\r` out of the line's content and inside
            // its terminator, so a delete removes both and a span never ends
            // mid-terminator.
            Some(at) => (at - usize::from(rest[..at].ends_with('\r')), at + 1),
            None => (rest.len(), rest.len()),
        };
        number = number.saturating_add(1);
        let content = &rest[..content_len];
        if commented {
            commented = content.trim() != "end comment";
        } else if content.split_whitespace().next() == Some("comment") {
            commented = true;
        } else if let Some(line) = alias_line(start, content, full_len, lines.len(), number) {
            lines.push(line);
        }
        start += full_len;
    }
    lines
}

/// One line, if it is an alias directive.
fn alias_line(
    base: usize,
    content: &str,
    full_len: usize,
    index: usize,
    number: u32,
) -> Option<AliasLine> {
    let after = content.strip_prefix("alias")?;
    if !after.starts_with([' ', '\t']) {
        return None;
    }
    let value_at = after.len() - after.trim_start().len();
    let value = after.trim_start();
    let value_base = base + "alias".len() + value_at;

    let (pattern_span, pattern, replacement_at, regex) = match value.strip_prefix('/') {
        Some(body) => {
            let close = crate::parse::unescaped_slash(body)?;
            let eq = body[close + 1..].find('=')? + close + 1;
            (
                value_base..value_base + close + 2,
                body[..close].trim().to_string(),
                // `eq` indexes `body`, which starts one byte into `value`.
                eq + 2,
                true,
            )
        }
        None => {
            let eq = value.find('=')?;
            let name = value[..eq].trim_end();
            (
                value_base..value_base + name.len(),
                name.trim().to_string(),
                eq + 1,
                false,
            )
        }
    };

    let tail = &value[replacement_at..];
    let lead = tail.len() - tail.trim_start().len();
    let replacement = tail.trim();
    let replacement_start = value_base + replacement_at + lead;
    let span = base..base + content.len();
    let full = base..base + full_len;

    Some(AliasLine {
        index,
        line: number,
        lock: lock_for(&pattern, replacement, regex),
        pattern,
        replacement: replacement.to_string(),
        regex,
        pattern_span,
        replacement_span: replacement_start..replacement_start + replacement.len(),
        span,
        full,
    })
}

/// Whether this line is read-only, and why. See [`AliasLock`].
fn lock_for(pattern: &str, replacement: &str, regex: bool) -> Option<AliasLock> {
    if pattern.contains(';')
        || pattern.contains('#')
        || replacement.contains(';')
        || replacement.contains('#')
    {
        return Some(AliasLock::CommentLike);
    }
    if pattern.is_empty() || replacement.is_empty() {
        return Some(AliasLock::Empty);
    }
    if has_control(pattern) || has_control(replacement) {
        return Some(AliasLock::Control);
    }
    if pattern.len() > MAX_PATTERN_BYTES || replacement.len() > MAX_REPLACEMENT_BYTES {
        return Some(AliasLock::TooLong);
    }
    if regex && pattern.contains('\\') {
        return Some(AliasLock::Delimiter);
    }
    if !regex && pattern.contains('=') {
        return Some(AliasLock::Delimiter);
    }
    None
}

/// The alias lines a journal file declares, read from disk.
///
/// A convenience for a caller that has a path rather than the text; it exists so
/// the read and the parse cannot drift apart between the two callers that do it.
///
/// # Errors
/// Whatever [`std::fs::read_to_string`] reports.
pub fn read_doc(path: &Path) -> std::io::Result<AliasDoc> {
    Ok(AliasDoc::parse(&std::fs::read_to_string(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    fn doc(text: &str) -> AliasDoc {
        AliasDoc::parse(text)
    }

    #[test]
    fn scans_both_forms_with_part_spans() {
        let text = "alias old:one = new:one\nalias /^PW (.+)$/ = assets:\\1\n";
        let parsed = doc(text);
        assert_eq!(parsed.lines().len(), 2);

        let plain = &parsed.lines()[0];
        assert_eq!(plain.pattern, "old:one");
        assert_eq!(plain.replacement, "new:one");
        assert!(!plain.regex);
        assert_eq!(&text[plain.pattern_span.clone()], "old:one");
        assert_eq!(&text[plain.replacement_span.clone()], "new:one");
        assert_eq!(plain.line, 1);

        let regex = &parsed.lines()[1];
        assert_eq!(regex.pattern, "^PW (.+)$");
        assert_eq!(regex.replacement, "assets:\\1");
        assert!(regex.regex);
        // The slashes are part of the pattern's extent, so switching forms is
        // one splice rather than three.
        assert_eq!(&text[regex.pattern_span.clone()], "/^PW (.+)$/");
        assert_eq!(regex.line, 2);
    }

    #[test]
    fn an_unedited_document_round_trips_byte_for_byte() {
        for text in [
            "",
            "alias a = b\n",
            "alias a = b",
            "\u{feff}alias a = b\r\nalias c = d\r\n",
            "; note\nalias  a   =   b   \n\n2026-01-01 x\n    a  $1\n    b\n",
        ] {
            let parsed = doc(text);
            assert_eq!(
                parsed.apply(&AliasPlan::default()).unwrap(),
                text,
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_isolated_edit_leaves_every_other_byte_alone() {
        let text = "alias one = a:one\nalias two = a:two\nalias three = a:three\n";
        let parsed = doc(text);
        let plan = AliasPlan {
            edits: vec![AliasEdit::Replace {
                index: 1,
                pattern: "two".to_string(),
                replacement: "b:two".to_string(),
                regex: false,
            }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(
            out,
            "alias one = a:one\nalias two = b:two\nalias three = a:three\n"
        );
        parsed.verify(&plan, &out).unwrap();
    }

    #[test]
    fn column_alignment_survives_because_the_separator_is_reused() {
        let text = "alias one      = a:one\nalias another  = a:another\n";
        let parsed = doc(text);
        let plan = AliasPlan {
            edits: vec![AliasEdit::Replace {
                index: 0,
                pattern: "one".to_string(),
                replacement: "z:one".to_string(),
                regex: false,
            }],
        };
        assert_eq!(
            parsed.apply(&plan).unwrap(),
            "alias one      = z:one\nalias another  = a:another\n"
        );
    }

    #[test]
    fn a_comment_bearing_alias_is_locked_not_rewritten() {
        // hledger reads `b ; note` as the account name, so this line is shown
        // and never edited.
        let parsed = doc("alias a = b ; note\n");
        assert_eq!(parsed.lines()[0].replacement, "b ; note");
        assert_eq!(parsed.lines()[0].lock, Some(AliasLock::CommentLike));
        assert!(!parsed.lines()[0].editable());

        let error = parsed
            .apply(&AliasPlan {
                edits: vec![AliasEdit::Replace {
                    index: 0,
                    pattern: "a".to_string(),
                    replacement: "b".to_string(),
                    regex: false,
                }],
            })
            .unwrap_err();
        assert!(
            matches!(error, AliasError::Locked { index: 0, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_locked_line_may_still_be_deleted() {
        let parsed = doc("alias a = b ; note\nalias c = d\n");
        let plan = AliasPlan {
            edits: vec![AliasEdit::Delete { index: 0 }],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(out, "alias c = d\n");
        parsed.verify(&plan, &out).unwrap();
    }

    /// The append plan used throughout the insertion-point tests.
    fn append(pattern: &str, replacement: &str) -> AliasPlan {
        AliasPlan {
            edits: vec![AliasEdit::Append {
                pattern: pattern.to_string(),
                replacement: replacement.to_string(),
                regex: false,
            }],
        }
    }

    #[test]
    fn append_lands_at_eof_and_supplies_a_missing_terminator() {
        // No alias to anchor to, and no final newline — the case that would
        // otherwise glue the directive onto the last posting.
        let parsed = doc("2026-01-01 x\n    a  $1\n    b");
        let plan = append("BANK 1234", "assets:bank");
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(
            out,
            "2026-01-01 x\n    a  $1\n    b\nalias BANK 1234 = assets:bank\n"
        );
        parsed.verify(&plan, &out).unwrap();
        assert!(parse_journal(&out, "t.journal").is_ok());
    }

    #[test]
    fn append_joins_the_existing_alias_block_rather_than_the_end_of_the_file() {
        // A new alias belongs with the others, and it is provably safe there: an
        // `alias` is a single-line column-1 directive, so nothing merges.
        let parsed = doc("alias one = a:one\n\n2026-01-01 x\n    a  $1\n    b\n");
        let plan = append("two", "a:two");
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(
            out,
            "alias one = a:one\nalias two = a:two\n\n2026-01-01 x\n    a  $1\n    b\n"
        );
        parsed.verify(&plan, &out).unwrap();
        assert!(parse_journal(&out, "t.journal").is_ok());
    }

    #[test]
    fn append_falls_back_to_eof_when_an_end_aliases_would_swallow_it() {
        // Placed beside the last alias, the new line would sit inside a scope
        // that has already been closed — in force nowhere, which is not what
        // "add an alias" means.
        let parsed = doc("alias one = a:one\nend aliases\n\n2026-01-01 x\n    a  $1\n    b\n");
        let plan = append("two", "a:two");
        let out = parsed.apply(&plan).unwrap();
        assert!(out.ends_with("alias two = a:two\n"), "{out}");

        let journal = parse_journal(&out, "t.journal").unwrap();
        let in_force: Vec<&str> = journal
            .aliases_in_force()
            .map(|alias| alias.pattern.as_str())
            .collect();
        assert_eq!(in_force, vec!["two"], "the new alias must be in force");
    }

    #[test]
    fn a_batch_of_appends_writes_the_more_specific_pattern_first_regardless_of_input_order() {
        // The reported real-world shape: a parent account's own alias is a
        // colon-segment prefix of one of its subaccounts', and both are typed
        // into the same "map every unmapped account" batch. If the shorter,
        // parent pattern lands first, `qb_import::resolve_account`'s
        // first-match-wins prefix cascade would resolve the subaccount via
        // the PARENT's alias before ever reaching the subaccount's own.
        let parsed = doc("2026-01-01 x\n    a  $1\n    b\n");
        let plan = AliasPlan {
            edits: vec![
                AliasEdit::Append {
                    pattern: "4000 Sales Revenue".to_string(),
                    replacement: "revenue:sales".to_string(),
                    regex: false,
                },
                AliasEdit::Append {
                    pattern: "4000 Sales Revenue:4021 Enterprise Subscription".to_string(),
                    replacement: "revenue:sales:enterprise-subscription".to_string(),
                    regex: false,
                },
            ],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(
            out,
            "2026-01-01 x\n    a  $1\n    b\n\
             alias 4000 Sales Revenue:4021 Enterprise Subscription = revenue:sales:enterprise-subscription\n\
             alias 4000 Sales Revenue = revenue:sales\n"
        );
        parsed.verify(&plan, &out).unwrap();

        let journal = parse_journal(&out, "t.journal").unwrap();
        let aliases: Vec<&AliasDirective> = journal.aliases_in_force().collect();
        assert_eq!(
            crate::qb_import::resolve_account(
                "4000 Sales Revenue:4021 Enterprise Subscription",
                &aliases
            ),
            Some("revenue:sales:enterprise-subscription".to_string()),
            "the subaccount's own, more specific alias must win, not the parent's cascade"
        );
        assert_eq!(
            crate::qb_import::resolve_account("4000 Sales Revenue", &aliases),
            Some("revenue:sales".to_string())
        );
    }

    #[test]
    fn appends_of_equal_length_keep_their_original_relative_order() {
        let parsed = doc("2026-01-01 x\n    a  $1\n    b\n");
        let plan = AliasPlan {
            edits: vec![
                AliasEdit::Append {
                    pattern: "aaa".to_string(),
                    replacement: "one".to_string(),
                    regex: false,
                },
                AliasEdit::Append {
                    pattern: "bbb".to_string(),
                    replacement: "two".to_string(),
                    regex: false,
                },
            ],
        };
        let out = parsed.apply(&plan).unwrap();
        assert_eq!(
            out,
            "2026-01-01 x\n    a  $1\n    b\nalias aaa = one\nalias bbb = two\n"
        );
        parsed.verify(&plan, &out).unwrap();
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        let parsed = doc("alias a = b\r\n");
        let plan = AliasPlan {
            edits: vec![AliasEdit::Append {
                pattern: "c".to_string(),
                replacement: "d".to_string(),
                regex: true,
            }],
        };
        assert_eq!(
            parsed.apply(&plan).unwrap(),
            "alias a = b\r\nalias /c/ = d\r\n"
        );
    }

    #[test]
    fn values_that_would_not_read_back_are_refused() {
        let parsed = doc("alias a = b\n");
        let replace = |pattern: &str, replacement: &str, regex: bool| {
            parsed
                .apply(&AliasPlan {
                    edits: vec![AliasEdit::Replace {
                        index: 0,
                        pattern: pattern.to_string(),
                        replacement: replacement.to_string(),
                        regex,
                    }],
                })
                .unwrap_err()
        };
        for (pattern, replacement, regex, needle) in [
            ("a", "b\nalias x = y", false, "control character"),
            ("a\n", "b", false, "control character"),
            ("", "b", false, "may not be empty"),
            ("a", "", false, "may not be empty"),
            ("a=b", "c", false, "hledger splits the line"),
            ("/a/", "c", false, "may not begin with `/`"),
            ("a/b", "c", true, "unescaped `/`"),
            ("a", "b ; note", false, "`;` or `#`"),
            ("a", " b", false, "whitespace"),
        ] {
            let error = replace(pattern, replacement, regex);
            assert!(
                error.to_string().contains(needle),
                "{pattern:?}/{replacement:?}: {error}"
            );
        }
    }

    #[test]
    fn a_stale_index_is_refused_rather_than_applied_elsewhere() {
        let parsed = doc("alias a = b\n");
        let error = parsed
            .apply(&AliasPlan {
                edits: vec![AliasEdit::Delete { index: 7 }],
            })
            .unwrap_err();
        assert_eq!(error, AliasError::UnknownAlias(7));
    }

    #[test]
    fn naming_one_line_twice_is_refused() {
        let parsed = doc("alias a = b\n");
        let error = parsed
            .apply(&AliasPlan {
                edits: vec![
                    AliasEdit::Delete { index: 0 },
                    AliasEdit::Replace {
                        index: 0,
                        pattern: "a".to_string(),
                        replacement: "c".to_string(),
                        regex: false,
                    },
                ],
            })
            .unwrap_err();
        assert_eq!(error, AliasError::DuplicateAlias(0));
    }

    #[test]
    fn verify_refuses_text_that_is_not_the_plan() {
        let parsed = doc("alias a = b\n");
        let plan = AliasPlan {
            edits: vec![AliasEdit::Replace {
                index: 0,
                pattern: "a".to_string(),
                replacement: "c".to_string(),
                regex: false,
            }],
        };
        assert_eq!(
            parsed.verify(&plan, "alias a = d\n").unwrap_err(),
            AliasError::RoundTripMismatch
        );
    }

    #[test]
    fn an_alias_inside_a_comment_block_is_not_one() {
        // hledger swallows the block, so counting the line would shift every
        // index below it and land an edit on the wrong alias.
        let text = "comment\nalias hidden = nope\nend comment\nalias real = yes\n";
        let parsed = doc(text);
        let seen: Vec<&str> = parsed
            .lines()
            .iter()
            .map(|line| line.pattern.as_str())
            .collect();
        assert_eq!(seen, vec!["real"]);
        assert_eq!(parsed.lines()[0].index, 0);
        assert_eq!(parsed.lines()[0].line, 4);
        // And the engine's own parse agrees, which is the property that makes
        // the index safe to join on.
        let journal = parse_journal(text, "t.journal").unwrap();
        assert_eq!(journal.aliases.len(), 1);
        assert_eq!(journal.aliases[0].position.line, 4);
    }

    #[test]
    fn forwarding_renders_both_forms_and_names_every_refusal() {
        let journal = parse_journal(
            "alias PW Roth IRA - 3077 = assets:ms:roth\n\
             alias /^CC (.+)$/ = liabilities:\\1\n\
             alias  = nothing\n\
             alias scoped = assets:scoped\n\
             end aliases\n\
             alias after = assets:after\n",
            "t.journal",
        )
        .unwrap();
        let forwarded = forward(&journal);
        let arguments: Vec<Result<&str, AliasRefusal>> = forwarded
            .iter()
            .map(|entry| entry.argument.as_deref().map_err(|refusal| *refusal))
            .collect();
        assert_eq!(
            arguments,
            vec![
                // `end aliases` closes EVERY alias in force before it, not just
                // the one beside it.
                Err(AliasRefusal::Scoped),
                Err(AliasRefusal::Scoped),
                // Checked before the scope, because "you mapped nothing" is the
                // more specific answer than "and it stopped anyway".
                Err(AliasRefusal::Empty),
                Err(AliasRefusal::Scoped),
                Ok("after=assets:after"),
            ]
        );
        assert_eq!(super::arguments(&journal), vec!["after=assets:after"]);
    }

    #[test]
    fn forwarding_renders_the_regex_form_with_its_slashes() {
        let journal = parse_journal(
            "alias PW Roth IRA - 3077 = assets:ms:roth\n\
             alias /^CC (.+)$/ = liabilities:\\1\n",
            "t.journal",
        )
        .unwrap();
        assert_eq!(
            arguments(&journal),
            vec![
                "PW Roth IRA - 3077=assets:ms:roth",
                "/^CC (.+)$/=liabilities:\\1"
            ]
        );
    }
}
