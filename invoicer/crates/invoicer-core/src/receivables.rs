//! What `receivables` shows (ARCHITECTURE §4.6): open invoices, aging,
//! days sales outstanding and average days to pay, plus drafts waiting.
//!
//! - **Accrual books** read the books, through Ledgeline's
//!   [`Receivables`](ledgeline_core::accrual::receivables::Receivables):
//!   every receivable document still owed (an invoice the invoicer booked,
//!   or any other: plan 45's, an opening balance), its settlement N:1, and
//!   Ledgeline's own aging, DSO and days to pay, so both apps show the same
//!   numbers. Payments not linked to any invoice are listed as unapplied.
//! - **Cash books** have no receivable: an invoice is owed by its markdown
//!   (sent, not void) less the payments applied to it in the books
//!   ([`BookStates::paid`]). Those rows are labelled `markdown`, and aging,
//!   DSO and days to pay are computed the same way from them.
//!
//! Buckets are Ledgeline's ([`AgingBucket`]): Current, 1–30, 31–60, 61–90,
//! 91+ days past due.

use crate::books::{Basis, dec_text};
use crate::doc::Status;
use crate::index::{DocumentIndex, ListFilter};
use crate::reconcile::BookStates;
use ledgeline_core::Dec;
use ledgeline_core::accrual::AccrualRole;
use ledgeline_core::accrual::receivables::{AgingBucket, DocumentStatus};
use ledgeline_core::reports::periods::{add_days, days_between};
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Where an open row comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenSource {
    /// A receivable in the books (accrual).
    Books,
    /// A sent invoice's markdown, less its applied payments (cash).
    Markdown,
}

/// One open invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenItem {
    /// Its number (`None`: a receivable booked without one).
    pub number: Option<String>,
    /// Its file, when there is one.
    pub path: Option<ledgeline_core::vault::VaultPath>,
    /// Its customer.
    pub customer: Option<String>,
    /// The receivable account (accrual).
    pub account: Option<String>,
    /// The commodity.
    pub commodity: String,
    /// Its date.
    pub date: String,
    /// When it is due.
    pub due: String,
    /// Its amount.
    #[serde(serialize_with = "dec_text")]
    pub amount: Dec,
    /// What is still owed.
    #[serde(serialize_with = "dec_text")]
    pub outstanding: Dec,
    /// Days past due (negative before the due date).
    pub days_past_due: i64,
    /// Its aging bucket's label.
    pub bucket: &'static str,
    /// Books or markdown.
    pub source: OpenSource,
}

/// One aging bucket's sum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BucketView {
    /// `Current`, `1–30`, `31–60`, `61–90`, `91+`.
    pub label: &'static str,
    /// What is owed in it.
    #[serde(serialize_with = "dec_text")]
    pub amount: Dec,
    /// Invoices in it.
    pub count: usize,
}

/// Aging for one account (accrual) and commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgingView {
    /// The receivable account (`None` in cash books).
    pub account: Option<String>,
    /// The commodity.
    pub commodity: String,
    /// The buckets, youngest first.
    pub buckets: Vec<BucketView>,
    /// Their sum.
    #[serde(serialize_with = "dec_text")]
    pub total: Dec,
    /// The oldest due date among the overdue.
    pub oldest_overdue: Option<String>,
}

/// Days sales outstanding in one commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DsoView {
    /// The commodity.
    pub commodity: String,
    /// Outstanding at the end of today.
    #[serde(serialize_with = "dec_text")]
    pub outstanding: Dec,
    /// Billed in the window.
    #[serde(serialize_with = "dec_text")]
    pub billed: Dec,
    /// The window, in days.
    pub days: u32,
    /// `outstanding / billed × days`, to one decimal; `None` with nothing
    /// billed.
    pub value: Option<String>,
}

/// Average days from invoice to payment in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DaysToPayView {
    /// Invoices paid in full that it averages.
    pub invoices: usize,
    /// The mean, to one decimal.
    pub average: Option<String>,
}

