//! What is due, whether it ties, and the entries that post it (plan 45 D6).
//!
//! An occurrence is **due** when its date is on or before `today`, its
//! `(id, date)` isn't posted, and it falls after the schedule's restated cut
//! ([`super::index::ScheduleInfo::restated_through`]). A due occurrence on or
//! before `closed-through` is **due in a closed period**: listed apart, never
//! posted into the closed month, only as one catch-up adjustment dated today
//! ([`closed_adjustments`]).
//!
//! [`entries_to_post`] gives each due occurrence the D6 entry shape: the rule
//! as `hledger print --forecast` prints it, without `generated-transaction`,
//! with `recognized:` added, no status mark and no `reviewed:` tag (Q6).

use super::book::RecognitionBook;
use super::index::RecognitionIndex;
use super::schedule::{Party, occurrence_days};
use super::{
    RECOGNITION_TAG, RECOGNIZED_TAG, RESTATED_TAG, RESTATED_THROUGH_TAG, day_of, iso_of, tag_value,
};
use crate::decimal::Dec;
use crate::edit::upsert_tag;
use crate::model::{
    Amount, Journal, PeriodicTransaction, Posting, SourcePos, Status, Tindex, Transaction,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// One due occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueEntry {
    /// The schedule id.
    pub id: String,
    /// The scheduled date: what the entry is dated and what `recognized:`
    /// holds.
    pub recognized: String,
    /// The rule generating it, as an index into the book's rules.
    pub rule: usize,
    /// On or before `closed-through`.
    pub closed: bool,
    /// The schedule's customer or vendor, for grouping.
    pub party: Option<Party>,
}

/// The due occurrences, open and closed, each in `(date, id)` order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DueSet {
    /// Postable as entries.
    pub open: Vec<DueEntry>,
    /// On or before `closed-through`: only as a catch-up adjustment.
    pub closed: Vec<DueEntry>,
}

impl DueSet {
    /// How many are due, open and closed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.open.len() + self.closed.len()
    }

    /// Whether nothing is due.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The open entries dated on or before `through` (and `today`, which the
    /// set was built under): what **Post through ‹date›** writes.
    #[must_use]
    pub fn through(&self, through: &str) -> Vec<DueEntry> {
        self.open
            .iter()
            .filter(|entry| entry.recognized.as_str() <= through)
            .cloned()
            .collect()
    }
}

/// The due set as of `today` (never later), with `closed_through` splitting
/// off the closed ones (D6).
#[must_use]
pub fn due(
    index: &RecognitionIndex,
    book: &RecognitionBook,
    today: &str,
    closed_through: Option<&str>,
) -> DueSet {
    let today = day_of(today);
    let closed = closed_through.map(day_of);
    let mut set = DueSet::default();
    for (n, schedule) in index.schedules().iter().enumerate() {
        if schedule.rules.is_empty() {
            continue;
        }
        let cut = schedule.restated_through().map(day_of);
        let occurrences: Vec<(i32, usize)> = match &schedule.occurrences {
            Some(all) => all
                .iter()
                .copied()
                .take_while(|(day, _)| *day <= today)
                .collect(),
            None => {
                let mut walked: Vec<(i32, usize)> = schedule
                    .rules
                    .iter()
                    .flat_map(|&rule| {
                        occurrence_days(&book.rules[rule].period, Some(today))
                            .unwrap_or_default()
                            .into_iter()
                            .map(move |day| (day, rule))
                    })
                    .collect();
                walked.sort_unstable();
                walked
            }
        };
        for (day, rule) in occurrences {
            if cut.is_some_and(|cut| day <= cut) || index.is_posted_day(n, day) {
                continue;
            }
            let entry = DueEntry {
                id: schedule.id.clone(),
                recognized: iso_of(day),
                rule,
                closed: closed.is_some_and(|closed| day <= closed),
                party: schedule.party().cloned(),
            };
            if entry.closed {
                set.closed.push(entry);
            } else {
                set.open.push(entry);
            }
        }
    }
    let key = |entry: &DueEntry| (entry.recognized.clone(), entry.id.clone());
    set.open.sort_by_key(key);
    set.closed.sort_by_key(key);
    set
}

