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
//!   Each bucket has a URL-safe `key` (`current`, `1-30`, `91+`) the SPA
//!   puts in the invoice list's `?aged=` filter.
//! - **The oldest overdue** item: earliest due date, then date, then number.
//! - **DSO** (days sales outstanding) over `[invoicer] dso-days` (90):
//!   what is owed at the end of today over what was billed in the window,
//!   times the window's days (Ledgeline's definition, a voided invoice in
//!   neither). **Average days to pay**: the mean days from an invoice's
//!   date to its payment in full, over every invoice paid in full.
//! - **Drafts waiting** to be sent: a count and their totals.
//! - **Paid in the last N days** (`[invoicer] paid-window-days`, 30; the
//!   page picks another): the money received in the window, by payment
//!   date, per commodity, and how many invoices it paid into ([`PaidView`]).
//! - **Recently updated invoices**: the latest few by their last event
//!   (edited, sent, paid, voided), each with where it stands
//!   ([`RecentView`]).
//! - **Recognition due** (plan 45 schedules, when the books have a
//!   recognition file): the entries Ledgeline's **Post through ‹today›**
//!   would post, and those due in closed periods it leaves to a catch-up.
//!
//! Only positive amounts still owed count: an item at zero or below (a
//! credit) is left out of the bar and the buckets.

use crate::books::{Basis, dec_text};
use crate::doc::{DocType, Status};
use crate::index::{DocumentIndex, IndexedDoc};
use crate::receivables::{self, OpenItem, files_by_number};
use crate::reconcile::BookStates;
use crate::session::Session;
use crate::settings::Settings;
use ledgeline_core::Dec;
use ledgeline_core::accrual::AccrualRole;
use ledgeline_core::accrual::due::DueSet;
use ledgeline_core::accrual::receivables::SettlementKind;
use ledgeline_core::reports::periods::{add_days, iso_from_days};
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::SystemTime;

/// The least the outstanding bar's scale reaches, in the commodity's
/// units ($10,000): below it the bar's grey remainder shows how small the
/// balance is.
pub const SCALE_MINIMUM: i128 = 10_000;

/// How many recently updated invoices the dashboard shows.
pub const RECENT_COUNT: usize = 6;

/// What the dashboard is computed with: `[invoicer]`'s `aging-buckets`,
/// `dso-days` and `paid-window-days`, and the local clock's offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardOptions {
    /// The last day past due of each bucket after Current, increasing.
    pub aging_edges: Vec<u32>,
    /// The DSO window, in days.
    pub dso_days: u32,
    /// The "paid in the last N days" window, in days (at least 1).
    pub paid_days: u32,
    /// The setting's window (`paid-window-days`), which the page offers as
    /// its default.
    pub paid_window_days: u32,
    /// Seconds east of UTC: a file's modification time is dated in local
    /// time, as `today` is.
    pub utc_offset: i64,
}

impl DashboardOptions {
    /// The options `settings` give (UTC; the app passes its own offset).
    #[must_use]
    pub fn of(settings: &Settings) -> Self {
        Self {
            aging_edges: settings.aging_buckets.clone(),
            dso_days: settings.dso_days,
            paid_days: settings.paid_window_days,
            paid_window_days: settings.paid_window_days,
            utc_offset: 0,
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

/// Money received in one commodity in the paid window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaidCommodityView {
    /// The commodity.
    pub commodity: String,
    /// Plain decimal text.
    #[serde(serialize_with = "dec_text")]
    pub amount: Dec,
    /// In the journal's style.
    pub text: String,
    /// Invoices (and other receivables) it paid into.
    pub count: usize,
    /// Of those, booked without an invoice file here (the list can't show
    /// them).
    pub unfiled: usize,
}

/// Paid in the last N days: payments received, by payment date, from
/// `from` through `to` (today), both included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaidView {
    /// The window, in days.
    pub days: u32,
    /// Its first day.
    pub from: String,
    /// Its last day: today.
    pub to: String,
    /// Per commodity, the default first, then the most received. Empty
    /// when nothing was.
    pub commodities: Vec<PaidCommodityView>,
}

/// Why an invoice is on the recently updated row: its latest event.
/// Ordered as an invoice's life goes, which breaks a same-day tie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecentEvent {
    /// Its date (nothing later known).
    Dated,
    /// Its file was saved.
    Edited,
    /// It was sent.
    Sent,
    /// Money came in for it.
    Paid,
    /// It was voided.
    Voided,
}

