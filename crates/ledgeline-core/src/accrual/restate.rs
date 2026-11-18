//! Restatement (plan 45 D4, D-credit): when a schedule's inputs change, what
//! happens to the rule and to everything already posted from it.
//!
//! Posted entries are matched to periods by **ordinal**: the entry filling
//! the old schedule's k-th period is the one that fills the new schedule's
//! k-th. Then, with `C` the `closed-through` date:
//!
//! - an entry dated after `C` is **rewritten** to its new period (amount,
//!   accounts, and — when the period moved — date and `recognized:`); if the
//!   new schedule has no k-th period, or it now falls on or before `C`, the
//!   entry is **deleted**;
//! - an entry dated on or before `C` is **kept** as it is;
//! - **one adjustment**, dated `today`, makes up the difference: every new
//!   period on or before `C`, less what the kept entries and earlier
//!   adjustments already recognized. It's tagged `recognition: ID,
//!   restated: TODAY, restated-through: C`, so no occurrence on or before `C`
//!   is ever due again.
//!
//! So after applying it, posted + adjustments + periods still to post equal
//! the new total exactly.

use super::book::RecognitionBook;
use super::due::{entry_for, new_transaction};
use super::index::ScheduleInfo;
use super::schedule::{Schedule, ScheduleError, ScheduleLine, amount_in, occurrence_days};
use super::{RECOGNITION_TAG, RESTATED_TAG, RESTATED_THROUGH_TAG, day_of};
use crate::decimal::Dec;
use crate::edit::EditOp;
use crate::model::{
    Journal, PeriodicTransaction, Posting, PostingType, Status, Tindex, Transaction,
};
use crate::parse::JournalStyles;
use std::collections::BTreeSet;
use thiserror::Error;

/// Why a restatement can't be computed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RestateError {
    /// The old and new schedules are different schedules.
    #[error("the restatement changes schedule {old} into {new}; ids never change")]
    IdChanged {
        /// The old id.
        old: String,
        /// The new id.
        new: String,
    },
    /// The new rules don't read back.
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    /// A posted entry or adjustment isn't where the index says.
    #[error("the journal changed under the restatement: {0}")]
    Stale(String),
}

/// What happens to one posted entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowAction {
    /// Closed: kept as it is; the adjustment absorbs the difference.
    Kept,
    /// Open, and already what the new schedule says.
    Unchanged,
    /// Open: rewritten in place.
    Rewrite,
    /// Open, with no place in the new schedule (or one now closed): deleted.
    Delete,
}

/// One posted entry in the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestatementRow {
    /// Its position in `journal.transactions`.
    pub position: usize,
    /// Its `tindex`.
    pub tindex: Tindex,
    /// Its date.
    pub date: String,
    /// The period it fills now.
    pub recognized: String,
    /// The period it fills after (`None` when deleted).
    pub recognized_after: Option<String>,
    /// Dated on or before `closed-through`.
    pub closed: bool,
    /// Per line, what it recognizes now.
    pub before: Vec<Dec>,
    /// Per line, what it recognizes after (`None` when deleted; `before` when
    /// kept).
    pub after: Option<Vec<Dec>>,
    /// What happens to it.
    pub action: RowAction,
}

/// The rule group, before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleChange {
    /// The rules as they are.
    pub before: String,
    /// The rules as they will be.
    pub after: String,
}

/// Everything a restatement writes (D4), for a preview and then
/// [`super::ops`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restatement {
    /// The schedule.
    pub id: String,
    /// The rule group change, when the rules change.
    pub rule_change: Option<RuleChange>,
    /// Every posted entry, in date order, with what happens to it.
    pub rows: Vec<RestatementRow>,
    /// The open entries to rewrite: each is one [`EditOp::Replace`].
    pub rewrites: Vec<(Tindex, Vec<EditOp>)>,
    /// The open entries to delete.
    pub deletes: Vec<Tindex>,
    /// The one catch-up adjustment, when the closed periods differ.
    pub adjustment: Option<Transaction>,
    /// The new schedule's total per line, for the preview's check.
    pub total_after: Vec<Dec>,
}

impl Restatement {
    /// Whether nothing would change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rule_change.is_none()
            && self.rewrites.is_empty()
            && self.deletes.is_empty()
            && self.adjustment.is_none()
    }
}

/// What `postings` recognize on each line: the sum of their quantities on the
/// line's target account, in the line's commodity.
fn on_lines(postings: &[Posting], lines: &[ScheduleLine]) -> Vec<Dec> {
    lines
        .iter()
        .map(|line| {
            postings
                .iter()
                .filter(|posting| posting.account.0 == line.target_account)
                .flat_map(|posting| &posting.amounts)
                .filter(|amount| amount.commodity == line.total.commodity)
                .try_fold(Dec::zero(), |sum, amount| sum.add(amount.quantity))
                .unwrap_or(Dec::zero())
        })
        .collect()
}

