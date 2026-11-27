//! Where the time went (plan 46 T-dash), what is unbilled (B-unbilled) and
//! what is unpaid (B-ap).
//!
//! [`summary`] is deliberately general — any one or two groupings, account
//! depth, top N with the rest as *other*, and a period series per row — so
//! the dashboard (Phase 4) can ask for what it shows without new routes.
//! Everything is exact; nothing is rounded here (B-round).

use super::link::{CustomerTime, customer_named, customer_time, timekeeper_for};
use super::{Qty, entry_id};
use crate::catalog::Catalog;
use crate::index::DocumentIndex;
use crate::table::money::MoneyStyle;
use ledgeline_core::Dec;
use ledgeline_core::reports::periods::{Interval, bucket_key};
use ledgeline_core::time::TimeQty;
use ledgeline_core::time::model::{TimeBook, TimeEntry};
use ledgeline_core::time::query::{self, Batch, Cadence, Filter, SuggestedPeriod};
use ledgeline_core::time::{TagRole, accounts::by_segment};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// One way to group time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// The account, cut to the query's depth.
    Account,
    /// The top-level branch.
    Branch,
    /// The linked customer (`""`: none).
    Customer,
    /// The person (`""`: unattributed).
    Person,
    /// `yes`/`no`.
    Billable,
    /// `unbilled`, `invoiced`, `nobill`, `not-billable`.
    Billed,
    /// The entry's note, trimmed (`""`: none): the dashboard's most
    /// frequent notes.
    Note,
    /// A period bucket (`2026-11`, `2026-W48`, …).
    Period(Interval),
}

impl Group {
    /// `account`, `branch`, `customer`, `person`, `billable`, `billed`,
    /// `note`, `day`, `week`, `month`, `quarter`, `year`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim() {
            "account" => Self::Account,
            "branch" => Self::Branch,
            "customer" => Self::Customer,
            "person" => Self::Person,
            "billable" => Self::Billable,
            "billed" => Self::Billed,
            "note" => Self::Note,
            other => Self::Period(interval(other)?),
        })
    }
}

/// `day`, `week`, `month`, `quarter`, `year` as an [`Interval`].
#[must_use]
pub fn interval(text: &str) -> Option<Interval> {
    Some(match text.trim() {
        "day" | "daily" => Interval::Daily,
        "week" | "weekly" => Interval::Weekly,
        "month" | "monthly" => Interval::Monthly,
        "quarter" | "quarterly" => Interval::Quarterly,
        "year" | "yearly" => Interval::Yearly,
        _ => return None,
    })
}

/// What [`summary`] is asked.
#[derive(Debug, Clone, Default)]
pub struct SummaryQuery<'a> {
    /// The entries to count.
    pub filter: Filter<'a>,
    /// One or more groupings, outermost first; none is one total row.
    pub group: Vec<Group>,
    /// Cut accounts to this many segments (`clients:acme`).
    pub depth: Option<usize>,
    /// Keep the largest N rows; the rest are summed as `other`.
    pub top: Option<usize>,
    /// A per-row series by this interval.
    pub series: Option<Interval>,
}

/// One row of a summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SummaryRow {
    /// The key, one part per grouping.
    pub key: Vec<String>,
    /// The time.
    pub quantity: Qty,
    /// How many entries.
    pub entries: usize,
    /// Per period bucket (with `series`): `(bucket, quantity)`, every bucket
    /// of [`Summary::buckets`] in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<(String, Qty)>>,
}

/// A summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// The rows, largest first (with `top`), else by key.
    pub rows: Vec<SummaryRow>,
    /// The rows beyond `top`, summed.
    pub other: Option<SummaryRow>,
    /// Everything counted.
    pub total: Qty,
    /// The series' buckets, in order.
    pub buckets: Vec<String>,
}