/// One recently updated invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecentView {
    /// Its number.
    pub number: Option<String>,
    /// Its file.
    pub path: VaultPath,
    /// Its customer's name (or link target).
    pub customer: Option<String>,
    /// Its date.
    pub date: Option<String>,
    /// The day of its latest event.
    pub updated: String,
    /// What that event was.
    pub event: RecentEvent,
    /// Where it stands: `draft`, `open`, `overdue`, `paid`, `void` (or
    /// another `status:` value).
    pub status: String,
    /// The amount due, in the journal's style.
    pub total: Option<String>,
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
    /// Paid in the last N days.
    pub paid: PaidView,
    /// The most recently updated invoices, latest first.
    pub recent: Vec<RecentView>,
    /// Recognition due; `None` when the books have no recognition file (or
    /// it can't be read: `check` says why).
    pub recognition: Option<RecognitionView>,
    /// The bucket edges and DSO window used (the settings).
    pub aging_buckets: Vec<u32>,
    /// The DSO window used.
    pub dso_days: u32,
    /// `paid-window-days`: the paid window the page starts on.
    pub paid_window_days: u32,
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
        format!("{from}+"),
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

/// Money received from `from` through `to`, per commodity: in accrual
/// books every receivable's payments (Ledgeline's settlements; credits and
/// voids are not money in), in cash books the payments applied to an
/// invoice number. By the payment's date in both.
fn paid_view(
    index: &DocumentIndex,
    states: &BookStates,
    days: u32,
    from: String,
    to: String,
) -> PaidView {
    // Per commodity: (amount, documents paid into, of them without a file).
    let mut sums: BTreeMap<String, (Dec, usize, usize)> = BTreeMap::new();
    let files = files_by_number(index);
    let in_window = |date: &str| date >= from.as_str() && date <= to.as_str();
    let mut tally = |commodity: &str, amount: Dec, number: Option<&str>| {
        let entry = sums
            .entry(commodity.to_string())
            .or_insert((Dec::zero(), 0, 0));
        add(&mut entry.0, amount);
        entry.1 += 1;
        let filed = number.is_some_and(|n| files.get(n.trim()).copied().flatten().is_some());
        if !filed {
            entry.2 += 1;
        }
    };
    match states.basis() {
        Basis::Accrual => {
            for doc in states
                .receivables()
                .documents()
                .iter()
                .filter(|d| d.role == AccrualRole::Receivable)
            {
                let mut received: Option<Dec> = None;
                for s in doc
                    .settlements
                    .iter()
                    .filter(|s| s.kind == SettlementKind::Payment && in_window(&s.date))
                {
                    add(received.get_or_insert_with(Dec::zero), s.amount);
                }
                if let Some(received) = received {
                    tally(&doc.commodity, received, doc.number.as_deref());
                }
            }
        }
        Basis::Cash => {
            for number in states.numbers() {
                let Some(standing) = states.standing(number) else {
                    continue;
                };
                let mut received: BTreeMap<&str, Dec> = BTreeMap::new();
                for p in standing
                    .payments
                    .iter()
                    .filter(|p| in_window(&p.source.date))
                {
                    add(
                        received
                            .entry(p.commodity.as_str())
                            .or_insert_with(Dec::zero),
                        p.amount,
                    );
                }
                for (commodity, amount) in received {
                    tally(commodity, amount, Some(number));
                }
            }
        }
    }
    let mut commodities: Vec<PaidCommodityView> = sums
        .into_iter()
        .filter(|(_, (amount, _, _))| *amount > Dec::zero())
        .map(|(commodity, (amount, count, unfiled))| PaidCommodityView {
            text: index.money().style(Some(&commodity)).format(amount),
            commodity,
            amount,
            count,
            unfiled,
        })
        .collect();
    let default = index.money().style(None).commodity().to_string();
    commodities.sort_by(|a, b| {
        (b.commodity == default)
            .cmp(&(a.commodity == default))
            .then(b.amount.cmp(&a.amount))
            .then(a.commodity.cmp(&b.commodity))
    });
    PaidView {
        days,
        from,
        to,
        commodities,
    }
}

/// `modified` as a local date, `utc_offset` seconds east of UTC.
fn local_date(modified: SystemTime, utc_offset: i64) -> Option<String> {
    let seconds = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let local = i64::try_from(seconds).ok()?.checked_add(utc_offset)?;
    Some(iso_from_days(local.div_euclid(86_400)))
}

/// Where a document stands, for its card: draft, open, overdue, paid,
/// void, or its own `status:`.
fn standing_of(doc: &IndexedDoc, states: &BookStates, today: &str) -> String {
    match doc.header.status {
        Some(Status::Sent) if super::overdue_in(Some(states), doc, today) => "overdue".into(),
        Some(Status::Sent) if super::open_in(Some(states), doc) => "open".into(),
        Some(Status::Sent) => "paid".into(),
        Some(status) => status.as_str().into(),
        None => "draft".into(),
    }
}