fn add_all(into: &mut [Dec], more: &[Dec]) -> Result<(), ScheduleError> {
    for (sum, value) in into.iter_mut().zip(more) {
        *sum = sum.add(*value)?;
    }
    Ok(())
}

/// The parsed rules of `schedule`, with each period's generating rule.
fn rules_by_date(
    schedule: &Schedule,
    styles: &JournalStyles<'_>,
) -> Result<Vec<(String, PeriodicTransaction)>, ScheduleError> {
    let text = schedule.rules_text();
    let book = RecognitionBook::parse(
        &text,
        std::path::Path::new("recognition.journal"),
        styles.declared,
    )
    .map_err(|error| ScheduleError::Invalid(error.to_string()))?;
    let mut out = Vec::new();
    for rule in book.rules {
        let days = occurrence_days(&rule.period, None)
            .ok_or_else(|| ScheduleError::Unbounded(rule.period.raw.clone()))?;
        out.extend(
            days.into_iter()
                .map(|day| (super::iso_of(day), rule.clone())),
        );
    }
    Ok(out)
}

/// Whether two transactions say the same thing, wherever they are.
fn same_entry(a: &Transaction, b: &Transaction) -> bool {
    let shape = |txn: &Transaction| {
        (
            txn.date.clone(),
            txn.description.clone(),
            txn.tags.clone(),
            txn.postings
                .iter()
                .map(|p| {
                    (
                        p.account.0.clone(),
                        p.amounts
                            .iter()
                            .map(|a| (a.commodity.0.clone(), a.quantity.normalized()))
                            .collect::<Vec<_>>(),
                        p.tags.clone(),
                    )
                })
                .collect::<Vec<_>>(),
        )
    };
    shape(a) == shape(b)
}

/// The postings of an adjustment of `amounts` per line: the source account
/// takes `-amount`, the target `+amount`, each with the line's party tag.
fn adjustment_postings(
    lines: &[ScheduleLine],
    amounts: &[Dec],
    styles: &JournalStyles<'_>,
) -> Result<Vec<Posting>, ScheduleError> {
    let posting = |account: &str, line: &ScheduleLine, quantity: Dec| {
        let style = styles
            .declared_style(&line.total.commodity)
            .cloned()
            .unwrap_or_else(|| line.total.style.clone());
        let comment = line
            .party
            .as_ref()
            .map(|party| format!("{}: {}\n", party.kind.tag(), party.name))
            .unwrap_or_default();
        Posting {
            status: Status::Unmarked,
            ptype: PostingType::Regular,
            account: crate::model::AccountName(account.to_string()),
            amounts: vec![amount_in(&line.total.commodity, &style, quantity)],
            balance_assertion: None,
            date: None,
            date2: None,
            tags: crate::parse::parse_tags(&comment),
            comment,
            generated: None,
            elided: false,
        }
    };
    let mut out = Vec::new();
    for (line, amount) in lines.iter().zip(amounts) {
        if amount.is_zero() {
            continue;
        }
        out.push(posting(&line.source_account, line, amount.neg()?));
        out.push(posting(&line.target_account, line, *amount));
    }
    Ok(out)
}

