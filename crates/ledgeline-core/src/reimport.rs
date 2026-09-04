//! Matching a re-downloaded statement against the journal it was already
//! imported into, by the stable id the bank gives each row.
//!
//! # The problem this exists for
//!
//! `hledger import` de-duplicates by **date**: it records the newest imported
//! date in `.latest.NAME` and proposes only rows after it. That is exactly right
//! for an append-only download and exactly wrong for the YTD-redownload
//! workflow, where a row is downloaded *twice* — once while the charge is still
//! an authorization hold, and again once it settles. The second copy sits before
//! `.latest`, is never proposed, and the journal silently keeps the pending
//! version forever. `TODO.md`'s "Import improvements" section names it, and adds
//! the sharper half: the dry-run's "N rows skipped" count "cannot distinguish
//! *already imported identically* from *already imported differently*".
//!
//! Distinguishing them is what this module does.
//!
//! # The id is the bank's, and hledger already carries it
//!
//! OFX/QFX/QBO give every transaction a `FITID`, which
//! [`convert::ofx`](crate::convert::ofx) already emits as the `fitid` column. A
//! rules file marks it as the dedup id with one line of ordinary hledger
//! grammar:
//!
//! ```text
//! comment id:%fitid
//! ```
//!
//! `comment` is a top-level assignable field, so hledger writes that text as the
//! transaction's own comment and re-reads it as the tag `id`. Verified end to
//! end against hledger 1.52 (`ttags: [["id","FIT0001"]]` in `print -O json`) and
//! then through this crate's own parser, which lands it in
//! [`Transaction::tags`](crate::model::Transaction::tags). **Nothing new was
//! added to the rules grammar for this**, and nothing here re-derives what a
//! rules file would produce: every proposal compared below is hledger's own
//! output, re-read by [`parse_journal`](crate::parse_journal).
//!
//! # What a match may and may not do
//!
//! An id match is authoritative in one direction only. It may **subtract** —
//! keep a row out of the import, because the journal demonstrably already holds
//! it — and it may flip a clearing status. It may never **add**: a row hledger's
//! own `.latest` bookkeeping declined to propose is still not proposed, and a
//! transaction whose fields disagree is reported and left exactly as the user
//! wrote it.
//!
//! That asymmetry is not timidity, it is the only safe reading. Journals hold
//! transactions imported *before* the rules file grew its `comment id:` line,
//! and those carry no id at all. If a missing id were ever read as "this row is
//! new", the first re-download after adding that line would duplicate every
//! untagged transaction in the file. So a missing id means only "this module has
//! nothing to say", and today's behaviour stands.

use crate::edit::render_amount;
use crate::model::{Journal, Status, Tindex, Transaction};
use std::borrow::Cow;
use std::collections::HashMap;

/// The comment tag a rules file writes to name a row's stable identity.
///
/// `id` rather than `ledgeline-id` or `fitid`: it is what a rules-file author
/// writes without being told (`comment id:%fitid`), it is the spelling the WP-16
/// plan named, and it keeps the rules file readable by anyone who runs hledger
/// from a terminal — nothing here is a Ledgeline-private convention.
///
/// The cost of so plain a name is that a journal could already use `id:` for
/// something of its own. That is survivable by construction: a collision has to
/// be with a bank's own opaque `FITID` string, and its worst outcome is a
/// [`RowClassification::Conflicting`] the user can see, never a silent
/// substitution — every path that *writes* requires every other field to match
/// as well.
pub const ID_TAG: &str = "id";

/// The id `txn` carries under `id_tag`, or `None`.
///
/// An **empty** value is `None`, deliberately. A rules file writing
/// `comment id:%fitid` over a statement whose `fitid` column is blank emits a
/// bare `; id:` on every such row, and treating those as a shared id would match
/// unrelated transactions to each other.
#[must_use]
pub fn id_of<'a>(txn: &'a Transaction, id_tag: &str) -> Option<&'a str> {
    txn.tags
        .iter()
        .find(|(name, _)| name == id_tag)
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
}

