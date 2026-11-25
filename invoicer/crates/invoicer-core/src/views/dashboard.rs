//! The receivables dashboard (invoicer PLAN Phase 5a): what is owed and how
//! late, in one read of the index and the books, for `GET /api/dashboard`.
//!
//! Everything comes from [`receivables`](crate::receivables::receivables),
//! so the dashboard, `receivables` and a customer's account never disagree:
//! its open items, DSO and average days to pay. On top of that:
//!
//! - **The outstanding bar**, per commodity: overdue (past due) and open
//!   (not yet due), drawn against a scale of the total or
//!   [`SCALE_MINIMUM`], whichever is more, so a small balance reads small.
//! - **Aging buckets** with configurable edges (`[invoicer] aging-buckets`,
//!   default Ledgeline's Current, 1–30, 31–60, 61–90, 91+ days past due).
//!   Each bucket has a URL-safe `key` (`current`, `1-30`, `91-`) the SPA
//!   puts in the invoice list's `?aged=` filter.
//! - **The oldest overdue** item: earliest due date, then date, then number.
//! - **DSO** (days sales outstanding) over `[invoicer] dso-days` (90):
//!   what is owed at the end of today over what was billed in the window,
//!   times the window's days (Ledgeline's definition, a voided invoice in
//!   neither). **Average days to pay**: the mean days from an invoice's
//!   date to its payment in full, over every invoice paid in full.
//! - **Drafts waiting** to be sent: a count and their totals.
//! - **Recognition due** (plan 45 schedules, when the books have a
//!   recognition file): the entries Ledgeline's **Post through ‹today›**
//!   would post, and those due in closed periods it leaves to a catch-up.
//!
//! Only positive amounts still owed count: an item at zero or below (a
//! credit) is left out of the bar and the buckets.

use crate::books::{Basis, dec_text};
use crate::doc::{DocType, Status};
use crate::index::{DocumentIndex, IndexedDoc};
use crate::receivables::{self, OpenItem};
use crate::reconcile::BookStates;
use crate::session::Session;
use crate::settings::Settings;
use ledgeline_core::Dec;
use ledgeline_core::accrual::due::DueSet;
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::BTreeMap;

/// The least the outstanding bar's scale reaches, in the commodity's
/// units ($10,000): below it the bar's grey remainder shows how small the
/// balance is.
pub const SCALE_MINIMUM: i128 = 10_000;

/// What the dashboard is computed with: `[invoicer]`'s `aging-buckets` and
/// `dso-days`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardOptions {
    /// The last day past due of each bucket after Current, increasing.
    pub aging_edges: Vec<u32>,
    /// The DSO window, in days.
    pub dso_days: u32,
}

impl DashboardOptions {
    /// The options `settings` give.
    #[must_use]
    pub fn of(settings: &Settings) -> Self {
        Self {
            aging_edges: settings.aging_buckets.clone(),
            dso_days: settings.dso_days,
        }
    }
}

/// An amount, as a number (for drawing) and as the journal writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AmountView {
    /// Plain decimal text (`8350.00`).
    #[serde(serialize_with = "dec_text")]
    pub amount: Dec,
    /// In the journal's style (`$8,350.00`).
    pub text: String,
}

/// A part of what is owed, and how many items make it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartView {
    /// Plain decimal text.
    #[serde(serialize_with = "dec_text")]
    pub amount: Dec,
    /// In the journal's style.
    pub text: String,
    /// Items in it.
    pub count: usize,
}

/// One aging bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BucketView {
    /// For URLs: `current`, `1-30`, `91-` (no upper end).
    pub key: String,
    /// For people: `Current`, `1–30`, `91+`.
    pub label: String,
    /// The first day past due in it (`None`: Current, not yet past due).
    pub from: Option<u32>,
    /// The last day past due in it (`None`: no upper end, or Current).
    pub to: Option<u32>,
    /// Plain decimal text.
    #[serde(serialize_with = "dec_text")]
    pub amount: Dec,
    /// In the journal's style.
    pub text: String,
    /// Items in it.
    pub count: usize,
}

/// The overdue item due longest ago.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OldestView {
    /// Its number (`None`: a receivable booked without one).
    pub number: Option<String>,
    /// Its file, when there is one.
    pub path: Option<VaultPath>,
    /// Its customer.
    pub customer: Option<String>,
    /// When it was due.
    pub due: String,
    /// Days past due on `today`.
    pub days_past_due: i64,
    /// What is still owed, in the journal's style.
    pub outstanding: String,
}

