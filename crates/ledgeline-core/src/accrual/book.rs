//! The recognition file (plan 45 D1): `~` rules the main journal does **not**
//! include, read beside it.
//!
//! - [`recognition_path`] says where it is: `recognition.journal` beside the
//!   main journal, or a hand-written `[books] recognition-file`, always inside
//!   the journal's own directory (a [`VaultPath`] cannot leave it).
//! - [`RecognitionBook`] is the parsed file: its rules, read with the main
//!   journal's commodity styles. It is **never** merged into
//!   [`crate::model::Journal::periodic_transactions`], which is what keeps
//!   schedules out of the budget and projections (D2). [`BookCache`] keeps the
//!   last parse, keyed by the file's bytes and the styles.
//! - [`RecognitionDoc`] edits it in the discipline of [`crate::periodic`]:
//!   an edit rewrites bytes only inside the group it names (append, replace or
//!   delete one schedule's rules), and [`RecognitionDoc::verify`] reparses the
//!   result and requires that group to read back as intended and every other
//!   rule to be unchanged. A group this module can't map back to one schedule
//!   (split in two, or holding a period this engine can't enumerate) is read
//!   and shown like any other, but never rewritten.

use super::schedule::Schedule;
use super::{RECOGNITION_TAG, tag_value};
use crate::display::commodity_directive;
use crate::edit::Fingerprint;
use crate::model::{AmountStyle, Commodity, Journal, PeriodKind, PeriodicTransaction};
use crate::parse::{ParseError, parse_journal_with_styles};
use crate::periodic::PeriodicDoc;
use crate::vault::VaultPath;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// The recognition file's default name, beside the main journal.
pub const RECOGNITION_FILE: &str = "recognition.journal";

/// The marker that starts a schedule's heading comment.
pub const HEADING_MARK: &str = "; ── ";

/// Why the recognition file can't be read, located or edited.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BookError {
    /// The configured path is unusable.
    #[error("the recognition file path `{path}` is unusable: {reason}")]
    Path {
        /// The path as configured.
        path: String,
        /// Why.
        reason: String,
    },
    /// The file doesn't parse.
    #[error("the recognition file does not parse: {0}")]
    Parse(String),
    /// The file's `~` blocks and its parsed rules don't line up, so no block
    /// can be mapped to a rule and nothing is rewritten.
    #[error("the recognition file can't be edited safely: {0}")]
    Unreadable(String),
    /// The schedule's rules can't be rewritten.
    #[error("schedule {id} can't be rewritten: {reason}")]
    Locked {
        /// The schedule id.
        id: String,
        /// Why.
        reason: &'static str,
    },
    /// No group holds this id.
    #[error("the recognition file holds no schedule {0}")]
    UnknownId(String),
    /// A group with this id already exists.
    #[error("the recognition file already holds schedule {0}")]
    DuplicateId(String),
    /// The edited text didn't read back as intended.
    #[error("the edit to the recognition file did not read back as intended: {0}")]
    Verify(String),
}

impl From<ParseError> for BookError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error.to_string())
    }
}

/// Where the recognition file is: `configured` (a `[books] recognition-file`
/// value, relative to the journal's directory) or [`RECOGNITION_FILE`],
/// beside `main`.
///
/// # Errors
/// [`BookError::Path`] when `configured` is absolute, climbs out of the
/// journal's directory, or names something that isn't a `.journal` file.
pub fn recognition_path(
    main: &VaultPath,
    configured: Option<&str>,
) -> Result<VaultPath, BookError> {
    let dir = main.parent().unwrap_or_default();
    let wanted = configured.map_or(RECOGNITION_FILE, str::trim);
    let refuse = |reason: &str| BookError::Path {
        path: wanted.to_string(),
        reason: reason.to_string(),
    };
    let path = dir
        .join(wanted)
        .map_err(|_| refuse("it must be a relative path inside the journal's folder"))?;
    if path.extension() != Some("journal") {
        return Err(refuse("it must name a `.journal` file"));
    }
    if &path == main {
        return Err(refuse("it is the main journal"));
    }
    Ok(path)
}

/// Why `path` (a recognition file's key) collides with the journal's own
/// files: the main journal, `budget.journal`, or any included file (D-guards
/// "Budget file collision"). `None` when it doesn't.
#[must_use]
pub fn collision(journal: &Journal, key: &Path) -> Option<&'static str> {
    if journal.source_files.first().is_some_and(|main| main == key) {
        return Some("it is the main journal");
    }
    if key.file_name().and_then(std::ffi::OsStr::to_str) == Some("budget.journal") {
        return Some("it is the budget file");
    }
    journal
        .source_files
        .iter()
        .any(|file| file == key)
        .then_some("the journal includes it")
}