/// The restatement of schedule `old` (as its rules now stand) into `new`,
/// given what `info` says is posted in `journal` (D4).
///
/// # Errors
/// [`RestateError::IdChanged`] when the ids differ; [`RestateError::Schedule`]
/// when the new rules don't read back; [`RestateError::Stale`] when `info`
/// doesn't describe `journal`.
pub fn restatement(
    old: &Schedule,
    new: &Schedule,
    journal: &Journal,
    info: &ScheduleInfo,
    closed_through: Option<&str>,
    today: &str,
    styles: &JournalStyles<'_>,
) -> Result<Restatement, RestateError> {
    if old.id != new.id {
        return Err(RestateError::IdChanged {
            old: old.id.clone(),
            new: new.id.clone(),
        });
    }
    let closed = closed_through.map(day_of);
    let is_closed = |date: &str| closed.is_some_and(|c| day_of(date) <= c);
    let new_rules = rules_by_date(new, styles)?;
    let rule_on = |date: &str| new_rules.iter().find(|(d, _)| d == date).map(|(_, r)| r);
    let old_lines = &old.spec.lines;
    let width = old_lines.len().max(new.spec.lines.len());
    let lines_at = |i: usize| new.spec.lines.get(i).or_else(|| old_lines.get(i));
    let lines: Vec<ScheduleLine> = (0..width).filter_map(|i| lines_at(i).cloned()).collect();
    let pad = |mut v: Vec<Dec>| {
        v.resize(width, Dec::zero());
        v
    };

    let mut rows = Vec::new();
    let mut rewrites = Vec::new();
    let mut deletes = Vec::new();
    let mut kept = vec![Dec::zero(); width];
    let mut filled = BTreeSet::new();
    let mut posted: Vec<_> = info.posted.iter().collect();
    posted.sort_by(|a, b| (&a.recognized, a.position).cmp(&(&b.recognized, b.position)));
    for entry in posted {
        if !filled.insert(entry.recognized.as_str()) {
            continue; // A duplicate is the duplicate finding's business.
        }
        let txn = journal
            .transactions
            .get(entry.position)
            .filter(|txn| txn.index == entry.tindex)
            .ok_or_else(|| RestateError::Stale(format!("entry {} moved", entry.tindex.0)))?;
        let before = pad(on_lines(&txn.postings, old_lines));
        let ordinal = old.periods.iter().position(|p| p.date == entry.recognized);
        let row =
            |action, recognized_after: Option<String>, after: Option<Vec<Dec>>| RestatementRow {
                position: entry.position,
                tindex: entry.tindex,
                date: entry.date.clone(),
                recognized: entry.recognized.clone(),
                recognized_after,
                closed: is_closed(&entry.date),
                before: before.clone(),
                after,
                action,
            };
        if is_closed(&entry.date) {
            add_all(&mut kept, &before)?;
            rows.push(row(
                RowAction::Kept,
                Some(entry.recognized.clone()),
                Some(before.clone()),
            ));
            continue;
        }
        let target = ordinal
            .and_then(|k| new.periods.get(k).map(|period| (k, period)))
            .filter(|(_, period)| !is_closed(&period.date));
        match target.and_then(|(k, period)| rule_on(&period.date).map(|rule| (k, period, rule))) {
            Some((k, period, rule)) => {
                let moved = old.periods[k].date != period.date;
                let date = if moved {
                    period.date.as_str()
                } else {
                    entry.date.as_str()
                };
                let rewritten = entry_for(rule, &period.date, date);
                let after = pad(period.amounts.clone());
                if same_entry(txn, &rewritten) {
                    rows.push(row(
                        RowAction::Unchanged,
                        Some(period.date.clone()),
                        Some(after),
                    ));
                } else {
                    rewrites.push((entry.tindex, vec![EditOp::Replace(Box::new(rewritten))]));
                    rows.push(row(
                        RowAction::Rewrite,
                        Some(period.date.clone()),
                        Some(after),
                    ));
                }
            }
            None => {
                deletes.push(entry.tindex);
                rows.push(row(RowAction::Delete, None, None));
            }
        }
    }

    // The adjustment: every new period on or before C, less what the kept
    // entries and earlier adjustments recognized.
    let mut owed = vec![Dec::zero(); width];
    for period in new.periods.iter().filter(|p| is_closed(&p.date)) {
        add_all(&mut owed, &pad(period.amounts.clone()))?;
    }
    let mut already = kept;
    for adjustment in &info.adjustments {
        let txn = journal
            .transactions
            .get(adjustment.position)
            .filter(|txn| txn.index == adjustment.tindex)
            .ok_or_else(|| {
                RestateError::Stale(format!("adjustment {} moved", adjustment.tindex.0))
            })?;
        add_all(&mut already, &pad(on_lines(&txn.postings, old_lines)))?;
    }
    let difference: Vec<Dec> = owed
        .iter()
        .zip(&already)
        .map(|(owed, already)| owed.sub(*already))
        .collect::<Result<_, _>>()
        .map_err(ScheduleError::from)?;
    let adjustment = if difference.iter().all(Dec::is_zero) {
        None
    } else {
        let mut tags = vec![(RECOGNITION_TAG, new.id.as_str()), (RESTATED_TAG, today)];
        if let Some(through) = closed_through {
            tags.push((RESTATED_THROUGH_TAG, through));
        }
        Some(new_transaction(
            today,
            format!("{} (restated)", new.spec.description).trim(),
            super::due::comment_with("", &tags),
            adjustment_postings(&lines, &difference, styles)?,
        ))
    };
    let (before, after) = (old.rules_text(), new.rules_text());
    Ok(Restatement {
        id: new.id.clone(),
        rule_change: (before != after).then_some(RuleChange { before, after }),
        rows,
        rewrites,
        deletes,
        adjustment,
        total_after: new.totals(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accrual::index::RecognitionIndex;
    use crate::accrual::schedule::{Party, PartyKind, Proration, ScheduleSpec, Term, build};
    use crate::parse::parse_journal;
    use std::path::Path;

    fn spec(total: &str, term: u32) -> ScheduleSpec {
        let amount = parse_journal(
            &format!("2026-01-01 x\n    a  {total}\n    b\n"),
            "x.journal",
        )
        .expect("parses")
        .transactions[0]
            .postings[0]
            .amounts[0]
            .clone();
        ScheduleSpec {
            id: "aaa111".into(),
            lines: vec![ScheduleLine {
                source_account: "liabilities:deferred-revenue".into(),
                target_account: "revenue:sales".into(),
                total: amount,
                party: Some(Party {
                    kind: PartyKind::Customer,
                    name: "Customer X".into(),
                }),
            }],
            start: "2026-01-05".into(),
            term: Term::Months(term),
            proration: Proration::Anniversary,
            description: "Customer X Recognized".into(),
            header_tags: vec![],
        }
    }

    /// A journal with the old schedule's first `posted` periods posted.
    fn setup(posted: usize) -> (Journal, Schedule, RecognitionIndex) {
        let main = parse_journal("commodity $1,000.00\n", "books.journal").expect("parses");
        let styles = JournalStyles::of(&main);
        let old = build(&spec("$-600.00", 6), &styles).expect("builds");
        let book = RecognitionBook::parse(
            &old.rules_text(),
            Path::new("/nonexistent/r.journal"),
            &main.commodity_styles,
        )
        .expect("parses");
        let set = crate::accrual::due::due(
            &RecognitionIndex::build(&main, &book),
            &book,
            "2027-01-01",
            None,
        );
        let mut text = "commodity $1,000.00\n\n2026-01-05 Payment\n    liabilities:deferred-revenue  $-600.00  ; recognition: aaa111, term-months: 6\n    assets:bank\n".to_string();
        for entry in crate::accrual::due::entries_to_post(&book, &set.open[..posted]) {
            text.push('\n');
            text.push_str(&crate::edit::format_transaction(&entry));
        }
        let journal = parse_journal(&text, "books.journal").expect("parses");
        let index = RecognitionIndex::build(&journal, &book);
        (journal, old, index)
    }

    #[test]
    fn a_no_op_edit_is_empty() {
        let (journal, old, index) = setup(3);
        let styles = JournalStyles::of(&journal);
        let r = restatement(
            &old,
            &old,
            &journal,
            index.schedule("aaa111").expect("s"),
            Some("2026-02-28"),
            "2026-03-20",
            &styles,
        )
        .expect("restates");
        assert!(r.is_empty(), "{r:#?}");
    }

    #[test]
    fn closed_periods_are_kept_open_ones_rewritten_and_one_adjustment_makes_up_the_rest() {
        let (journal, old, index) = setup(3);
        let styles = JournalStyles::of(&journal);
        let new = build(&spec("$-570.00", 6), &styles).expect("builds");
        let r = restatement(
            &old,
            &new,
            &journal,
            index.schedule("aaa111").expect("s"),
            Some("2026-02-28"),
            "2026-03-20",
            &styles,
        )
        .expect("restates");
        let actions: Vec<RowAction> = r.rows.iter().map(|row| row.action).collect();
        assert_eq!(
            actions,
            vec![RowAction::Kept, RowAction::Kept, RowAction::Rewrite]
        );
        assert_eq!(r.rewrites.len(), 1);
        let adjustment = r.adjustment.as_ref().expect("an adjustment");
        assert_eq!(adjustment.date, "2026-03-20");
        // Two closed periods at $95 instead of $100: $10 back.
        let text = crate::edit::format_transaction(adjustment);
        assert!(
            text.contains("restated: 2026-03-20, restated-through: 2026-02-28"),
            "{text}"
        );
        assert!(
            text.contains("revenue:sales  $10.00") || text.contains("$10.00"),
            "{text}"
        );
        assert!(r.rule_change.is_some());
    }

    #[test]
    fn a_shortened_term_deletes_open_periods() {
        let (journal, old, index) = setup(5);
        let styles = JournalStyles::of(&journal);
        let new = build(&spec("$-300.00", 3), &styles).expect("builds");
        let r = restatement(
            &old,
            &new,
            &journal,
            index.schedule("aaa111").expect("s"),
            Some("2026-01-31"),
            "2026-06-01",
            &styles,
        )
        .expect("restates");
        let actions: Vec<RowAction> = r.rows.iter().map(|row| row.action).collect();
        assert_eq!(
            actions,
            // The kept periods' entries change too: their `renewal-date` moves.
            vec![
                RowAction::Kept,
                RowAction::Rewrite,
                RowAction::Rewrite,
                RowAction::Delete,
                RowAction::Delete
            ]
        );
        assert_eq!(r.deletes.len(), 2);
        assert!(r.adjustment.is_none());
    }
}