/// A payment in the books not linked to any invoice (accrual).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnappliedView {
    /// Its date.
    pub date: String,
    /// The receivable account.
    pub account: String,
    /// The commodity.
    pub commodity: String,
    /// What it holds.
    #[serde(serialize_with = "dec_text")]
    pub amount: Dec,
    /// The `invoice:` its transaction names, if any.
    pub number: Option<String>,
    /// Its customer.
    pub customer: Option<String>,
}

/// A draft waiting to be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftItem {
    /// Its number.
    pub number: Option<String>,
    /// Its file.
    pub path: ledgeline_core::vault::VaultPath,
    /// Its customer, as billed.
    pub customer: Option<String>,
    /// Its date.
    pub date: Option<String>,
    /// Its total, as the table writes it.
    pub total: Option<String>,
}

/// `receivables`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceivablesView {
    /// The books' basis.
    pub basis: Basis,
    /// What "overdue" is measured against.
    pub today: String,
    /// Every invoice still owed, oldest due first.
    pub open: Vec<OpenItem>,
    /// Aging per account and commodity.
    pub aging: Vec<AgingView>,
    /// DSO per commodity.
    pub dso: Vec<DsoView>,
    /// Average days to pay.
    pub average_days_to_pay: DaysToPayView,
    /// Payments not linked to an invoice (accrual).
    pub unapplied: Vec<UnappliedView>,
    /// Drafts waiting to be sent, newest first.
    pub drafts: Vec<DraftItem>,
}

fn dec_label(value: Dec) -> String {
    ledgeline_core::edit::render_dec(value, '.')
}

fn bucket_of(days: i64) -> AgingBucket {
    AgingBucket::of(i32::try_from(days).unwrap_or(if days < 0 { i32::MIN } else { i32::MAX }))
}

fn aging_of(account: Option<String>, commodity: String, items: &[&OpenItem]) -> AgingView {
    let mut buckets: Vec<BucketView> = AgingBucket::ALL
        .iter()
        .map(|b| BucketView {
            label: b.label(),
            amount: Dec::zero(),
            count: 0,
        })
        .collect();
    let mut total = Dec::zero();
    let mut oldest: Option<String> = None;
    for item in items {
        let slot = AgingBucket::ALL
            .iter()
            .position(|b| *b == bucket_of(item.days_past_due))
            .unwrap_or(0);
        buckets[slot].amount = buckets[slot]
            .amount
            .add(item.outstanding)
            .unwrap_or(buckets[slot].amount);
        buckets[slot].count += 1;
        total = total.add(item.outstanding).unwrap_or(total);
        if item.days_past_due > 0 && oldest.as_deref().is_none_or(|o| item.due.as_str() < o) {
            oldest = Some(item.due.clone());
        }
    }
    AgingView {
        account,
        commodity,
        buckets,
        total,
        oldest_overdue: oldest,
    }
}

/// `a / b`, to one decimal, half away from zero.
fn ratio(a: Dec, b: Dec) -> Option<String> {
    if b.is_zero() {
        return None;
    }
    // In tenths: a × 10 / b, rounded.
    let scale = |d: Dec, p: u32| d.mantissa.checked_mul(10_i128.checked_pow(p)?);
    let places = a.places.max(b.places);
    let top = scale(a, places - a.places)?.checked_mul(10)?;
    let bottom = scale(b, places - b.places)?;
    let (q, r) = (top / bottom, top % bottom);
    let q = if r.unsigned_abs() * 2 >= bottom.unsigned_abs() {
        q + if (top < 0) == (bottom < 0) { 1 } else { -1 }
    } else {
        q
    };
    Some(dec_label(Dec::new(q, 1)))
}