fn key_part(book: &TimeBook, entry: &TimeEntry, group: Group, depth: Option<usize>) -> String {
    match group {
        Group::Account => match depth {
            Some(depth) => entry
                .account
                .split(':')
                .take(depth.max(1))
                .collect::<Vec<_>>()
                .join(":"),
            None => entry.account.clone(),
        },
        Group::Branch => entry.account.split(':').next().unwrap_or("").to_string(),
        Group::Customer => query::customer_of(book, entry).unwrap_or("").to_string(),
        Group::Person => entry.person.clone().unwrap_or_default(),
        Group::Billable => if query::is_billable(book, entry) {
            "yes"
        } else {
            "no"
        }
        .to_string(),
        Group::Billed => match super::entries::Billed::of(book, entry) {
            super::entries::Billed::Unbilled => "unbilled",
            super::entries::Billed::Invoiced => "invoiced",
            super::entries::Billed::Nobill => "nobill",
            super::entries::Billed::NotBillable => "not-billable",
        }
        .to_string(),
        Group::Note => entry.note.trim().to_string(),
        Group::Period(interval) => bucket_key(&entry.date, interval),
    }
}

/// Time per group (T-dash), as `query` asks.
#[must_use]
pub fn summary(book: &TimeBook, query: &SummaryQuery<'_>) -> Summary {
    struct Acc {
        total: TimeQty,
        count: usize,
        series: BTreeMap<String, TimeQty>,
    }
    let mut rows: BTreeMap<Vec<String>, Acc> = BTreeMap::new();
    let mut buckets: BTreeSet<String> = BTreeSet::new();
    let mut total = TimeQty::ZERO;
    for (_, entry) in query::entries(book, &query.filter) {
        let key: Vec<String> = query
            .group
            .iter()
            .map(|g| key_part(book, entry, *g, query.depth))
            .collect();
        let acc = rows.entry(key).or_insert_with(|| Acc {
            total: TimeQty::ZERO,
            count: 0,
            series: BTreeMap::new(),
        });
        acc.total = acc.total.plus(entry.quantity);
        acc.count += 1;
        if let Some(interval) = query.series {
            let bucket = bucket_key(&entry.date, interval);
            let slot = acc.series.entry(bucket.clone()).or_default();
            *slot = slot.plus(entry.quantity);
            buckets.insert(bucket);
        }
        total = total.plus(entry.quantity);
    }
    let buckets: Vec<String> = buckets.into_iter().collect();
    let row = |key: Vec<String>, acc: &Acc| SummaryRow {
        key,
        quantity: acc.total.into(),
        entries: acc.count,
        series: query.series.map(|_| {
            buckets
                .iter()
                .map(|b| {
                    (
                        b.clone(),
                        acc.series.get(b).copied().unwrap_or_default().into(),
                    )
                })
                .collect()
        }),
    };
    let mut all: Vec<(Vec<String>, Acc)> = rows.into_iter().collect();
    if query.group.first() == Some(&Group::Account) && query.group.len() == 1 {
        all.sort_by(|a, b| by_segment(&a.0[0], &b.0[0]));
    }
    let (kept, other) = match query.top {
        Some(top) if all.len() > top => {
            all.sort_by(|a, b| b.1.total.cmp(&a.1.total).then_with(|| a.0.cmp(&b.0)));
            let rest = all.split_off(top);
            let mut other = Acc {
                total: TimeQty::ZERO,
                count: 0,
                series: BTreeMap::new(),
            };
            for (_, acc) in &rest {
                other.total = other.total.plus(acc.total);
                other.count += acc.count;
                for (bucket, q) in &acc.series {
                    let slot = other.series.entry(bucket.clone()).or_default();
                    *slot = slot.plus(*q);
                }
            }
            (all, Some(row(vec!["other".to_string()], &other)))
        }
        _ => (all, None),
    };
    Summary {
        rows: kept.into_iter().map(|(key, acc)| row(key, &acc)).collect(),
        other,
        total: total.into(),
        buckets,
    }
}

/// One period to bill (B-unbilled).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeriodView {
    /// First date.
    pub from: String,
    /// Last date (inclusive).
    pub to: String,
    /// The unbilled time in it.
    pub quantity: Qty,
    /// How many entries.
    pub entries: usize,
}