/// The transactions a journal already holds, keyed by their id tag.
///
/// Built over the **whole** journal tree rather than over the import's target
/// file: a row imported into last year's file is still a row this statement must
/// not import again.
#[derive(Debug, Default)]
pub struct IdIndex<'a> {
    /// First transaction in file order per id, and how many carry that id.
    by_id: HashMap<&'a str, (&'a Transaction, usize)>,
}

impl<'a> IdIndex<'a> {
    /// The first transaction in file order carrying `id`, and how many do.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<(&'a Transaction, usize)> {
        self.by_id.get(id).copied()
    }

    /// How many distinct ids the journal carries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the journal carries no id at all — the state a journal is in
    /// before its rules file ever wrote one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Index every transaction in `journal` that carries `id_tag`.
///
/// A duplicated id keeps the **first** in file order and records the count, so
/// [`classify`] can refuse to act on an id it cannot resolve to one transaction
/// rather than picking one arbitrarily.
#[must_use]
pub fn build_index<'a>(journal: &'a Journal, id_tag: &str) -> IdIndex<'a> {
    let mut by_id: HashMap<&'a str, (&'a Transaction, usize)> = HashMap::new();
    for txn in &journal.transactions {
        if let Some(id) = id_of(txn, id_tag) {
            by_id
                .entry(id)
                .and_modify(|(_, count)| *count += 1)
                .or_insert((txn, 1));
        }
    }
    IdIndex { by_id }
}

/// One field on which a re-downloaded row and the transaction already carrying
/// its id disagree.
///
/// Both sides are rendered rather than typed, because this is shown to a person
/// deciding whether their own edit was the right one. `field` is a phrase, not
/// an enum: postings are named positionally (`posting 2 amount`) and a closed
/// set could not spell that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDiff {
    /// What disagrees, e.g. `date`, `description`, `posting 1 amount`.
    pub field: String,
    /// What the journal says today.
    pub existing: String,
    /// What this statement proposes.
    pub incoming: String,
}

impl FieldDiff {
    fn new(
        field: impl Into<String>,
        existing: impl Into<String>,
        incoming: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            existing: existing.into(),
            incoming: incoming.into(),
        }
    }
}

/// What one proposed row turns out to be, given what the journal already holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowClassification {
    /// No transaction in the journal carries this id. Ordinary import.
    New,
    /// A transaction carries this id and agrees on every compared field. Not
    /// imported, not edited, counted.
    Unchanged,
    /// A transaction carries this id and differs in its clearing status **and
    /// nothing else** — the pending charge that settled. Safe to sync.
    StatusOnly {
        /// The existing transaction's index.
        index: Tindex,
        /// What the journal says today.
        existing_status: Status,
        /// What this statement says.
        new_status: Status,
    },
    /// A transaction carries this id and differs in something a status flip
    /// cannot express. Never imported, **never edited** — the user very probably
    /// meant it.
    Conflicting {
        /// The existing transaction's index.
        index: Tindex,
        /// Every disagreement, in field order.
        diffs: Vec<FieldDiff>,
    },
}

/// Whether the rules file behind `proposed` assigns hledger's `status` field.
///
/// Answered from hledger's own output rather than by reading the rules file:
/// `import` never marks a transaction by itself (verified against 1.52 — a rules
/// file with no `status` field proposes `2026-01-05 COFFEE SHOP`, unmarked), so
/// any marker in a proposal was put there by an assignment the author wrote.
///
/// It gates [`RowClassification::StatusOnly`] because **without it a status
/// difference is not a status difference at all**. If the rules file assigns no
/// status, every proposed row is [`Status::Unmarked`]; a journal transaction the
/// user marked `*` by hand would then differ "only in status", and syncing it
/// would rub out their own mark. So a file that never assigns a status can
/// produce no status-only rows, and every difference it does produce is a
/// conflict.
#[must_use]
pub fn maps_status(proposed: &[Transaction]) -> bool {
    proposed.iter().any(|txn| txn.status != Status::Unmarked)
}