/// Days sales outstanding in one commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DsoView {
    /// `outstanding / billed × days`, to one decimal; `None` with nothing
    /// billed in the window.
    pub value: Option<String>,
    /// The window, in days.
    pub days: u32,
    /// Owed at the end of today, in the journal's style.
    pub outstanding: String,
    /// Billed in the window, in the journal's style.
    pub billed: String,
}

/// What is owed in one commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommodityView {
    /// The commodity (`$`).
    pub commodity: String,
    /// Everything owed: overdue plus not yet due.
    pub outstanding: PartView,
    /// Past due on `today` (red).
    pub overdue: PartView,
    /// Owed, not yet due (yellow).
    pub not_due: PartView,
    /// The part of `outstanding` with no invoice file in the books folder:
    /// receivables booked in the journal only (an opening balance, an
    /// invoice Ledgeline booked). They count here, as in Ledgeline, but the
    /// invoice list can't show them.
    pub unfiled: PartView,
    /// The bar's full width: `outstanding`, or [`SCALE_MINIMUM`] when that
    /// is more.
    pub scale: AmountView,
    /// The aging buckets, Current first.
    pub buckets: Vec<BucketView>,
    /// The overdue item due longest ago.
    pub oldest_overdue: Option<OldestView>,
    /// Days sales outstanding.
    pub dso: Option<DsoView>,
}

/// Average days from invoice to payment in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DaysToPayView {
    /// Invoices paid in full that it averages.
    pub invoices: usize,
    /// The mean, to one decimal; `None` when none are paid.
    pub average: Option<String>,
}

/// Drafts waiting to be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftsView {
    /// How many.
    pub count: usize,
    /// Their totals, per commodity (drafts without a table add nothing).
    pub totals: Vec<AmountView>,
    /// The oldest draft's date.
    pub oldest: Option<String>,
}

/// Recognition entries due (plan 45), as Ledgeline counts them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecognitionView {
    /// Due on or before `today` in open periods: what **Post through
    /// ‹today›** posts (customers' and vendors' schedules alike).
    pub due: usize,
    /// Due in closed periods: left for Ledgeline's catch-up adjustment.
    pub closed: usize,
    /// The earliest due date among `due`.
    pub earliest: Option<String>,
}

/// The dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dashboard {
    /// The books' basis.
    pub basis: Basis,
    /// What "overdue" is measured against.
    pub today: String,
    /// Invoices in the books folder (every status): zero shows the empty
    /// state.
    pub invoices: usize,
    /// What is owed, per commodity: the books' default commodity first,
    /// then the most owed. Empty when nothing is.
    pub commodities: Vec<CommodityView>,
    /// Average days to pay.
    pub average_days_to_pay: DaysToPayView,
    /// Drafts waiting.
    pub drafts: DraftsView,
    /// Recognition due; `None` when the books have no recognition file (or
    /// it can't be read: `check` says why).
    pub recognition: Option<RecognitionView>,
    /// The bucket edges and DSO window used (the settings).
    pub aging_buckets: Vec<u32>,
    /// The DSO window used.
    pub dso_days: u32,
}

/// The buckets for `edges` (empty, at `Dec::zero()`): Current, then one
/// per edge, then the open-ended last.
fn empty_buckets(edges: &[u32]) -> Vec<BucketView> {
    let blank = |key: String, label: String, from: Option<u32>, to: Option<u32>| BucketView {
        key,
        label,
        from,
        to,
        amount: Dec::zero(),
        text: String::new(),
        count: 0,
    };
    let mut out = vec![blank("current".into(), "Current".into(), None, None)];
    let mut from = 1u32;
    for &edge in edges {
        out.push(blank(
            format!("{from}-{edge}"),
            format!("{from}–{edge}"),
            Some(from),
            Some(edge),
        ));
        from = edge.saturating_add(1);
    }
    out.push(blank(
        format!("{from}-"),
        format!("{from}+"),
        Some(from),
        None,
    ));
    out
}

/// The bucket index for `days` past due under `edges`.
fn bucket_index(edges: &[u32], days: i64) -> usize {
    if days <= 0 {
        return 0;
    }
    1 + edges
        .iter()
        .position(|&edge| days <= i64::from(edge))
        .unwrap_or(edges.len())
}