impl From<&SuggestedPeriod> for PeriodView {
    fn from(period: &SuggestedPeriod) -> Self {
        Self {
            from: period.start.clone(),
            to: period.end.clone(),
            quantity: period.batch.quantity.into(),
            entries: period.batch.entries.len(),
        }
    }
}

/// One customer's unbilled time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomerUnbilled {
    /// The `customer:` value the time accounts carry.
    pub customer: String,
    /// The customer note, when one has that name.
    pub title: Option<String>,
    /// The note.
    pub path: Option<String>,
    /// `cadence`, as written.
    pub cadence: Option<String>,
    /// The time.
    pub quantity: Qty,
    /// How many entries.
    pub entries: usize,
    /// The oldest date.
    pub oldest: String,
    /// The newest.
    pub newest: String,
    /// An estimate at each account's item rate, plain decimal, where every
    /// account has a rate (else `None`).
    pub estimate: Option<String>,
    /// The estimate's commodity.
    pub commodity: Option<String>,
    /// The periods to bill, the main suggestion first.
    pub periods: Vec<PeriodView>,
}

/// Billable time with no customer (*Needs a customer*), per account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedsCustomer {
    /// The account.
    pub account: String,
    /// The time.
    pub quantity: Qty,
    /// The oldest date.
    pub oldest: String,
}

/// A line tagged `invoice: N` where no invoice N is in the files or the
/// books: a draft deleted outside the app, or a bill half made. Release it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Orphan {
    /// The number.
    pub invoice: String,
    /// The entries.
    pub entries: Vec<String>,
    /// Their time.
    pub quantity: Qty,
}

/// B-unbilled, as of `today`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unbilled {
    /// The day it was computed for.
    pub today: String,
    /// Per customer.
    pub customers: Vec<CustomerUnbilled>,
    /// Billable time with no customer.
    pub needs_customer: Vec<NeedsCustomer>,
    /// `invoice:` tags naming no invoice.
    pub orphans: Vec<Orphan>,
    /// The banner (*3 customers have unbilled time through October*): the
    /// total of every customer's main suggestion.
    pub suggested_total: Qty,
}

/// The catalog item `code`, from the first catalog that has it.
pub(crate) fn catalog_item<'c>(
    catalogs: &'c [Catalog],
    code: &str,
) -> Option<&'c crate::catalog::CatalogItem> {
    catalogs.iter().find_map(|c| c.item(code))
}

/// The estimated value of `entries` at each account's item rate.
fn estimate(
    book: &TimeBook,
    entries: &[usize],
    customer: Option<&CustomerTime>,
    catalogs: &[Catalog],
    money: &MoneyStyle,
) -> Option<Dec> {
    let mut total = Dec::zero();
    for &at in entries {
        let entry = &book.entries[at];
        let code = customer
            .and_then(|c| c.item_rule(&entry.account, entry.person.as_deref()))
            .map(|rule| rule.item.clone())
            .or_else(|| {
                book.account(&entry.account)
                    .and_then(|a| a.item.as_ref())
                    .map(|r| r.value.clone())
            })?;
        let rate = catalog_item(catalogs, &code)?.rate?;
        let hours = entry.quantity.hours();
        total = total.add(rate.mul(hours).ok()?).ok()?;
    }
    money.round(total).ok()
}