/// Classify one proposed row against the journal.
///
/// Pure. `proposed` is one transaction of **hledger's own dry-run proposal**,
/// re-read by this crate's parser, so nothing here re-evaluates a rules file.
/// `status_mapped` is [`maps_status`] over the whole proposal — see there for
/// why it cannot be decided per row.
#[must_use]
pub fn classify(
    index: &IdIndex<'_>,
    id: &str,
    proposed: &Transaction,
    status_mapped: bool,
) -> RowClassification {
    let Some((existing, carriers)) = index.get(id) else {
        return RowClassification::New;
    };
    if carriers > 1 {
        // Two transactions already claim this id, so "the one to compare
        // against" has no answer. Report it; never guess which to edit.
        return RowClassification::Conflicting {
            index: existing.index,
            diffs: vec![FieldDiff::new(
                "id",
                format!("{carriers} transactions in the journal carry this id"),
                "one row in this statement".to_string(),
            )],
        };
    }

    let diffs = diff(existing, proposed);
    match diffs.as_slice() {
        [] => RowClassification::Unchanged,
        [only] if only.field == "status" && status_mapped => RowClassification::StatusOnly {
            index: existing.index,
            existing_status: existing.status,
            new_status: proposed.status,
        },
        _ => RowClassification::Conflicting {
            index: existing.index,
            diffs,
        },
    }
}

/// One classified row of a proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedRow {
    /// The row's id, as the rules file produced it.
    pub id: String,
    /// What it turned out to be.
    pub classification: RowClassification,
}

/// Classify a whole proposal against the journal's ids, or answer `None` when
/// this statement's rules file declares no id at all.
///
/// `None` is the "behave exactly as before" answer, and it is what puts
/// `idMatches: null` on the wire. It is deliberately observational — no proposed
/// row carries the tag — rather than a reading of the rules file, so a rules file
/// that *declares* `comment id:%fitid` over a column that turns out to be empty
/// gets the same silence as one that declares nothing.
///
/// Rows with no id of their own are still returned, as
/// [`RowClassification::New`]: a statement may mix rows the bank identified with
/// rows it did not, and the ones it did not are exactly today's behaviour.
///
/// It takes the index rather than the journal so that one build serves this and
/// [`retain_new`], which are asked about **different** proposals of the same
/// import — see the caller in `import_api`.
#[must_use]
pub fn reconcile(
    index: &IdIndex<'_>,
    proposed: &[Transaction],
    id_tag: &str,
) -> Option<Vec<ClassifiedRow>> {
    if !proposed.iter().any(|txn| id_of(txn, id_tag).is_some()) {
        return None;
    }
    let status_mapped = maps_status(proposed);
    Some(
        proposed
            .iter()
            .map(|txn| match id_of(txn, id_tag) {
                Some(id) => ClassifiedRow {
                    id: id.to_string(),
                    classification: classify(index, id, txn, status_mapped),
                },
                None => ClassifiedRow {
                    id: String::new(),
                    classification: RowClassification::New,
                },
            })
            .collect(),
    )
}