/// Whether the journal includes the recognition file (the
/// `recognition-file-included` finding): then hledger and the Budget tab treat
/// every schedule as a budget goal, and Ledgeline does not load it twice.
#[must_use]
pub fn is_included(journal: &Journal, key: &Path) -> bool {
    journal.source_files.iter().skip(1).any(|file| file == key)
}

/// A new recognition file's text: what the file is, the command that reads
/// it, the warning not to include it, and the main journal's `decimal-mark`.
/// Commodity directives are added as schedules need them
/// ([`RecognitionDoc::append`]).
#[must_use]
pub fn new_file_text(main_name: &str, file_name: &str, journal: &Journal) -> String {
    let mark = journal
        .decimal_mark
        .map(|mark| format!("decimal-mark {mark}\n"))
        .unwrap_or_default();
    format!(
        "; Revenue and expense recognition schedules for {main_name}, written by Ledgeline.\n\
         ; Read it beside your books, never instead of them:\n\
         ;   hledger -f {main_name} -f {file_name} --forecast=tomorrow.. reg tag:recognition\n\
         ; Do not `include` this file: hledger and the Budget tab would read every\n\
         ; schedule as a budget goal. Posted entries live in {main_name} itself.\n\
         {mark}\n"
    )
}

/// The parsed recognition file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecognitionBook {
    /// The file's key ([`crate::vault::Vault::key`]): what its rules record as
    /// their `source_file`.
    pub key: PathBuf,
    /// The file's text.
    pub text: String,
    /// The fingerprint of [`Self::text`]: the revision a write must still find.
    pub revision: Fingerprint,
    /// Every `~` rule in the file, in file order.
    pub rules: Vec<PeriodicTransaction>,
}

impl RecognitionBook {
    /// Parse `text` (the file at `key`) with `inherited` styles in force, as
    /// though the main journal had declared them first (fact 8).
    ///
    /// # Errors
    /// [`BookError::Parse`] when it doesn't parse.
    pub fn parse(
        text: &str,
        key: &Path,
        inherited: &[(Commodity, AmountStyle)],
    ) -> Result<Self, BookError> {
        let parsed = parse_journal_with_styles(text, &key.to_string_lossy(), inherited)?;
        let own = parsed.source_files.first().cloned().unwrap_or_default();
        Ok(Self {
            key: key.to_path_buf(),
            text: text.to_string(),
            revision: Fingerprint::of_bytes(text.as_bytes()),
            rules: parsed
                .periodic_transactions
                .into_iter()
                .filter(|rule| rule.source_file == own)
                .collect(),
        })
    }

    /// The book of a file that doesn't exist yet.
    #[must_use]
    pub fn missing(key: &Path) -> Self {
        Self {
            key: key.to_path_buf(),
            text: String::new(),
            revision: Fingerprint::of_bytes(b""),
            rules: Vec::new(),
        }
    }

    /// The rules carrying `recognition: id`, in file order.
    #[must_use]
    pub fn rules_of(&self, id: &str) -> Vec<&PeriodicTransaction> {
        self.rules
            .iter()
            .filter(|rule| tag_value(&rule.tags, RECOGNITION_TAG) == Some(id))
            .collect()
    }

    /// The dates `id`'s rules fire on, sorted (unbounded rules skipped).
    #[must_use]
    pub fn occurrence_dates(&self, id: &str) -> Vec<String> {
        let mut days: Vec<i32> = self
            .rules_of(id)
            .into_iter()
            .filter_map(|rule| super::schedule::occurrence_days(&rule.period, None))
            .flatten()
            .collect();
        days.sort_unstable();
        days.dedup();
        days.into_iter().map(super::iso_of).collect()
    }

    /// Every schedule id, in order of first appearance.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        let mut seen = std::collections::BTreeSet::new();
        self.rules
            .iter()
            .filter_map(|rule| tag_value(&rule.tags, RECOGNITION_TAG))
            .filter(|id| seen.insert(*id))
            .collect()
    }
}

/// The last parse of a recognition file, reused while neither its bytes nor
/// the inherited styles change — so a main-journal edit doesn't re-read it.
#[derive(Debug, Default)]
pub struct BookCache {
    last: Mutex<Option<(CacheKey, Arc<RecognitionBook>)>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    key: PathBuf,
    text: Fingerprint,
    styles: Fingerprint,
}