/// What is unbilled, per customer, with the periods to bill (B-unbilled).
#[must_use]
pub fn unbilled(
    book: &TimeBook,
    index: &DocumentIndex,
    known: &BTreeSet<String>,
    today: &str,
) -> Unbilled {
    let report = query::unbilled(book, None);
    let catalogs: Vec<Catalog> = index.catalogs().cloned().collect();
    let money = index.money().style(None);
    let mut suggested = TimeQty::ZERO;
    let customers = report
        .customers
        .iter()
        .map(|(name, batch)| {
            let found = customer_named(index, name).map(|c| customer_time(index.site(), &c));
            let cadence = found.as_ref().map_or(Cadence::None, CustomerTime::cadence);
            let periods: Vec<PeriodView> = query::suggest_periods(book, batch, cadence, today)
                .iter()
                .map(PeriodView::from)
                .collect();
            if let Some(first) = periods.first() {
                suggested = suggested.plus(TimeQty::from_seconds(first.quantity.seconds));
            }
            let value = estimate(book, &batch.entries, found.as_ref(), &catalogs, &money);
            CustomerUnbilled {
                customer: name.clone(),
                title: found.as_ref().map(|c| c.title.clone()),
                path: found.as_ref().map(|c| c.path.to_string()),
                cadence: found.as_ref().and_then(|c| c.cadence.clone()),
                quantity: batch.quantity.into(),
                entries: batch.entries.len(),
                oldest: batch.oldest.clone(),
                newest: batch.newest.clone(),
                estimate: value.map(|v| ledgeline_core::edit::render_dec(v, '.')),
                commodity: value.map(|_| money.commodity().to_string()),
                periods,
            }
        })
        .collect();
    let needs_customer = report
        .needs_customer
        .iter()
        .map(|(account, batch)| NeedsCustomer {
            account: account.clone(),
            quantity: batch.quantity.into(),
            oldest: batch.oldest.clone(),
        })
        .collect();
    let mut orphaned: BTreeMap<&str, Vec<&TimeEntry>> = BTreeMap::new();
    for entry in &book.entries {
        if let Some(number) = entry.tag(&book.tags, TagRole::Invoice)
            && !known.contains(number)
        {
            orphaned.entry(number).or_default().push(entry);
        }
    }
    Unbilled {
        today: today.to_string(),
        customers,
        needs_customer,
        orphans: orphaned
            .into_iter()
            .map(|(number, entries)| Orphan {
                invoice: number.to_string(),
                quantity: TimeQty::sum(entries.iter().map(|e| e.quantity)).into(),
                entries: entries.iter().map(|e| entry_id(e)).collect(),
            })
            .collect(),
        suggested_total: suggested.into(),
    }
}

/// One person's time with no `bill` tag (B-ap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonUnpaid {
    /// The person (`""`: unattributed, which can't be paid until assigned).
    pub person: String,
    /// Their timekeeper note, when one links them.
    pub note: Option<String>,
    /// Its title.
    pub title: Option<String>,
    /// The time.
    pub quantity: Qty,
    /// How many entries.
    pub entries: usize,
    /// The oldest date.
    pub oldest: String,
    /// The newest.
    pub newest: String,
    /// The periods to pay, as for a customer: the note's `cadence`, else
    /// everything through last month.
    pub periods: Vec<PeriodView>,
}

/// Time to pay, per person (B-ap). Only people with a timekeeper note are
/// listed unless `everyone`: the owner's own time is not a payable.
#[must_use]
pub fn unpaid(
    book: &TimeBook,
    index: &DocumentIndex,
    today: &str,
    everyone: bool,
) -> Vec<PersonUnpaid> {
    query::unpaid(book, None)
        .into_iter()
        .filter_map(|(person, batch): (String, Batch)| {
            let keeper = (!person.is_empty())
                .then(|| timekeeper_for(index.site(), &person))
                .flatten();
            if keeper.is_none() && !everyone {
                return None;
            }
            let cadence = keeper
                .as_ref()
                .and_then(|k| index.site().note(&k.path))
                .and_then(|n| super::link::field_text(n.frontmatter.get("cadence")));
            let periods =
                query::suggest_periods(book, &batch, Cadence::parse(cadence.as_deref()), today)
                    .iter()
                    .map(PeriodView::from)
                    .collect();
            Some(PersonUnpaid {
                note: keeper.as_ref().map(|k| k.path.to_string()),
                title: keeper.as_ref().map(|k| k.title.clone()),
                quantity: batch.quantity.into(),
                entries: batch.entries.len(),
                oldest: batch.oldest.clone(),
                newest: batch.newest.clone(),
                periods,
                person,
            })
        })
        .collect()
}