/// Each number filed by exactly one document, and its file: one pass over
/// the index, so finding an open item's file is a lookup, not a scan.
fn files_by_number(index: &DocumentIndex) -> HashMap<&str, Option<&VaultPath>> {
    let mut out: HashMap<&str, Option<&VaultPath>> = HashMap::new();
    for doc in index.docs() {
        if let Some(number) = doc.number() {
            out.entry(number)
                // A second document with the number: no single file.
                .and_modify(|path| *path = None)
                .or_insert(Some(&doc.path));
        }
    }
    out
}

fn from_books(
    files: &HashMap<&str, Option<&VaultPath>>,
    d: &DocumentStatus,
    today: &str,
) -> OpenItem {
    let days = days_between(&d.due, today);
    OpenItem {
        number: d.number.clone(),
        path: d
            .number
            .as_deref()
            .and_then(|n| files.get(n.trim()).copied().flatten())
            .cloned(),
        customer: d.party.as_ref().map(|p| p.name.clone()),
        account: Some(d.account.clone()),
        commodity: d.commodity.clone(),
        date: d.date.clone(),
        due: d.due.clone(),
        amount: d.amount,
        outstanding: d.outstanding,
        days_past_due: days,
        bucket: bucket_of(days).label(),
        source: OpenSource::Books,
    }
}

/// `receivables` as of `today`, with DSO over the `dso_days` ending today.
#[must_use]
pub fn receivables(
    index: &DocumentIndex,
    states: &BookStates,
    today: &str,
    dso_days: u32,
) -> ReceivablesView {
    let basis = states.basis();
    let mut view = ReceivablesView {
        basis,
        today: today.to_string(),
        open: Vec::new(),
        aging: Vec::new(),
        dso: Vec::new(),
        average_days_to_pay: DaysToPayView {
            invoices: 0,
            average: None,
        },
        unapplied: Vec::new(),
        drafts: index
            .list(&ListFilter {
                status: Some(Status::Draft),
                ..ListFilter::default()
            })
            .into_iter()
            .map(|d| DraftItem {
                number: d.number().map(str::to_string),
                path: d.path.clone(),
                customer: d.header.invoicee.name.clone(),
                date: d.header.date.clone(),
                total: d.summary.as_ref().map(|s| s.total_text.clone()),
            })
            .collect(),
    };
    match basis {
        Basis::Accrual => {
            let books = states.receivables();
            let files = files_by_number(index);
            view.open = books
                .open(AccrualRole::Receivable)
                .map(|d| from_books(&files, d, today))
                .collect();
            view.aging = books
                .aging(AccrualRole::Receivable, today)
                .into_iter()
                .map(|row| AgingView {
                    account: Some(row.account),
                    commodity: row.commodity,
                    buckets: AgingBucket::ALL
                        .iter()
                        .enumerate()
                        .map(|(at, b)| BucketView {
                            label: b.label(),
                            amount: row.buckets[at],
                            count: row.counts[at],
                        })
                        .collect(),
                    total: row.total,
                    oldest_overdue: row.oldest_overdue,
                })
                .collect();
            view.dso = books
                .dso(AccrualRole::Receivable, today, dso_days)
                .into_iter()
                .map(|d| DsoView {
                    commodity: d.commodity,
                    outstanding: d.outstanding,
                    billed: d.billed,
                    days: d.days,
                    value: d.value.map(dec_label),
                })
                .collect();
            let paid = books.average_days_to_pay(AccrualRole::Receivable);
            view.average_days_to_pay = DaysToPayView {
                invoices: paid.documents,
                average: paid.average.map(dec_label),
            };
            view.unapplied = books
                .unapplied()
                .iter()
                .filter(|u| u.role == AccrualRole::Receivable)
                .map(|u| UnappliedView {
                    date: u.date.clone(),
                    account: u.account.clone(),
                    commodity: u.commodity.clone(),
                    amount: u.amount,
                    number: u.number.clone(),
                    customer: u.party.as_ref().map(|p| p.name.clone()),
                })
                .collect();
        }
        Basis::Cash => cash(index, states, today, dso_days, &mut view),
    }
    view.open.sort_by(|a, b| {
        (a.due.as_str(), a.date.as_str(), a.number.as_deref()).cmp(&(
            b.due.as_str(),
            b.date.as_str(),
            b.number.as_deref(),
        ))
    });
    view
}