impl BookCache {
    /// The book for `text` at `key` under `inherited`, parsed only when it
    /// differs from the last one asked for.
    ///
    /// # Errors
    /// As [`RecognitionBook::parse`].
    pub fn get(
        &self,
        text: &str,
        key: &Path,
        inherited: &[(Commodity, AmountStyle)],
    ) -> Result<Arc<RecognitionBook>, BookError> {
        let wanted = CacheKey {
            key: key.to_path_buf(),
            text: Fingerprint::of_bytes(text.as_bytes()),
            styles: Fingerprint::of_bytes(format!("{inherited:?}").as_bytes()),
        };
        // A poisoned lock only means another reader panicked mid-parse; the
        // cache is an optimization, so start again rather than propagate.
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached, book)) = last.as_ref()
            && *cached == wanted
            && book.text == text
        {
            return Ok(Arc::clone(book));
        }
        let book = Arc::new(RecognitionBook::parse(text, key, inherited)?);
        *last = Some((wanted, Arc::clone(&book)));
        Ok(book)
    }
}

// ---------------------------------------------------------------------------
// The block model
// ---------------------------------------------------------------------------

/// Why a group can be read but not rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupLock {
    /// The id's rules are not one contiguous run of blocks.
    Split,
    /// A rule's period is one this engine can't enumerate.
    UnsupportedPeriod,
}

impl GroupLock {
    /// A sentence for the user.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Split => "its rules are not together in the file; move them next to each other",
            Self::UnsupportedPeriod => "a rule has a period Ledgeline can't read",
        }
    }
}

/// One schedule's rules in the file: a contiguous run of `~` blocks sharing a
/// `recognition:` id, with the heading comment directly above them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The schedule id.
    pub id: String,
    /// Heading (if any) through the end of the last block.
    pub span: Range<usize>,
    /// `Some` when the group may not be rewritten.
    pub lock: Option<GroupLock>,
}

/// One change to the recognition file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupEdit {
    /// Add a new schedule's group at the end.
    Append {
        /// The schedule.
        id: String,
        /// The heading comment line and the rules, as [`group_text`] renders
        /// them.
        text: String,
        /// `commodity` directives the rules need, added before the group when
        /// the file doesn't already hold them verbatim.
        directives: Vec<String>,
    },
    /// Replace an existing group's heading and rules.
    Replace {
        /// The schedule.
        id: String,
        /// The new heading and rules.
        text: String,
        /// As for [`GroupEdit::Append`].
        directives: Vec<String>,
    },
    /// Remove a group.
    Delete {
        /// The schedule.
        id: String,
    },
}

impl GroupEdit {
    fn id(&self) -> &str {
        match self {
            Self::Append { id, .. } | Self::Replace { id, .. } | Self::Delete { id } => id,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Self::Append { text, .. } | Self::Replace { text, .. } => Some(text),
            Self::Delete { .. } => None,
        }
    }
}

/// A schedule's heading comment: `; ── Customer X · k7f3a2 · 2026-11-17
/// Customer X Payment`.
#[must_use]
pub fn heading(
    party: Option<&str>,
    id: &str,
    source_date: &str,
    source_description: &str,
) -> String {
    let flat = |text: &str| text.replace(['\n', '\r'], " ");
    let parts: Vec<String> = party
        .map(flat)
        .into_iter()
        .chain([
            id.to_string(),
            flat(&format!("{source_date} {source_description}"))
                .trim()
                .to_string(),
        ])
        .collect();
    format!("{HEADING_MARK}{}\n", parts.join(" · "))
}

/// A schedule's group: its heading line and its rules.
#[must_use]
pub fn group_text(heading: &str, schedule: &Schedule) -> String {
    format!("{heading}{}", schedule.rules_text())
}