/// A rule's header comment with `name: value` set (and any earlier value of
/// it dropped), in the model's comment shape.
pub(crate) fn comment_with(comment: &str, tags: &[(&str, &str)]) -> String {
    let body = comment.strip_suffix('\n').unwrap_or(comment);
    let (first, rest) = body
        .split_once('\n')
        .map_or((body, None), |(f, r)| (f, Some(r)));
    let first = tags.iter().fold(first.to_string(), |line, (name, _)| {
        upsert_tag(&line, name, None)
    });
    let added = tags
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    let first = match (first.trim(), added.as_str()) {
        ("", _) => added.clone(),
        (kept, "") => kept.to_string(),
        (kept, _) => format!("{kept}, {added}"),
    };
    match rest {
        Some(rest) => format!("{first}\n{rest}\n"),
        None => format!("{first}\n"),
    }
}

/// A transaction built by Ledgeline: no position, no file — placement is
/// [`crate::edit::JournalEditor::add_transactions`]'s job.
pub(crate) fn new_transaction(
    date: &str,
    description: &str,
    comment: String,
    postings: Vec<Posting>,
) -> Transaction {
    let tags = crate::parse::parse_tags(&comment);
    Transaction {
        index: Tindex(0),
        date: date.to_string(),
        date2: None,
        status: Status::Unmarked,
        code: String::new(),
        description: description.to_string(),
        comment,
        preceding_comment: String::new(),
        tags,
        postings,
        source_span: (
            SourcePos { line: 1, column: 1 },
            SourcePos { line: 1, column: 1 },
        ),
        source_file: PathBuf::new(),
    }
}

/// The entry `rule` posts for its occurrence `recognized`, dated `date`.
#[must_use]
pub fn entry_for(rule: &PeriodicTransaction, recognized: &str, date: &str) -> Transaction {
    let postings = rule
        .postings
        .iter()
        .map(|posting| Posting {
            generated: None,
            elided: false,
            balance_assertion: None,
            date: None,
            date2: None,
            ..posting.clone()
        })
        .collect();
    new_transaction(
        date,
        &rule.description,
        comment_with(&rule.comment, &[(RECOGNIZED_TAG, recognized)]),
        postings,
    )
}

/// The entries that post `entries` (D6 shape), in the given order.
#[must_use]
pub fn entries_to_post(book: &RecognitionBook, entries: &[DueEntry]) -> Vec<Transaction> {
    entries
        .iter()
        .filter_map(|entry| {
            book.rules
                .get(entry.rule)
                .map(|rule| entry_for(rule, &entry.recognized, &entry.recognized))
        })
        .collect()
}

/// `postings` summed per account, commodity and comment, zeros dropped, in
/// first-seen order.
pub(crate) fn merged(postings: impl IntoIterator<Item = Posting>) -> Vec<Posting> {
    let mut out: Vec<Posting> = Vec::new();
    for posting in postings {
        let Some(amount) = posting.amounts.first() else {
            continue;
        };
        match out.iter_mut().find(|have| {
            have.account == posting.account
                && have.comment == posting.comment
                && have.amounts.first().map(|a| &a.commodity) == Some(&amount.commodity)
        }) {
            Some(have) => {
                if let Some(sum) = have.amounts.first_mut() {
                    sum.quantity = sum.quantity.add(amount.quantity).unwrap_or(sum.quantity);
                    sum.style.precision = sum.style.precision.max(sum.quantity.places);
                }
            }
            None => out.push(posting),
        }
    }
    out.into_iter()
        .filter(|posting| posting.amounts.iter().any(|a| !a.quantity.is_zero()))
        .collect()
}