fn add(sum: &mut Dec, value: Dec) {
    *sum = sum.add(value).unwrap_or(*sum);
}

fn part(amount: Dec, count: usize, format: &dyn Fn(Dec) -> String) -> PartView {
    PartView {
        amount,
        text: format(amount),
        count,
    }
}

/// One commodity's bar, buckets and oldest overdue, from its open items
/// (oldest due first, as `receivables` sorts them).
fn commodity_view(
    commodity: &str,
    items: &[&OpenItem],
    edges: &[u32],
    index: &DocumentIndex,
) -> CommodityView {
    let style = index.money().style(Some(commodity));
    let format = |d: Dec| style.format(d);
    let mut buckets = empty_buckets(edges);
    let (mut overdue, mut overdue_n) = (Dec::zero(), 0usize);
    let (mut not_due, mut not_due_n) = (Dec::zero(), 0usize);
    let (mut unfiled, mut unfiled_n) = (Dec::zero(), 0usize);
    let mut oldest: Option<&OpenItem> = None;
    for item in items {
        if item.path.is_none() {
            add(&mut unfiled, item.outstanding);
            unfiled_n += 1;
        }
        let slot = &mut buckets[bucket_index(edges, item.days_past_due)];
        add(&mut slot.amount, item.outstanding);
        slot.count += 1;
        if item.days_past_due > 0 {
            add(&mut overdue, item.outstanding);
            overdue_n += 1;
            let key = |i: &OpenItem| (i.due.clone(), i.date.clone(), i.number.clone());
            if oldest.is_none_or(|o| key(item) < key(o)) {
                oldest = Some(item);
            }
        } else {
            add(&mut not_due, item.outstanding);
            not_due_n += 1;
        }
    }
    for bucket in &mut buckets {
        bucket.text = format(bucket.amount);
    }
    let mut total = overdue;
    add(&mut total, not_due);
    let minimum = Dec::new(SCALE_MINIMUM, 0);
    let scale = total.max(minimum);
    CommodityView {
        commodity: commodity.to_string(),
        outstanding: part(total, overdue_n + not_due_n, &format),
        overdue: part(overdue, overdue_n, &format),
        not_due: part(not_due, not_due_n, &format),
        unfiled: part(unfiled, unfiled_n, &format),
        scale: AmountView {
            amount: scale,
            text: style.format_compact(scale),
        },
        buckets,
        oldest_overdue: oldest.map(|o| OldestView {
            number: o.number.clone(),
            path: o.path.clone(),
            customer: o.customer.clone(),
            due: o.due.clone(),
            days_past_due: o.days_past_due,
            outstanding: format(o.outstanding),
        }),
        dso: None,
    }
}

fn is_invoice(doc: &IndexedDoc) -> bool {
    doc.header.doc_type.unwrap_or(DocType::Invoice) == DocType::Invoice
}

fn drafts(index: &DocumentIndex) -> DraftsView {
    let mut totals: BTreeMap<&str, Dec> = BTreeMap::new();
    let (mut count, mut oldest): (usize, Option<&str>) = (0, None);
    for doc in index
        .docs()
        .filter(|d| is_invoice(d) && d.header.status == Some(Status::Draft))
    {
        count += 1;
        if let Some(summary) = &doc.summary {
            add(
                totals
                    .entry(summary.commodity.as_str())
                    .or_insert_with(Dec::zero),
                summary.total,
            );
        }
        if let Some(date) = doc.header.date.as_deref()
            && oldest.is_none_or(|o| date < o)
        {
            oldest = Some(date);
        }
    }
    DraftsView {
        count,
        totals: totals
            .into_iter()
            .map(|(commodity, amount)| AmountView {
                amount,
                text: index.money().style(Some(commodity)).format(amount),
            })
            .collect(),
        oldest: oldest.map(str::to_string),
    }
}

/// What is due for recognition in `due` (built under `today`).
#[must_use]
pub fn recognition_view(due: &DueSet, today: &str) -> RecognitionView {
    let open = due.through(today);
    RecognitionView {
        due: open.len(),
        closed: due.closed.len(),
        earliest: open.iter().map(|e| e.recognized.clone()).min(),
    }
}