/// The `commodity` directives `schedule`'s lines need, in the main journal's
/// styles.
#[must_use]
pub fn directives_for(schedule: &Schedule, journal: &Journal) -> Vec<String> {
    let styles = crate::parse::JournalStyles::of(journal);
    let mut out: Vec<String> = schedule
        .spec
        .lines
        .iter()
        .filter(|line| !line.total.commodity.0.is_empty())
        .map(|line| {
            let style = styles
                .declared_style(&line.total.commodity)
                .unwrap_or(&line.total.style);
            commodity_directive(&line.total.commodity.0, style)
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The recognition file's groups, over its text.
#[derive(Debug, Clone)]
pub struct RecognitionDoc {
    text: String,
    groups: Vec<Group>,
}

impl RecognitionDoc {
    /// Map `book`'s rules onto its text's `~` blocks.
    ///
    /// # Errors
    /// [`BookError::Unreadable`] when the blocks and the parsed rules don't
    /// line up one to one.
    pub fn new(book: &RecognitionBook) -> Result<Self, BookError> {
        let doc = PeriodicDoc::parse(&book.text);
        let blocks = doc.blocks();
        if blocks.len() != book.rules.len() {
            return Err(BookError::Unreadable(format!(
                "{} `~` blocks but {} rules",
                blocks.len(),
                book.rules.len()
            )));
        }
        let text = book.text.as_str();
        let mut groups: Vec<Group> = Vec::new();
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for (block, rule) in blocks.iter().zip(&book.rules) {
            let Some(id) = tag_value(&rule.tags, RECOGNITION_TAG) else {
                continue;
            };
            let unsupported = rule.period.kind == PeriodKind::Unsupported;
            match groups.last_mut() {
                Some(group)
                    if group.id == id
                        && group.span.end <= block.full.start
                        && text[group.span.end..block.full.start].trim().is_empty() =>
                {
                    group.span.end = block.full.end;
                    if unsupported {
                        group.lock = Some(GroupLock::UnsupportedPeriod);
                    }
                }
                _ => {
                    if let Some(&at) = seen.get(id) {
                        groups[at].lock = Some(GroupLock::Split);
                    }
                    seen.insert(id.to_string(), groups.len());
                    groups.push(Group {
                        id: id.to_string(),
                        span: heading_start(text, block.full.start)..block.full.end,
                        lock: unsupported.then_some(GroupLock::UnsupportedPeriod),
                    });
                }
            }
        }
        // A split id locks every run of it.
        let split: Vec<String> = groups
            .iter()
            .filter(|g| g.lock == Some(GroupLock::Split))
            .map(|g| g.id.clone())
            .collect();
        for group in &mut groups {
            if split.contains(&group.id) {
                group.lock = Some(GroupLock::Split);
            }
        }
        Ok(Self {
            text: book.text.clone(),
            groups,
        })
    }

    /// The groups, in file order.
    #[must_use]
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    fn group(&self, id: &str) -> Result<&Group, BookError> {
        let group = self
            .groups
            .iter()
            .find(|group| group.id == id)
            .ok_or_else(|| BookError::UnknownId(id.to_string()))?;
        match group.lock {
            Some(lock) => Err(BookError::Locked {
                id: id.to_string(),
                reason: lock.message(),
            }),
            None => Ok(group),
        }
    }

    /// The directives in `directives` the file doesn't already hold verbatim.
    fn missing_directives(&self, directives: &[String]) -> String {
        directives
            .iter()
            .filter(|line| {
                !self
                    .text
                    .lines()
                    .any(|have| have.trim_end() == line.as_str())
            })
            .map(|line| format!("{line}\n"))
            .collect()
    }

    /// The new text with `edit` applied. Bytes outside the edited group are
    /// copied verbatim.
    ///
    /// # Errors
    /// [`BookError::UnknownId`], [`BookError::DuplicateId`] or
    /// [`BookError::Locked`].
    pub fn apply(&self, edit: &GroupEdit) -> Result<String, BookError> {
        let text = self.text.as_str();
        match edit {
            GroupEdit::Append {
                id,
                text: group,
                directives,
            } => {
                if self.groups.iter().any(|g| &g.id == id) {
                    return Err(BookError::DuplicateId(id.clone()));
                }
                let lead = match text {
                    "" => "",
                    t if t.ends_with("\n\n") => "",
                    t if t.ends_with('\n') => "\n",
                    _ => "\n\n",
                };
                Ok(format!(
                    "{text}{lead}{}{group}",
                    self.missing_directives(directives)
                ))
            }
            GroupEdit::Replace {
                id,
                text: group,
                directives,
            } => {
                let span = self.group(id)?.span.clone();
                Ok(format!(
                    "{}{}{group}{}",
                    &text[..span.start],
                    self.missing_directives(directives),
                    &text[span.end..]
                ))
            }
            GroupEdit::Delete { id } => {
                let span = self.group(id)?.span.clone();
                // One blank line after the group goes with it.
                let rest = &text[span.end..];
                let skip = if rest.starts_with("\r\n") {
                    2
                } else {
                    usize::from(rest.starts_with('\n'))
                };
                Ok(format!("{}{}", &text[..span.start], &rest[skip..]))
            }
        }
    }

    /// Check that `new_text` is `edit` applied and reads back as intended:
    /// the edited group's rules are exactly the rules its text parses to on
    /// its own, every other rule is unchanged, and nothing else moved.
    /// Returns the new book.
    ///
    /// # Errors
    /// [`BookError::Verify`] when it doesn't; [`BookError::Parse`] when the
    /// new text doesn't parse.
    pub fn verify(
        &self,
        old: &RecognitionBook,
        edit: &GroupEdit,
        new_text: &str,
        inherited: &[(Commodity, AmountStyle)],
    ) -> Result<RecognitionBook, BookError> {
        if self.apply(edit)? != new_text {
            return Err(BookError::Verify("the text is not the planned edit".into()));
        }
        let new = RecognitionBook::parse(new_text, &old.key, inherited)?;
        let id = edit.id();
        let others = |book: &RecognitionBook| -> Vec<RuleShape> {
            book.rules
                .iter()
                .filter(|rule| tag_value(&rule.tags, RECOGNITION_TAG) != Some(id))
                .map(RuleShape::of)
                .collect()
        };
        if others(old) != others(&new) {
            return Err(BookError::Verify("another schedule's rules changed".into()));
        }
        let intended: Vec<RuleShape> = match edit.text() {
            Some(group) => RecognitionBook::parse(group, &old.key, inherited)?
                .rules
                .iter()
                .map(RuleShape::of)
                .collect(),
            None => Vec::new(),
        };
        let got: Vec<RuleShape> = new.rules_of(id).into_iter().map(RuleShape::of).collect();
        if got != intended {
            return Err(BookError::Verify(format!(
                "schedule {id} reads back as {} rules, not the {} intended",
                got.len(),
                intended.len()
            )));
        }
        Ok(new)
    }
}

/// The start of the heading comment directly above a block at `start`, or
/// `start` itself when there is none.
fn heading_start(text: &str, start: usize) -> usize {
    let before = &text[..start];
    let line_start = before
        .trim_end_matches(['\n', '\r'])
        .rfind('\n')
        .map_or(0, |at| at + 1);
    let previous = &before[line_start..];
    // Only a heading immediately above (no blank line between) belongs to it.
    let directly_above = before.ends_with('\n') && !before[..before.len() - 1].ends_with('\n')
        || before.ends_with("\r\n") && !before[..before.len() - 2].ends_with('\n');
    if directly_above && previous.starts_with(HEADING_MARK) {
        line_start
    } else {
        start
    }
}

/// One posting's account, amounts and tags.
type PostingShape = (
    String,
    Vec<(String, crate::decimal::Dec)>,
    Vec<(String, String)>,
);

/// What a rule means, without where it is.
#[derive(Debug, PartialEq, Eq)]
struct RuleShape {
    period: String,
    description: String,
    tags: Vec<(String, String)>,
    postings: Vec<PostingShape>,
}

impl RuleShape {
    fn of(rule: &PeriodicTransaction) -> Self {
        Self {
            period: rule.period.raw.clone(),
            description: rule.description.clone(),
            tags: rule.tags.clone(),
            postings: rule
                .postings
                .iter()
                .map(|posting| {
                    (
                        posting.account.0.clone(),
                        posting
                            .amounts
                            .iter()
                            .map(|amount| {
                                (amount.commodity.0.clone(), amount.quantity.normalized())
                            })
                            .collect(),
                        posting.tags.clone(),
                    )
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "/nonexistent/recognition.journal";

    fn book(text: &str) -> RecognitionBook {
        RecognitionBook::parse(text, Path::new(KEY), &[]).expect("parses")
    }

    const TWO: &str = "; header\n\n\
; ── A · aaa111 · 2026-01-01 A\n\
~ monthly from 2026-01-01 to 2026-03-01  A  ; recognition: aaa111\n    l:d  $1\n    r:s  $-1\n\
~ 2026-03-01  A  ; recognition: aaa111\n    l:d  $2\n    r:s  $-2\n\
\n\
; ── B · bbb222 · 2026-02-01 B\n\
~ monthly from 2026-02-01 to 2026-04-01  B  ; recognition: bbb222\n    l:d  $5\n    r:s  $-5\n";

    #[test]
    fn groups_take_their_heading_and_contiguous_blocks() {
        let b = book(TWO);
        let doc = RecognitionDoc::new(&b).expect("maps");
        let ids: Vec<&str> = doc.groups().iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["aaa111", "bbb222"]);
        assert!(TWO[doc.groups()[0].span.clone()].starts_with("; ── A"));
        assert!(TWO[doc.groups()[1].span.clone()].starts_with("; ── B"));
        assert_eq!(b.ids(), vec!["aaa111", "bbb222"]);
    }

    #[test]
    fn replace_and_delete_touch_only_their_group_and_verify() {
        let b = book(TWO);
        let doc = RecognitionDoc::new(&b).expect("maps");
        let edit = GroupEdit::Replace {
            id: "aaa111".into(),
            text: "; ── A · aaa111 · x\n~ 2026-01-01  A  ; recognition: aaa111\n    l:d  $3\n    r:s  $-3\n".into(),
            directives: vec![],
        };
        let new = doc.apply(&edit).expect("applies");
        assert!(new.starts_with("; header\n\n; ── A · aaa111 · x\n~ 2026-01-01"));
        assert!(new.ends_with(&TWO[TWO.find("\n; ── B").expect("B")..]));
        let after = doc.verify(&b, &edit, &new, &[]).expect("verifies");
        assert_eq!(after.rules_of("aaa111").len(), 1);

        let delete = GroupEdit::Delete {
            id: "aaa111".into(),
        };
        let new = doc.apply(&delete).expect("applies");
        assert_eq!(
            new,
            format!("; header\n\n{}", &TWO[TWO.find("; ── B").expect("B")..])
        );
        doc.verify(&b, &delete, &new, &[]).expect("verifies");
        // A forged text is refused.
        assert!(doc.verify(&b, &delete, TWO, &[]).is_err());
    }

    #[test]
    fn append_adds_missing_directives_once_and_refuses_a_duplicate() {
        let b = book("; header\n");
        let doc = RecognitionDoc::new(&b).expect("maps");
        let edit = GroupEdit::Append {
            id: "ccc333".into(),
            text: "; ── C · ccc333 · x\n~ 2026-01-01  C  ; recognition: ccc333\n    l:d  $3\n    r:s  $-3\n".into(),
            directives: vec!["commodity $1,000.00".into()],
        };
        let new = doc.apply(&edit).expect("applies");
        assert_eq!(
            new,
            "; header\n\ncommodity $1,000.00\n; ── C · ccc333 · x\n~ 2026-01-01  C  ; recognition: ccc333\n    l:d  $3\n    r:s  $-3\n"
        );
        let after = doc.verify(&b, &edit, &new, &[]).expect("verifies");
        let again = RecognitionDoc::new(&after).expect("maps");
        assert!(matches!(again.apply(&edit), Err(BookError::DuplicateId(_))));
    }

    #[test]
    fn a_split_id_is_locked() {
        let text = "~ 2026-01-01  A  ; recognition: aaa111\n    l:d  $1\n    r:s  $-1\n\n\
~ 2026-01-01  B  ; recognition: bbb222\n    l:d  $1\n    r:s  $-1\n\n\
~ 2026-02-01  A  ; recognition: aaa111\n    l:d  $1\n    r:s  $-1\n";
        let doc = RecognitionDoc::new(&book(text)).expect("maps");
        assert!(matches!(
            doc.apply(&GroupEdit::Delete {
                id: "aaa111".into()
            }),
            Err(BookError::Locked { .. })
        ));
        assert!(
            doc.apply(&GroupEdit::Delete {
                id: "bbb222".into()
            })
            .is_ok()
        );
    }

    #[test]
    fn paths_stay_inside_and_never_name_the_journal() {
        let main = VaultPath::new("books/main.journal").expect("path");
        assert_eq!(
            recognition_path(&main, None).expect("default").as_str(),
            "books/recognition.journal"
        );
        assert_eq!(
            recognition_path(&main, Some("sched/r.journal"))
                .expect("configured")
                .as_str(),
            "books/sched/r.journal"
        );
        for bad in ["../../x.journal", "/etc/x.journal", "main.journal", "r.txt"] {
            assert!(recognition_path(&main, Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_cache_reparses_only_on_change() {
        let cache = BookCache::default();
        let a = cache.get(TWO, Path::new(KEY), &[]).expect("parses");
        let b = cache.get(TWO, Path::new(KEY), &[]).expect("parses");
        assert!(Arc::ptr_eq(&a, &b));
        let c = cache.get("; other\n", Path::new(KEY), &[]).expect("parses");
        assert!(!Arc::ptr_eq(&a, &c));
    }
}