/// `entries` with every transaction whose id the journal already holds removed.
///
/// `proposed` must be `entries` as parsed by
/// [`parse_journal`](crate::parse_journal) — the caller already has it, and
/// re-parsing here could only produce a second answer.
///
/// # Why this is a line-span deletion and not a re-render
///
/// `docs/imports.md` § "hledger proposes; Ledgeline appends" rests on the
/// preview being *the bytes*: what the dry-run showed is what the commit
/// appends, character for character, with nothing re-rendering it in between.
/// So this deletes whole lines out of hledger's own text and touches nothing
/// else, exactly as `rules.rs` splices spans rather than pretty-printing an AST.
/// When nothing is dropped the input is returned **borrowed and unmodified**, so
/// an import with no id matches cannot differ from today's by so much as a byte.
#[must_use]
pub fn retain_new<'t>(
    entries: &'t str,
    proposed: &[Transaction],
    index: &IdIndex<'_>,
    id_tag: &str,
) -> Cow<'t, str> {
    let dropped: Vec<&Transaction> = proposed
        .iter()
        .filter(|txn| id_of(txn, id_tag).is_some_and(|id| index.get(id).is_some()))
        .collect();
    if dropped.is_empty() {
        return Cow::Borrowed(entries);
    }

    // Lines with their terminators kept, so a CRLF proposal or a missing final
    // newline survives the ones that are kept.
    let lines: Vec<&str> = split_inclusive_lines(entries);
    let mut remove = vec![false; lines.len()];
    for txn in dropped {
        // `source_span` is [first line, line AFTER the last posting], both
        // 1-based — verified against a real proposal, where a three-line entry
        // starting at line 1 spans 1..4 and line 4 is the blank separator.
        let start = txn.source_span.0.line.saturating_sub(1) as usize;
        let end = txn.source_span.1.line.saturating_sub(1) as usize;
        for slot in remove.iter_mut().take(end.min(lines.len())).skip(start) {
            *slot = true;
        }
        // …and the blank separator hledger writes after it, or the entry below
        // would be welded onto the one above it.
        for idx in end..lines.len() {
            if !lines[idx].trim().is_empty() {
                break;
            }
            remove[idx] = true;
        }
    }
    Cow::Owned(
        lines
            .iter()
            .zip(&remove)
            .filter_map(|(line, drop)| (!drop).then_some(*line))
            .collect(),
    )
}

/// `text` split into lines with their terminators retained.
fn split_inclusive_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split_inclusive('\n').collect()
    }
}

/// Every field on which `existing` and `proposed` disagree, in field order.
///
/// # What is compared, and what is deliberately not
///
/// Compared: the transaction's date, secondary date, code, description, status,
/// and every posting's account, amount and balance assertion. Those are what a
/// rules file *writes*, so they are what a re-download can legitimately change.
///
/// **Not compared: the comment, and therefore the tags.** The comment is where a
/// person annotates a transaction they have already imported (`; id:FIT0001,
/// reimbursed by work`), and reading a note as a conflict would report one for
/// every transaction anybody had ever written on. The id itself is not compared
/// for the same reason it is not in the diff: it is the premise of the
/// comparison, not a term in it.
fn diff(existing: &Transaction, proposed: &Transaction) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();
    let mut compare = |field: &str, left: &str, right: &str| {
        if left != right {
            diffs.push(FieldDiff::new(field, left, right));
        }
    };
    compare("date", &existing.date, &proposed.date);
    compare(
        "secondary date",
        existing.date2.as_deref().unwrap_or_default(),
        proposed.date2.as_deref().unwrap_or_default(),
    );
    compare("code", &existing.code, &proposed.code);
    compare("description", &existing.description, &proposed.description);
    compare(
        "status",
        status_word(existing.status),
        status_word(proposed.status),
    );

    if existing.postings.len() != proposed.postings.len() {
        diffs.push(FieldDiff::new(
            "postings",
            existing.postings.len().to_string(),
            proposed.postings.len().to_string(),
        ));
        return diffs;
    }
    for (nth, (was, now)) in existing.postings.iter().zip(&proposed.postings).enumerate() {
        let n = nth + 1;
        compare(
            &format!("posting {n} account"),
            &was.account.0,
            &now.account.0,
        );
        compare(&format!("posting {n} amount"), &amounts(was), &amounts(now));
        compare(
            &format!("posting {n} assertion"),
            &assertion(was),
            &assertion(now),
        );
    }
    diffs
}

/// One posting's amounts as a journal line writes them.
fn amounts(posting: &crate::model::Posting) -> String {
    posting
        .amounts
        .iter()
        .map(render_amount)
        .collect::<Vec<_>>()
        .join(", ")
}

/// One posting's balance assertion as a journal line writes it, or the empty
/// string when it has none.
fn assertion(posting: &crate::model::Posting) -> String {
    posting
        .balance_assertion
        .as_ref()
        .map_or_else(String::new, |assertion| {
            let equals = if assertion.total { "==" } else { "=" };
            let star = if assertion.inclusive { "*" } else { "" };
            format!("{equals}{star} {}", render_amount(&assertion.amount))
        })
}