/// The dashboard on `today`, from the index, the books' states and the
/// recognition due (`None`: the books have no recognition file).
#[must_use]
pub fn dashboard(
    index: &DocumentIndex,
    states: &BookStates,
    recognition: Option<RecognitionView>,
    today: &str,
    options: &DashboardOptions,
) -> Dashboard {
    let view = receivables::receivables(index, states, today, options.dso_days);
    let mut by_commodity: BTreeMap<&str, Vec<&OpenItem>> = BTreeMap::new();
    for item in view.open.iter().filter(|i| i.outstanding > Dec::zero()) {
        by_commodity
            .entry(item.commodity.as_str())
            .or_default()
            .push(item);
    }
    let mut commodities: Vec<CommodityView> = by_commodity
        .iter()
        .map(|(commodity, items)| commodity_view(commodity, items, &options.aging_edges, index))
        .collect();
    for dso in &view.dso {
        let style = index.money().style(Some(&dso.commodity));
        let dso_view = DsoView {
            value: dso.value.clone(),
            days: dso.days,
            outstanding: style.format(dso.outstanding),
            billed: style.format(dso.billed),
        };
        match commodities
            .iter_mut()
            .find(|c| c.commodity == dso.commodity)
        {
            Some(c) => c.dso = Some(dso_view),
            // Billed in the window, nothing owed now: DSO 0, an empty bar.
            None => {
                let mut empty = commodity_view(&dso.commodity, &[], &options.aging_edges, index);
                empty.dso = Some(dso_view);
                commodities.push(empty);
            }
        }
    }
    let default = index.money().style(None).commodity().to_string();
    commodities.sort_by(|a, b| {
        (b.commodity == default)
            .cmp(&(a.commodity == default))
            .then(b.outstanding.amount.cmp(&a.outstanding.amount))
            .then(a.commodity.cmp(&b.commodity))
    });
    Dashboard {
        basis: view.basis,
        today: today.to_string(),
        invoices: index.docs().filter(|d| is_invoice(d)).count(),
        commodities,
        average_days_to_pay: DaysToPayView {
            invoices: view.average_days_to_pay.invoices,
            average: view.average_days_to_pay.average,
        },
        drafts: drafts(index),
        recognition,
        aging_buckets: options.aging_edges.clone(),
        dso_days: options.dso_days,
    }
}

/// [`dashboard`] for an open session: its settings, and the recognition
/// due when the books have a recognition file.
#[must_use]
pub fn dashboard_for(session: &Session, today: &str) -> Dashboard {
    let recognition = session
        .loaded()
        .ok()
        .filter(ledgeline_core::accrual::ops::Loaded::recognition_exists)
        .map(|loaded| recognition_view(&loaded.due(today), today));
    dashboard(
        session.index(),
        session.book_states(),
        recognition,
        today,
        &DashboardOptions::of(session.settings()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_buckets_are_ledgelines() {
        let buckets = empty_buckets(&[30, 60, 90]);
        let keys: Vec<&str> = buckets.iter().map(|b| b.key.as_str()).collect();
        let labels: Vec<&str> = buckets.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(keys, ["current", "1-30", "31-60", "61-90", "91-"]);
        assert_eq!(labels, ["Current", "1–30", "31–60", "61–90", "91+"]);
        assert_eq!((buckets[1].from, buckets[1].to), (Some(1), Some(30)));
        assert_eq!((buckets[4].from, buckets[4].to), (Some(91), None));
        assert_eq!((buckets[0].from, buckets[0].to), (None, None));
    }

    #[test]
    fn custom_edges_make_their_own_buckets() {
        let keys: Vec<String> = empty_buckets(&[15, 45])
            .into_iter()
            .map(|b| b.key)
            .collect();
        assert_eq!(keys, ["current", "1-15", "16-45", "46-"]);
        let keys: Vec<String> = empty_buckets(&[7]).into_iter().map(|b| b.key).collect();
        assert_eq!(keys, ["current", "1-7", "8-"]);
    }

    #[test]
    fn days_past_due_land_in_their_bucket() {
        let edges = [30, 60, 90];
        for (days, slot) in [
            (-10, 0),
            (0, 0),
            (1, 1),
            (30, 1),
            (31, 2),
            (60, 2),
            (61, 3),
            (90, 3),
            (91, 4),
            (10_000, 4),
        ] {
            assert_eq!(bucket_index(&edges, days), slot, "{days} days");
        }
    }
}