/// The invoices updated most recently on or before `today`, latest first
/// (PLAN R15). An invoice's update is the latest of its date, its file's
/// modification day, its sent date, the last money received and its void
/// date; anything after `today` doesn't count (the dashboard is as of
/// `today`). A same-day tie goes to the later event in an invoice's life
/// (voided, paid, sent, edited, dated), then the later modification time,
/// then the higher number.
fn recent(
    index: &DocumentIndex,
    states: &BookStates,
    today: &str,
    utc_offset: i64,
) -> Vec<RecentView> {
    type Key = (String, RecentEvent, Option<SystemTime>);
    let on_or_before = |d: &String| d.as_str() <= today;
    let mut rows: Vec<(Key, &IndexedDoc)> = Vec::new();
    for doc in index.docs().filter(|d| is_invoice(d)) {
        let header = &doc.header;
        let edited = doc
            .stamp
            .modified
            .and_then(|m| local_date(m, utc_offset).map(|d| (m, d)))
            .filter(|(_, d)| on_or_before(d));
        let events = [
            (header.date.clone(), RecentEvent::Dated),
            (edited.as_ref().map(|(_, d)| d.clone()), RecentEvent::Edited),
            (header.sent_date.clone(), RecentEvent::Sent),
            (
                super::paid_dates(Some(states), doc)
                    .into_iter()
                    .filter(on_or_before)
                    .max(),
                RecentEvent::Paid,
            ),
            (header.void_date.clone(), RecentEvent::Voided),
        ];
        let latest = events
            .into_iter()
            .filter_map(|(date, event)| date.filter(on_or_before).map(|d| (d, event)))
            .max();
        if let Some((date, event)) = latest {
            rows.push(((date, event, edited.map(|(m, _)| m)), doc));
        }
    }
    rows.sort_by(|(a, x), (b, y)| {
        b.cmp(a).then_with(|| {
            crate::numbering::natural_cmp(y.number().unwrap_or(""), x.number().unwrap_or(""))
        })
    });
    rows.into_iter()
        .take(RECENT_COUNT)
        .map(|((updated, event, _), doc)| {
            let party = &doc.header.invoicee;
            RecentView {
                number: doc.number().map(str::to_string),
                path: doc.path.clone(),
                customer: party.name.clone().or_else(|| {
                    party
                        .link
                        .as_deref()
                        .map(|l| crate::notes::wikilink_target(l).unwrap_or(l).to_string())
                }),
                date: doc.header.date.clone(),
                updated,
                event,
                status: standing_of(doc, states, today),
                total: doc.summary.as_ref().map(|s| s.total_text.clone()),
            }
        })
        .collect()
}

/// The paid window of `days` ending on `today`: its first and last day,
/// both included (30 days to 2026-11-22 is 2026-10-24 through 2026-11-22).
#[must_use]
pub fn paid_window(today: &str, days: u32) -> (String, String) {
    (
        add_days(today, 1 - i64::from(days.max(1))),
        today.to_string(),
    )
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
        paid: {
            let (from, to) = paid_window(today, options.paid_days);
            paid_view(index, states, options.paid_days.max(1), from, to)
        },
        recent: recent(index, states, today, options.utc_offset),
        recognition,
        aging_buckets: options.aging_edges.clone(),
        dso_days: options.dso_days,
        paid_window_days: options.paid_window_days,
    }
}

/// [`dashboard`] for an open session with `options` (start from
/// [`DashboardOptions::of`] its settings), and the recognition due when the
/// books have a recognition file.
#[must_use]
pub fn dashboard_for(session: &Session, today: &str, options: &DashboardOptions) -> Dashboard {
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
        options,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_paid_window_ends_today_and_counts_both_ends() {
        assert_eq!(
            paid_window("2026-11-22", 30),
            ("2026-10-24".to_string(), "2026-11-22".to_string())
        );
        assert_eq!(paid_window("2026-11-22", 1).0, "2026-11-22");
        assert_eq!(
            paid_window("2026-11-22", 0).0,
            "2026-11-22",
            "at least a day"
        );
        assert_eq!(paid_window("2026-03-01", 7).0, "2026-02-23");
    }

    #[test]
    fn a_files_time_is_dated_in_local_time() {
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_795_000_000);
        // 2026-11-18 11:06:40 UTC.
        assert_eq!(local_date(at, 0).as_deref(), Some("2026-11-18"));
        assert_eq!(local_date(at, -12 * 3600).as_deref(), Some("2026-11-17"));
        assert_eq!(local_date(at, 16 * 3600).as_deref(), Some("2026-11-19"));
    }

    #[test]
    fn a_later_event_wins_a_same_day_tie() {
        assert!(RecentEvent::Voided > RecentEvent::Paid);
        assert!(RecentEvent::Paid > RecentEvent::Sent);
        assert!(RecentEvent::Sent > RecentEvent::Edited);
        assert!(RecentEvent::Edited > RecentEvent::Dated);
    }

    #[test]
    fn default_buckets_are_ledgelines() {
        let buckets = empty_buckets(&[30, 60, 90]);
        let keys: Vec<&str> = buckets.iter().map(|b| b.key.as_str()).collect();
        let labels: Vec<&str> = buckets.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(keys, ["current", "1-30", "31-60", "61-90", "91+"]);
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
        assert_eq!(keys, ["current", "1-15", "16-45", "46+"]);
        let keys: Vec<String> = empty_buckets(&[7]).into_iter().map(|b| b.key).collect();
        assert_eq!(keys, ["current", "1-7", "8+"]);
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