/// A status as the wire and the diff spell it.
#[must_use]
pub fn status_word(status: Status) -> &'static str {
    match status {
        Status::Unmarked => "unmarked",
        Status::Pending => "pending",
        Status::Cleared => "cleared",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    /// A journal holding one pending and one cleared imported row, each tagged.
    const IMPORTED: &str = "\
2026-01-05 ! COFFEE SHOP  ; id:FIT0001
    assets:bank:checking           -4.50
    expenses:unknown                4.50

2026-01-06 * GROCERY MART  ; id:FIT0002
    assets:bank:checking          -32.10
    expenses:unknown               32.10
";

    fn journal(text: &str) -> Journal {
        parse_journal(text, "journal").expect("the fixture parses")
    }

    fn proposal(text: &str) -> Vec<Transaction> {
        journal(text).transactions
    }

    /// The classification of `text`'s single transaction against `IMPORTED`.
    fn classified(text: &str) -> RowClassification {
        let holding = journal(IMPORTED);
        let index = build_index(&holding, ID_TAG);
        let proposed = proposal(text);
        let id = id_of(&proposed[0], ID_TAG).expect("the fixture carries an id");
        classify(&index, id, &proposed[0], maps_status(&proposed))
    }

    #[test]
    fn an_id_is_read_off_a_transactions_comment() {
        let holding = journal(IMPORTED);
        let index = build_index(&holding, ID_TAG);
        assert_eq!(index.len(), 2);
        assert_eq!(
            index.get("FIT0001").map(|(txn, _)| txn.description.clone()),
            Some("COFFEE SHOP".to_string())
        );
        assert_eq!(index.get("NOPE"), None);
    }

    /// A blank `fitid` column emits `; id:` on every row. Matching those to each
    /// other would fuse unrelated transactions.
    #[test]
    fn an_empty_id_is_no_id() {
        let holding = journal("2026-01-05 A  ; id:\n    a  1\n    b  -1\n");
        assert!(build_index(&holding, ID_TAG).is_empty());
        assert_eq!(id_of(&holding.transactions[0], ID_TAG), None);
    }

    #[test]
    fn an_unknown_id_is_new() {
        assert_eq!(
            classified(
                "2026-01-08 * ACME PAYROLL  ; id:FIT0003\n    a  1500.00\n    b  -1500.00\n"
            ),
            RowClassification::New
        );
    }

    /// The no-op case: the same row, re-downloaded, status and all.
    #[test]
    fn an_identical_row_is_unchanged() {
        assert_eq!(
            classified(
                "2026-01-06 * GROCERY MART  ; id:FIT0002\n    assets:bank:checking          -32.10\n    expenses:unknown               32.10\n"
            ),
            RowClassification::Unchanged
        );
    }

    /// A hold that settled: `!` became `*` and nothing else moved.
    #[test]
    fn a_settled_hold_is_status_only() {
        assert_eq!(
            classified(
                "2026-01-05 * COFFEE SHOP  ; id:FIT0001\n    assets:bank:checking           -4.50\n    expenses:unknown                4.50\n"
            ),
            RowClassification::StatusOnly {
                index: Tindex(1),
                existing_status: Status::Pending,
                new_status: Status::Cleared,
            }
        );
    }

    /// **Status-only means ONLY status.** A hold that settled for a different
    /// amount — the tip added at the till — is a conflict, not a sync.
    #[test]
    fn a_status_change_with_anything_else_is_conflicting() {
        let RowClassification::Conflicting { index, diffs } = classified(
            "2026-01-05 * COFFEE SHOP  ; id:FIT0001\n    assets:bank:checking           -5.40\n    expenses:unknown                5.40\n",
        ) else {
            panic!("a second difference must defeat StatusOnly");
        };
        assert_eq!(index, Tindex(1));
        let fields: Vec<&str> = diffs.iter().map(|d| d.field.as_str()).collect();
        assert_eq!(
            fields,
            vec!["status", "posting 1 amount", "posting 2 amount"]
        );
        assert_eq!(diffs[1].existing, "-4.50");
        assert_eq!(diffs[1].incoming, "-5.40");
    }

    /// The hand-edit the whole feature exists to protect.
    #[test]
    fn a_hand_edited_amount_is_conflicting() {
        let RowClassification::Conflicting { diffs, .. } = classified(
            "2026-01-06 * GROCERY MART  ; id:FIT0002\n    assets:bank:checking          -35.60\n    expenses:unknown               35.60\n",
        ) else {
            panic!("an amount change is a conflict");
        };
        assert_eq!(
            diffs.iter().map(|d| d.field.as_str()).collect::<Vec<_>>(),
            vec!["posting 1 amount", "posting 2 amount"]
        );
    }

    /// A rules file that assigns no `status` can produce no status-only row —
    /// otherwise a `*` the user typed by hand would be rubbed out.
    #[test]
    fn without_a_status_assignment_a_status_difference_is_a_conflict() {
        let holding = journal(IMPORTED);
        let index = build_index(&holding, ID_TAG);
        // Unmarked everywhere: this is what a rules file with no `status` field
        // proposes.
        let proposed = proposal(
            "2026-01-05 COFFEE SHOP  ; id:FIT0001\n    assets:bank:checking           -4.50\n    expenses:unknown                4.50\n",
        );
        assert!(!maps_status(&proposed));
        assert!(matches!(
            classify(&index, "FIT0001", &proposed[0], false),
            RowClassification::Conflicting { .. }
        ));
        // …and the same row IS status-only once some row in the proposal shows
        // the rules file does assign one.
        assert!(matches!(
            classify(&index, "FIT0001", &proposed[0], true),
            RowClassification::StatusOnly { .. }
        ));
    }

    /// Two transactions claiming one id: no answer to "which one", so no edit.
    #[test]
    fn a_duplicated_id_is_conflicting_rather_than_guessed_at() {
        let holding = journal(
            "2026-01-05 ! A  ; id:FIT0001\n    a  -1\n    b  1\n\n2026-01-09 ! B  ; id:FIT0001\n    a  -1\n    b  1\n",
        );
        let index = build_index(&holding, ID_TAG);
        assert_eq!(index.get("FIT0001").map(|(_, n)| n), Some(2));
        let proposed = proposal("2026-01-05 * A  ; id:FIT0001\n    a  -1\n    b  1\n");
        let RowClassification::Conflicting { diffs, .. } =
            classify(&index, "FIT0001", &proposed[0], true)
        else {
            panic!("an ambiguous id must never be edited");
        };
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].field, "id");
    }

    /// A comment the user added is not a conflict. It is a note.
    #[test]
    fn an_added_note_is_not_a_conflict() {
        let holding = journal(
            "2026-01-06 * GROCERY MART  ; id:FIT0002, reimbursed by work\n    assets:bank:checking          -32.10\n    expenses:unknown               32.10\n",
        );
        let index = build_index(&holding, ID_TAG);
        let proposed = proposal(
            "2026-01-06 * GROCERY MART  ; id:FIT0002\n    assets:bank:checking          -32.10\n    expenses:unknown               32.10\n",
        );
        assert_eq!(
            classify(&index, "FIT0002", &proposed[0], true),
            RowClassification::Unchanged
        );
    }

    #[test]
    fn a_proposal_with_no_ids_reconciles_to_nothing_at_all() {
        let holding = journal(IMPORTED);
        let proposed = proposal("2026-01-08 * ACME\n    a  1\n    b  -1\n");
        let index = build_index(&holding, ID_TAG);
        assert_eq!(reconcile(&index, &proposed, ID_TAG), None);
    }

    #[test]
    fn reconcile_splits_a_redownload_four_ways() {
        let holding = journal(IMPORTED);
        let proposed = proposal(
            "\
2026-01-05 * COFFEE SHOP  ; id:FIT0001
    assets:bank:checking           -4.50
    expenses:unknown                4.50

2026-01-06 * GROCERY MART  ; id:FIT0002
    assets:bank:checking          -35.60
    expenses:unknown               35.60

2026-01-08 * ACME PAYROLL  ; id:FIT0003
    assets:bank:checking         1500.00
    expenses:unknown            -1500.00
",
        );
        let index = build_index(&holding, ID_TAG);
        let rows = reconcile(&index, &proposed, ID_TAG).expect("ids are declared");
        assert!(matches!(
            rows[0].classification,
            RowClassification::StatusOnly { .. }
        ));
        assert!(matches!(
            rows[1].classification,
            RowClassification::Conflicting { .. }
        ));
        assert_eq!(rows[2].classification, RowClassification::New);
        assert_eq!(rows[2].id, "FIT0003");
    }

    /// The invariant the whole opt-in rests on: with nothing to drop, the
    /// proposal comes back the same `str` it went in as.
    #[test]
    fn retaining_everything_returns_the_bytes_untouched() {
        let holding = journal("2026-01-01 opening\n    a  1\n    b  -1\n");
        let index = build_index(&holding, ID_TAG);
        let entries = "2026-01-08 * ACME  ; id:FIT0003\n    a  1500.00\n    b  -1500.00\n\n";
        let proposed = proposal(entries);
        let kept = retain_new(entries, &proposed, &index, ID_TAG);
        assert!(matches!(kept, Cow::Borrowed(_)), "it must not be rebuilt");
        assert_eq!(kept, entries);
    }

    #[test]
    fn retain_new_removes_a_matched_entry_and_its_separator() {
        let holding = journal(IMPORTED);
        let index = build_index(&holding, ID_TAG);
        let entries = "\
2026-01-05 * COFFEE SHOP  ; id:FIT0001
    assets:bank:checking           -4.50
    expenses:unknown                4.50

2026-01-08 * ACME PAYROLL  ; id:FIT0003
    assets:bank:checking         1500.00
    expenses:unknown            -1500.00

";
        let proposed = proposal(entries);
        let kept = retain_new(entries, &proposed, &index, ID_TAG);
        assert_eq!(
            kept,
            "\
2026-01-08 * ACME PAYROLL  ; id:FIT0003
    assets:bank:checking         1500.00
    expenses:unknown            -1500.00

"
        );
        // Still a journal, and still exactly the transaction that survived.
        let reread = journal(&kept);
        assert_eq!(reread.transactions.len(), 1);
        assert_eq!(reread.transactions[0].description, "ACME PAYROLL");
    }

    /// Dropping the LAST entry must not leave a dangling separator or take the
    /// one above it with it.
    #[test]
    fn retain_new_removes_a_trailing_entry_cleanly() {
        let holding = journal(IMPORTED);
        let index = build_index(&holding, ID_TAG);
        let entries = "\
2026-01-08 * ACME PAYROLL  ; id:FIT0003
    assets:bank:checking         1500.00
    expenses:unknown            -1500.00

2026-01-06 * GROCERY MART  ; id:FIT0002
    assets:bank:checking          -32.10
    expenses:unknown               32.10

";
        let proposed = proposal(entries);
        let kept = retain_new(entries, &proposed, &index, ID_TAG);
        assert_eq!(
            kept,
            "\
2026-01-08 * ACME PAYROLL  ; id:FIT0003
    assets:bank:checking         1500.00
    expenses:unknown            -1500.00

"
        );
    }

    /// Every entry matched: the proposal empties out, and `appended_text`'s
    /// "an empty proposal appends nothing" path takes over.
    #[test]
    fn retain_new_can_empty_a_proposal() {
        let holding = journal(IMPORTED);
        let index = build_index(&holding, ID_TAG);
        let entries = "\
2026-01-05 * COFFEE SHOP  ; id:FIT0001
    assets:bank:checking           -4.50
    expenses:unknown                4.50

";
        let proposed = proposal(entries);
        assert_eq!(retain_new(entries, &proposed, &index, ID_TAG), "");
    }
}