/// One catch-up adjustment per schedule for its closed due occurrences
/// (D6's closed section), dated `today` and tagged `recognition: ID,
/// restated: TODAY, restated-through: CLOSED_THROUGH`, so none of them is ever
/// due again. A schedule's closed occurrences are always folded together.
#[must_use]
pub fn closed_adjustments(
    book: &RecognitionBook,
    closed: &[DueEntry],
    today: &str,
    closed_through: &str,
) -> Vec<Transaction> {
    let ids: BTreeSet<&str> = closed.iter().map(|entry| entry.id.as_str()).collect();
    ids.into_iter()
        .filter_map(|id| {
            let mine: Vec<&DueEntry> = closed.iter().filter(|entry| entry.id == id).collect();
            let first = book.rules.get(mine.first()?.rule)?;
            let postings = merged(
                mine.iter()
                    .filter_map(|entry| book.rules.get(entry.rule))
                    .flat_map(|rule| entry_for(rule, today, today).postings),
            );
            (!postings.is_empty()).then(|| {
                new_transaction(
                    today,
                    format!("{} (closed periods)", first.description).trim(),
                    comment_with(
                        "",
                        &[
                            (RECOGNITION_TAG, id),
                            (RESTATED_TAG, today),
                            (RESTATED_THROUGH_TAG, closed_through),
                        ],
                    ),
                    postings,
                )
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Ties
// ---------------------------------------------------------------------------

/// One month's check of one deferred/prepaid account (D6 "Ties").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tie {
    /// `YYYY-MM`.
    pub month: String,
    /// The account.
    pub account: String,
    /// The commodity.
    pub commodity: String,
    /// The account's balance at month end, before posting.
    pub before: Dec,
    /// After posting every open due entry dated up to month end.
    pub after: Dec,
    /// What the schedules say is left at month end: every source line dated
    /// up to then, less every occurrence and adjustment up to then.
    pub scheduled: Dec,
    /// `after == scheduled`.
    pub ties: bool,
}

/// The last day of `month` (`YYYY-MM`).
fn month_end(month: &str) -> String {
    crate::reports::periods::add_days(
        &crate::reports::periods::add_months(&format!("{month}-01"), 1),
        -1,
    )
}

/// Dated movements of one (account, commodity), sorted, with running sums.
#[derive(Default)]
struct Series {
    points: Vec<(String, Dec)>,
}

impl Series {
    fn push(&mut self, date: &str, quantity: Dec) {
        self.points.push((date.to_string(), quantity));
    }

    fn finish(mut self) -> Self {
        self.points.sort_by(|a, b| a.0.cmp(&b.0));
        let mut running = Dec::zero();
        for point in &mut self.points {
            running = running.add(point.1).unwrap_or(running);
            point.1 = running;
        }
        self
    }

    fn at(&self, date: &str) -> Dec {
        let n = self.points.partition_point(|(d, _)| d.as_str() <= date);
        n.checked_sub(1).map_or(Dec::zero(), |i| self.points[i].1)
    }
}

type SeriesKey = (String, String);

fn add_posting(series: &mut BTreeMap<SeriesKey, Series>, date: &str, posting: &Posting) {
    for amount in &posting.amounts {
        series
            .entry((posting.account.0.clone(), amount.commodity.0.clone()))
            .or_default()
            .push(date, amount.quantity);
    }
}

/// The D6 ties for every month the open due entries fall in, for every
/// deferred/prepaid account a schedule draws on: the account's month-end
/// balance before and after posting, and the scheduled remaining balance.
#[must_use]
pub fn ties(
    journal: &Journal,
    index: &RecognitionIndex,
    book: &RecognitionBook,
    due: &DueSet,
) -> Vec<Tie> {
    let accounts: BTreeSet<&str> = index
        .schedules()
        .iter()
        .flat_map(|schedule| {
            schedule
                .sources
                .iter()
                .map(|source| source.account.as_str())
        })
        .collect();
    if accounts.is_empty() || due.open.is_empty() {
        return Vec::new();
    }
    let wanted = |posting: &Posting| accounts.contains(posting.account.0.as_str());
    let mut actual: BTreeMap<SeriesKey, Series> = BTreeMap::new();
    for txn in &journal.transactions {
        for posting in txn.postings.iter().filter(|p| wanted(p)) {
            add_posting(&mut actual, &txn.date, posting);
        }
    }
    let mut pending: BTreeMap<SeriesKey, Series> = BTreeMap::new();
    for entry in &due.open {
        if let Some(rule) = book.rules.get(entry.rule) {
            for posting in rule.postings.iter().filter(|p| wanted(p)) {
                add_posting(&mut pending, &entry.recognized, posting);
            }
        }
    }
    let mut scheduled: BTreeMap<SeriesKey, Series> = BTreeMap::new();
    for schedule in index.schedules() {
        for source in &schedule.sources {
            scheduled
                .entry((source.account.clone(), source.amount.commodity.0.clone()))
                .or_default()
                .push(&source.date, source.amount.quantity);
        }
        for (day, rule) in schedule.occurrences.iter().flatten() {
            let date = iso_of(*day);
            for posting in book.rules[*rule].postings.iter().filter(|p| wanted(p)) {
                add_posting(&mut scheduled, &date, posting);
            }
        }
        for adjustment in &schedule.adjustments {
            let txn = &journal.transactions[adjustment.position];
            for posting in txn.postings.iter().filter(|p| wanted(p)) {
                add_posting(&mut scheduled, &txn.date, posting);
            }
        }
    }
    let finish = |map: BTreeMap<SeriesKey, Series>| -> BTreeMap<SeriesKey, Series> {
        map.into_iter().map(|(k, s)| (k, s.finish())).collect()
    };
    let (actual, pending, scheduled) = (finish(actual), finish(pending), finish(scheduled));
    let keys: BTreeSet<&SeriesKey> = pending.keys().chain(scheduled.keys()).collect();
    let months: BTreeSet<&str> = due
        .open
        .iter()
        .map(|entry| &entry.recognized[..7])
        .collect();
    months
        .into_iter()
        .flat_map(|month| {
            let end = month_end(month);
            keys.iter()
                .map(|key| {
                    let at = |map: &BTreeMap<SeriesKey, Series>| {
                        map.get(*key).map_or(Dec::zero(), |series| series.at(&end))
                    };
                    let before = at(&actual);
                    let after = before.add(at(&pending)).unwrap_or(before);
                    let scheduled = at(&scheduled);
                    Tie {
                        month: month.to_string(),
                        account: key.0.clone(),
                        commodity: key.1.clone(),
                        before,
                        after,
                        scheduled,
                        ties: after == scheduled,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The amounts `entry`'s postings put on `account`, per commodity.
#[must_use]
pub fn amounts_on(txn: &Transaction, account: &str) -> Vec<Amount> {
    txn.postings
        .iter()
        .filter(|posting| posting.account.0 == account)
        .flat_map(|posting| posting.amounts.iter().cloned())
        .collect()
}

/// Whether `txn` is the entry for `(id, recognized)`.
#[must_use]
pub fn fills(txn: &Transaction, id: &str, recognized: &str) -> bool {
    tag_value(&txn.tags, RECOGNITION_TAG) == Some(id)
        && tag_value(&txn.tags, RECOGNIZED_TAG) == Some(recognized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;
    use proptest::prelude::*;
    use std::path::Path;

    const BOOKS: &str = "\
account liabilities:deferred-revenue  ; type: L, kpi: deferred-revenue
account revenue:sales  ; type: R
account assets:bank  ; type: C

2026-01-05 Customer X Payment
    liabilities:deferred-revenue  $-600.00  ; customer: Customer X, recognition: aaa111, term-months: 6
    assets:bank

2026-01-05 Customer X Recognized  ; recognition: aaa111, recognized: 2026-01-05
    liabilities:deferred-revenue  $100.00  ; customer: Customer X
    revenue:sales  $-100.00  ; customer: Customer X

2026-02-03 Customer X Recognized  ; recognition: aaa111, recognized: 2026-02-05
    liabilities:deferred-revenue  $100.00  ; customer: Customer X
    revenue:sales  $-100.00  ; customer: Customer X
";

    const RULES: &str = "\
~ monthly from 2026-01-05 to 2026-07-05  Customer X Recognized  ; recognition: aaa111, renewal-date: 2026-07-05
    liabilities:deferred-revenue  $100.00  ; customer: Customer X
    revenue:sales  $-100.00  ; customer: Customer X
";

    fn setup(books: &str) -> (Journal, RecognitionBook, RecognitionIndex) {
        let journal = parse_journal(books, "books.journal").expect("parses");
        let book = RecognitionBook::parse(
            RULES,
            Path::new("/nonexistent/r.journal"),
            &journal.commodity_styles,
        )
        .expect("parses");
        let index = RecognitionIndex::build(&journal, &book);
        (journal, book, index)
    }

    fn dates(entries: &[DueEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.recognized.as_str()).collect()
    }

    #[test]
    fn a_moved_entry_still_fills_its_period_and_closed_ones_split_off() {
        let (_, book, index) = setup(BOOKS);
        let set = due(&index, &book, "2026-04-20", Some("2026-03-31"));
        assert_eq!(dates(&set.closed), vec!["2026-03-05"]);
        assert_eq!(dates(&set.open), vec!["2026-04-05"]);
        assert!(due(&index, &book, "2026-01-04", None).is_empty());
    }

    #[test]
    fn the_entry_shape_is_the_rule_plus_recognized() {
        let (_, book, index) = setup(BOOKS);
        let set = due(&index, &book, "2026-03-05", None);
        let entries = entries_to_post(&book, &set.open);
        assert_eq!(
            crate::edit::format_transaction(&entries[0]),
            "2026-03-05 Customer X Recognized  ; recognition: aaa111, renewal-date: 2026-07-05, recognized: 2026-03-05\n    \
             liabilities:deferred-revenue  $100.00  ; customer: Customer X\n    \
             revenue:sales                 $-100.00  ; customer: Customer X\n"
        );
    }

    #[test]
    fn posting_is_idempotent_and_ties() {
        let (journal, book, index) = setup(BOOKS);
        let set = due(&index, &book, "2026-04-05", None);
        let tie = ties(&journal, &index, &book, &set);
        assert!(tie.iter().all(|t| t.ties), "{tie:#?}");
        let mut text = BOOKS.to_string();
        for entry in entries_to_post(&book, &set.open) {
            text.push('\n');
            text.push_str(&crate::edit::format_transaction(&entry));
        }
        let (_, book, index) = setup(&text);
        assert!(due(&index, &book, "2026-04-05", None).is_empty());
        assert!(index.issues().is_empty(), "{:?}", index.issues());
    }

    #[test]
    fn closed_periods_fold_into_one_adjustment_that_is_never_due_again() {
        let (_, book, index) = setup(BOOKS);
        let set = due(&index, &book, "2026-05-10", Some("2026-04-30"));
        assert_eq!(dates(&set.closed), vec!["2026-03-05", "2026-04-05"]);
        let adjustments = closed_adjustments(&book, &set.closed, "2026-05-10", "2026-04-30");
        assert_eq!(adjustments.len(), 1);
        let text = format!(
            "{BOOKS}\n{}",
            crate::edit::format_transaction(&adjustments[0])
        );
        assert!(text.contains("restated-through: 2026-04-30"), "{text}");
        assert!(text.contains("$200.00"), "{text}");
        let (_, book, index) = setup(&text);
        let set = due(&index, &book, "2026-05-10", Some("2026-04-30"));
        assert!(set.closed.is_empty());
        assert_eq!(dates(&set.open), vec!["2026-05-05"]);
        assert!(index.issues().is_empty(), "{:?}", index.issues());
    }

    proptest! {
        #[test]
        fn nothing_after_today_is_ever_due(today in 0_i32..400, closed in proptest::option::of(0_i32..400)) {
            let (_, book, index) = setup(BOOKS);
            let base = day_of("2025-12-01");
            let today = iso_of(base + today);
            let closed = closed.map(|c| iso_of(base + c));
            let set = due(&index, &book, &today, closed.as_deref());
            for entry in set.open.iter().chain(&set.closed) {
                prop_assert!(entry.recognized <= today);
                prop_assert!(!index.is_posted(&entry.id, &entry.recognized));
            }
            for entry in &set.open {
                prop_assert!(closed.as_deref().is_none_or(|c| entry.recognized.as_str() > c));
            }
            for entry in &set.closed {
                prop_assert!(closed.as_deref().is_some_and(|c| entry.recognized.as_str() <= c));
            }
        }
    }
}