/// Cash books: owed from the markdown, less the payments applied.
fn cash(
    index: &DocumentIndex,
    states: &BookStates,
    today: &str,
    dso_days: u32,
    view: &mut ReceivablesView,
) {
    let window_start = add_days(today, -i64::from(dso_days));
    let mut dso: BTreeMap<String, (Dec, Dec)> = BTreeMap::new();
    let (mut paid_count, mut paid_days) = (0usize, 0i64);
    for doc in index.docs() {
        let (Some(number), Some(summary), Some(date)) = (
            doc.number(),
            doc.summary.as_ref(),
            doc.header.date.as_deref(),
        ) else {
            continue;
        };
        if doc.header.status != Some(Status::Sent) || date > today {
            continue;
        }
        let Some(paid) = states.paid(doc) else {
            continue;
        };
        let outstanding = if paid.is_full() {
            Dec::zero()
        } else {
            summary.total.sub(paid.paid()).unwrap_or(summary.total)
        };
        let (owed, billed) = dso
            .entry(summary.commodity.clone())
            .or_insert((Dec::zero(), Dec::zero()));
        *owed = owed.add(outstanding).unwrap_or(*owed);
        if date > window_start.as_str() {
            *billed = billed.add(summary.total).unwrap_or(*billed);
        }
        if let crate::reconcile::BooksPaid::Full { date: on } = &paid {
            if !summary.total.is_zero() {
                paid_count += 1;
                paid_days += days_between(date, on);
            }
            continue;
        }
        let due = doc
            .header
            .due_date
            .clone()
            .unwrap_or_else(|| date.to_string());
        let days = days_between(&due, today);
        view.open.push(OpenItem {
            number: Some(number.to_string()),
            path: Some(doc.path.clone()),
            customer: doc.header.invoicee.name.clone(),
            account: None,
            commodity: summary.commodity.clone(),
            date: date.to_string(),
            due,
            amount: summary.total,
            outstanding,
            days_past_due: days,
            bucket: bucket_of(days).label(),
            source: OpenSource::Markdown,
        });
    }
    let mut by_commodity: BTreeMap<&str, Vec<&OpenItem>> = BTreeMap::new();
    for item in &view.open {
        by_commodity
            .entry(item.commodity.as_str())
            .or_default()
            .push(item);
    }
    view.aging = by_commodity
        .into_iter()
        .map(|(commodity, items)| aging_of(None, commodity.to_string(), &items))
        .collect();
    view.dso = dso
        .into_iter()
        .map(|(commodity, (outstanding, billed))| DsoView {
            commodity,
            outstanding,
            billed,
            days: dso_days,
            value: outstanding
                .mul(Dec::new(i128::from(dso_days), 0))
                .ok()
                .and_then(|scaled| ratio(scaled, billed)),
        })
        .collect();
    view.average_days_to_pay = DaysToPayView {
        invoices: paid_count,
        average: i128::try_from(paid_count)
            .ok()
            .and_then(|n| ratio(Dec::new(i128::from(paid_days), 0), Dec::new(n, 0))),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_rounds_to_tenths() {
        assert_eq!(
            ratio(Dec::new(10, 0), Dec::new(3, 0)).as_deref(),
            Some("3.3")
        );
        assert_eq!(
            ratio(Dec::new(5, 0), Dec::new(4, 0)).as_deref(),
            Some("1.3")
        );
        assert_eq!(
            ratio(Dec::new(150_000, 2), Dec::new(4, 0)).as_deref(),
            Some("375.0")
        );
        assert_eq!(ratio(Dec::new(1, 0), Dec::zero()), None);
    }
}
