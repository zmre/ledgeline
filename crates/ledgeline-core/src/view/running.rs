//! Running balances in the journal view (plan 36): pin an account and every
//! row gains what it did to that account's subtree (**change**) and the
//! subtree's balance after it (**balance**) — `hledger reg -H <account>`
//! inside the journal table, whatever the filter shows.
//!
//! # Semantics
//!
//! - **One order, the assertion checker's.** Postings are summed in
//!   [`posting_order`]: effective (posting) date, ties in file read order. It
//!   is the same function [`crate::assertions::check_balance_assertions`]
//!   evaluates in, so the column cannot contradict an assertion mark, and it
//!   is the order `hledger reg` lists postings in (pinned by the parity
//!   golden in `tests/running_balance.rs`).
//! - **The whole journal, always.** The balance is computed once over every
//!   transaction; a filter only decides which rows are shown. Nothing before
//!   the visible range is dropped (hledger's `-H`).
//! - **Subtree.** The pinned root and every account under it
//!   ([`account_matches`]), real, virtual and balanced-virtual postings alike,
//!   generated (`--auto`) postings included when the snapshot has them.
//! - **Quantities as written.** Costs are ignored, as assertions ignore them.
//! - **One commodity per column.** Every commodity the subtree posts is kept,
//!   including one that nets to zero, ranked by posting count (ties by name);
//!   the first is the default.
//!
//! # A row's cell ([`RunningBalance::cell`])
//!
//! | Row | change | balance |
//! |---|---|---|
//! | has postings in the subtree, this commodity | their sum | the total after its LAST such posting in the stream |
//! | anything else | `None` | carried: the total after every posting keyed ≤ (row date, row position) |
//!
//! A posting-dated leg (`; date:2026-04-02` on a transaction dated 03-28)
//! enters the stream on its own date, so a touching row's balance is the
//! total at that point and [`RunningCell::posted_on`] names the date.
//!
//! # Assertion marks ([`RunningBalance::assertion_marks`])
//!
//! Pass or fail is the checker's verdict ([`AssertionFailure`]), never a
//! comparison with the column: a plain `=` on a child asserts the child's own
//! balance while the column sums the whole subtree.
//!
//! # Balance over time ([`RunningBalance::series`], plan 36 amendment A1)
//!
//! The same stream as a chart: one point per period of a [`BalanceWindow`],
//! each the total after the last posting whose effective date is on or before
//! the period's (window-clamped) end — found by binary search over the entries
//! already built, never a second pass over the journal. A period with no
//! posting carries the balance flat; a window before the first posting is all
//! zero. The window follows the journal's date filter (an open start is the
//! subtree's first posting, an open end `today`) at the interval
//! [`auto_interval`] picks for the holdings value-over-time chart, so the two
//! balance-over-time lines in the app coarsen alike.

pub use crate::assertions::{PostingRef, posting_order};

use super::rows::{RowAmount, RowContext};
use super::wire::{DeclaredStyles, WireDec, WireStyle};
use crate::assertions::{AssertionFailure, operator};
use crate::decimal::{Dec, DecError};
use crate::holdings::auto_interval;
use crate::model::{Amount, AmountStyle, Commodity, Journal};
use crate::reports::{Interval, ReportError, account_matches, bucket_label, range_buckets};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// One posting's amount in a commodity stream, with the running total after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Where the posting sits in the journal.
    pub at: PostingRef,
    /// The subtree's balance in this commodity after the posting.
    pub after: Dec,
}

/// One transaction's postings in a commodity stream, by position.
#[derive(Debug, Clone, Copy)]
struct RowTouch {
    position: u32,
    /// Sum of the row's amounts in this commodity.
    change: Dec,
    /// Index into the stream's entries of the row's last posting.
    last: u32,
}

/// One commodity's running balance over the pinned subtree.
#[derive(Debug)]
struct Stream {
    commodity: Commodity,
    /// Postings (not amounts) in the subtree carrying this commodity.
    postings: usize,
    /// How a figure in this commodity displays when no declaration says
    /// otherwise: the first written (not generated) amount's style.
    style: AmountStyle,
    entries: Vec<Entry>,
    /// Sorted by position.
    rows: Vec<RowTouch>,
}

impl Stream {
    fn row(&self, position: u32) -> Option<&RowTouch> {
        self.rows
            .binary_search_by_key(&position, |row| row.position)
            .ok()
            .map(|at| &self.rows[at])
    }
}

/// A pinned subtree's running balance in every commodity it posts.
#[derive(Debug)]
pub struct RunningBalance {
    root: String,
    /// Ranked: most postings first, ties by commodity.
    streams: Vec<Stream>,
    /// Every posting in the subtree that carries an assertion, in stream order.
    asserting: Vec<PostingRef>,
}

/// What one row shows in the pinned columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningCell<'j> {
    /// The row's net in the subtree and commodity; `None` when it has no
    /// posting there.
    pub change: Option<Dec>,
    /// The subtree's balance after the row (see the module docs).
    pub balance: Dec,
    /// Whether the row has a posting in the subtree in this commodity.
    pub touches: bool,
    /// The effective date of the row's last posting in the stream, when it is
    /// not the row's own date (a posting-dated leg).
    pub posted_on: Option<&'j str>,
}

/// An assertion on a posting in the pinned subtree, with the checker's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionMark {
    /// The asserting posting: its transaction's position, and (through
    /// [`PostingRef::date`]) the effective date it is evaluated on.
    pub at: PostingRef,
    /// The asserted account (the root or one under it).
    pub account: String,
    /// `=`, `==`, `=*` or `==*`.
    pub operator: &'static str,
    /// The commodity this mark is about.
    pub commodity: Commodity,
    /// What the journal claims.
    pub asserted: Dec,
    /// What the balance actually was; only for a failure.
    pub calculated: Option<Dec>,
    /// The checker's verdict.
    pub ok: bool,
    /// A `==`/`==*` failure on a commodity OTHER than the one written: it
    /// asserted this commodity is zero and it is not. Shown whichever
    /// commodity is pinned.
    pub zero_check: bool,
    /// How `asserted`/`calculated` display absent a declared style.
    pub style: AmountStyle,
}

/// The dates and grain a balance-over-time series is drawn over: both ends
/// inclusive, as the journal filter's are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceWindow {
    /// The first day drawn.
    pub from: String,
    /// The last day drawn.
    pub to: String,
    /// The period each point closes.
    pub interval: Interval,
}

impl BalanceWindow {
    /// The window for the journal filter's `from`/`to`: an open start is
    /// `first_posting` (the subtree's), an open end `today`, and an absent
    /// `interval` is [`auto_interval`]'s choice for the span.
    ///
    /// An open end never lands before an explicit start (a range starting in
    /// the future runs to its own start), nor an open start after an explicit
    /// end (a range ending before the first posting starts at its end): the
    /// open side follows the closed one rather than inverting the window. Two
    /// explicit ends are taken as given; a caller rejects `from > to`.
    #[must_use]
    pub fn resolve(
        first_posting: &str,
        from: Option<&str>,
        to: Option<&str>,
        today: &str,
        interval: Option<Interval>,
    ) -> Self {
        let to = to.map_or_else(
            || from.map_or(today, |from| today.max(from)).to_string(),
            str::to_string,
        );
        let from = from.map_or_else(
            || first_posting.min(to.as_str()).to_string(),
            str::to_string,
        );
        let interval = interval.unwrap_or_else(|| auto_interval(&from, &to));
        Self { from, to, interval }
    }
}

/// One period of a balance-over-time series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesPoint<'m> {
    /// The period's bucket key (`2026-03`, `2026-W10`, `2026-03-02`, …).
    pub key: String,
    /// The period's last day, clamped to the window: the date the balance is
    /// taken at.
    pub end: String,
    /// The subtree's balance after every posting dated on or before `end`.
    pub balance: Dec,
    /// The transaction (position) whose posting is the last one counted in
    /// this period; `None` when nothing in this commodity posted in it.
    pub last: Option<u32>,
    /// The failing assertions in this commodity whose effective date falls in
    /// the period, in evaluation order.
    pub failures: Vec<SeriesFailure<'m>>,
}

/// A failing assertion placed on the balance line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesFailure<'m> {
    /// The checker's verdict on it.
    pub mark: &'m AssertionMark,
    /// Where the line would be at this period's end had the books matched the
    /// assertion: the point's balance shifted by `asserted − calculated`. For
    /// an `=*` on the root that is the last posting of its period this is the
    /// asserted figure itself; for a `=` on a child (which asserts the child,
    /// not the subtree the line sums) it is the same GAP drawn from the line,
    /// so the vertical distance always reads as "off by".
    pub expected: Dec,
}

impl RunningBalance {
    /// The running balance of `root`'s subtree, one pass over `order` (from
    /// [`posting_order`] over the same `journal`).
    ///
    /// # Errors
    /// [`DecError`] when a running total overflows.
    pub fn build(journal: &Journal, order: &[PostingRef], root: &str) -> Result<Self, DecError> {
        // Per stream: (position, entry index, quantity), for the row table.
        let mut touches: Vec<Vec<(u32, u32, Dec)>> = Vec::new();
        let mut streams: Vec<Stream> = Vec::new();
        let mut written_style: Vec<bool> = Vec::new();
        let mut index: HashMap<&Commodity, usize> = HashMap::new();
        let mut asserting = Vec::new();
        for at in order {
            let (_, posting) = at.resolve(journal);
            if !account_matches(root, &posting.account.0) {
                continue;
            }
            if posting.balance_assertion.is_some() {
                asserting.push(*at);
            }
            for amount in &posting.amounts {
                let slot = *index.entry(&amount.commodity).or_insert_with(|| {
                    streams.push(Stream {
                        commodity: amount.commodity.clone(),
                        postings: 0,
                        style: amount.style.clone(),
                        entries: Vec::new(),
                        rows: Vec::new(),
                    });
                    touches.push(Vec::new());
                    written_style.push(false);
                    streams.len() - 1
                });
                let stream = &mut streams[slot];
                if !written_style[slot] && posting.generated.is_none() {
                    stream.style = amount.style.clone();
                    written_style[slot] = true;
                }
                let last = stream.entries.last();
                if last.is_none_or(|entry| entry.at != *at) {
                    stream.postings += 1;
                }
                // `amount.cost` is deliberately untouched, as for assertions.
                let after = last
                    .map_or_else(Dec::zero, |entry| entry.after)
                    .add(amount.quantity)?;
                let entry_index = u32::try_from(stream.entries.len()).unwrap_or(u32::MAX);
                stream.entries.push(Entry { at: *at, after });
                touches[slot].push((at.position, entry_index, amount.quantity));
            }
        }
        for (stream, mut touched) in streams.iter_mut().zip(touches) {
            // Stable: a row's amounts keep stream order, so its last is last.
            touched.sort_by_key(|(position, _, _)| *position);
            let mut rows: Vec<RowTouch> = Vec::new();
            for (position, entry, quantity) in touched {
                match rows.last_mut() {
                    Some(row) if row.position == position => {
                        row.change = row.change.add(quantity)?;
                        row.last = entry;
                    }
                    _ => rows.push(RowTouch {
                        position,
                        change: quantity,
                        last: entry,
                    }),
                }
            }
            stream.rows = rows;
        }
        streams.sort_by(|a, b| {
            b.postings
                .cmp(&a.postings)
                .then_with(|| a.commodity.cmp(&b.commodity))
        });
        Ok(Self {
            root: root.to_string(),
            streams,
            asserting,
        })
    }

    /// The pinned root.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Whether no posting in the journal is in the subtree.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    /// Every commodity the subtree posts with its posting count, ranked: most
    /// postings first, ties by commodity. The first is the default.
    pub fn commodities(&self) -> impl Iterator<Item = (&Commodity, usize)> {
        self.streams
            .iter()
            .map(|stream| (&stream.commodity, stream.postings))
    }

    /// `requested` when the subtree posts it, else the default (most postings);
    /// `None` only for an empty subtree. A stale commodity in a shared link
    /// therefore still draws a column.
    #[must_use]
    pub fn resolve_commodity(&self, requested: Option<&str>) -> Option<&Commodity> {
        requested
            .and_then(|wanted| self.stream(wanted))
            .or_else(|| self.streams.first())
            .map(|stream| &stream.commodity)
    }

    /// The display style for a figure in `commodity` absent a declaration.
    #[must_use]
    pub fn style(&self, commodity: &str) -> Option<&AmountStyle> {
        self.stream(commodity).map(|stream| &stream.style)
    }

    /// The `commodity` stream's entries in evaluation order.
    #[must_use]
    pub fn entries(&self, commodity: &str) -> &[Entry] {
        self.stream(commodity)
            .map_or(&[], |stream| stream.entries.as_slice())
    }

    fn stream(&self, commodity: &str) -> Option<&Stream> {
        self.streams
            .iter()
            .find(|stream| stream.commodity.0 == commodity)
    }

    /// The cell for the transaction at `position` in `commodity`; `None` when
    /// the subtree never posts `commodity` or there is no such transaction.
    #[must_use]
    pub fn cell<'j>(
        &self,
        journal: &'j Journal,
        commodity: &str,
        position: u32,
    ) -> Option<RunningCell<'j>> {
        let stream = self.stream(commodity)?;
        let txn = journal.transactions.get(position as usize)?;
        if let Some(row) = stream.row(position) {
            let entry = stream.entries[row.last as usize];
            let posted = entry.at.date(journal);
            return Some(RunningCell {
                change: Some(row.change),
                balance: entry.after,
                touches: true,
                posted_on: (posted != txn.date).then_some(posted),
            });
        }
        // Entries are in (effective date, position, posting) order, so the
        // ones keyed at or before the row are a prefix.
        let key = (txn.date.as_str(), position);
        let before = stream
            .entries
            .partition_point(|entry| (entry.at.date(journal), entry.at.position) <= key);
        Some(RunningCell {
            change: None,
            balance: before
                .checked_sub(1)
                .map_or_else(Dec::zero, |at| stream.entries[at].after),
            touches: false,
            posted_on: None,
        })
    }

    /// The earliest effective date of any posting in the subtree, in any
    /// commodity; `None` for an empty subtree.
    #[must_use]
    pub fn first_date<'j>(&self, journal: &'j Journal) -> Option<&'j str> {
        self.streams
            .iter()
            .filter_map(|stream| stream.entries.first())
            .map(|entry| entry.at.date(journal))
            .min()
    }

    /// The subtree's balance in `commodity` at the end of every period of
    /// `window`, oldest first (see the module docs). `marks` is the
    /// commodity's list from [`RunningBalance::assertion_marks`]; its failures
    /// are placed in the period their effective date falls in. A commodity
    /// the subtree never posts is a flat zero line.
    ///
    /// # Errors
    /// [`ReportError`] when the window's buckets cannot be computed
    /// (unreachable for well-formed dates) or a failure's offset overflows.
    pub fn series<'m>(
        &self,
        journal: &Journal,
        commodity: &str,
        window: &BalanceWindow,
        marks: &'m [AssertionMark],
    ) -> Result<Vec<SeriesPoint<'m>>, ReportError> {
        let entries = self.entries(commodity);
        // Marks are in stream order, so the failing ones are sorted by
        // effective date too and a period's are a contiguous run.
        let failing: Vec<&AssertionMark> = marks.iter().filter(|mark| !mark.ok).collect();
        let entry_date = |entry: &Entry| entry.at.date(journal);
        let mark_date = |mark: &&AssertionMark| mark.at.date(journal);
        range_buckets(&window.from, &window.to, Some(window.interval))?
            .into_iter()
            .map(|span| {
                let (from, to) = (span.from.as_str(), span.to.as_str());
                let through = entries.partition_point(|entry| entry_date(entry) <= to);
                let since = entries.partition_point(|entry| entry_date(entry) < from);
                let balance = through
                    .checked_sub(1)
                    .map_or_else(Dec::zero, |at| entries[at].after);
                let failures = failing[failing.partition_point(|mark| mark_date(mark) < from)
                    ..failing.partition_point(|mark| mark_date(mark) <= to)]
                    .iter()
                    .filter_map(|mark| mark.calculated.map(|calculated| (*mark, calculated)))
                    .map(|(mark, calculated)| {
                        Ok(SeriesFailure {
                            mark,
                            expected: balance.add(mark.asserted)?.sub(calculated)?,
                        })
                    })
                    .collect::<Result<Vec<_>, DecError>>()?;
                Ok(SeriesPoint {
                    last: (through > since).then(|| entries[through - 1].at.position),
                    end: span.to,
                    key: span.key,
                    balance,
                    failures,
                })
            })
            .collect()
    }

    /// Every assertion on a posting in the subtree, by the commodity it is
    /// about, each list in stream (hledger evaluation) order.
    ///
    /// An assertion is a pass unless `failures` (the checker's output for the
    /// same journal) names it. It is filed under its written commodity; a
    /// `==`/`==*` zero-check failure on another commodity is filed under that
    /// one too, as a [`AssertionMark::zero_check`].
    #[must_use]
    pub fn assertion_marks(
        &self,
        journal: &Journal,
        failures: &[AssertionFailure],
    ) -> BTreeMap<Commodity, Vec<AssertionMark>> {
        let mut by_site: HashMap<(usize, u32, u32), Vec<&AssertionFailure>> = HashMap::new();
        for failure in failures {
            let site = (
                failure.transaction_index,
                failure.position.line,
                failure.position.column,
            );
            by_site.entry(site).or_default().push(failure);
        }
        let mut marks: BTreeMap<Commodity, Vec<AssertionMark>> = BTreeMap::new();
        for at in &self.asserting {
            let (_, posting) = at.resolve(journal);
            let Some(assertion) = &posting.balance_assertion else {
                continue;
            };
            let site = (
                at.position as usize,
                assertion.position.line,
                assertion.position.column,
            );
            let failed: Vec<&AssertionFailure> = by_site
                .get(&site)
                .into_iter()
                .flatten()
                .copied()
                .filter(|failure| failure.account == posting.account)
                .collect();
            let written = &assertion.amount.commodity;
            let mark = |commodity: &Commodity,
                        asserted: Dec,
                        calculated: Option<Dec>,
                        zero_check: bool,
                        style: AmountStyle| {
                AssertionMark {
                    at: *at,
                    account: posting.account.0.clone(),
                    operator: operator(assertion.total, assertion.inclusive),
                    commodity: commodity.clone(),
                    asserted,
                    calculated,
                    ok: calculated.is_none(),
                    zero_check,
                    style,
                }
            };
            let own = failed.iter().find(|failure| failure.commodity == *written);
            marks.entry(written.clone()).or_default().push(mark(
                written,
                assertion.amount.quantity,
                own.map(|failure| failure.calculated),
                false,
                assertion.amount.style.clone(),
            ));
            for failure in failed
                .iter()
                .filter(|failure| failure.commodity != *written)
            {
                marks
                    .entry(failure.commodity.clone())
                    .or_default()
                    .push(mark(
                        &failure.commodity,
                        failure.asserted,
                        Some(failure.calculated),
                        true,
                        failure.style.clone(),
                    ));
            }
        }
        marks
    }
}

// ---------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------

/// A row's `running` object on `GET /api/journal/rows?balance=`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireRunning<'a> {
    /// `null` when the row has no posting in the subtree in this commodity.
    change: Option<RowAmount>,
    balance: RowAmount,
    touches: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    posted_on: Option<&'a str>,
    assertions: Vec<WireMark<'a>>,
}

/// One assertion mark on a row.
#[derive(Debug, Serialize)]
pub struct WireMark<'a> {
    account: &'a str,
    op: &'static str,
    asserted: RowAmount,
    #[serde(skip_serializing_if = "Option::is_none")]
    calculated: Option<RowAmount>,
    ok: bool,
}

impl<'a> WireMark<'a> {
    /// `mark` in the row's shape.
    #[must_use]
    pub fn new(mark: &'a AssertionMark, context: &RowContext<'_>) -> Self {
        let amount = |quantity| context.amount_of(&mark.commodity, quantity, &mark.style);
        Self {
            account: &mark.account,
            op: mark.operator,
            asserted: amount(mark.asserted),
            calculated: mark.calculated.map(amount),
            ok: mark.ok,
        }
    }
}

impl<'a> WireRunning<'a> {
    /// `cell` in `commodity`, rendered with `context`, carrying `marks`.
    #[must_use]
    pub fn new(
        cell: RunningCell<'a>,
        commodity: &Commodity,
        style: &AmountStyle,
        context: &RowContext<'_>,
        assertions: Vec<WireMark<'a>>,
    ) -> Self {
        let amount = |quantity| context.amount_of(commodity, quantity, style);
        Self {
            change: cell.change.map(amount),
            balance: amount(cell.balance),
            touches: cell.touches,
            posted_on: cell.posted_on,
            assertions,
        }
    }
}

/// The `GET /api/journal/running` body (less `generation`).
#[derive(Debug, Serialize)]
pub struct WireRunningSummary<'a> {
    account: &'a str,
    commodities: Vec<WireCommodityCount<'a>>,
    assertions: BTreeMap<&'a str, Vec<WireAssertionStop<'a>>>,
}

/// One commodity the subtree posts, with its posting count.
#[derive(Debug, Serialize)]
pub struct WireCommodityCount<'a> {
    commodity: &'a str,
    postings: usize,
}

/// One assertion in a commodity's ordered list.
#[derive(Debug, Serialize)]
pub struct WireAssertionStop<'a> {
    tindex: u32,
    /// The asserting transaction's primary date — the date a date filter and
    /// a row jump work with.
    date: &'a str,
    ok: bool,
}

impl<'a> WireRunningSummary<'a> {
    /// The chip's and picker's data for `running`.
    #[must_use]
    pub fn new(
        journal: &'a Journal,
        running: &'a RunningBalance,
        marks: &'a BTreeMap<Commodity, Vec<AssertionMark>>,
    ) -> Self {
        Self {
            account: running.root(),
            commodities: running
                .commodities()
                .map(|(commodity, postings)| WireCommodityCount {
                    commodity: &commodity.0,
                    postings,
                })
                .collect(),
            assertions: marks
                .iter()
                .map(|(commodity, marks)| {
                    let stops = marks
                        .iter()
                        .map(|mark| {
                            let txn = &journal.transactions[mark.at.position as usize];
                            WireAssertionStop {
                                tindex: txn.index.0,
                                date: &txn.date,
                                ok: mark.ok,
                            }
                        })
                        .collect();
                    (commodity.0.as_str(), stops)
                })
                .collect(),
        }
    }
}

/// The `GET /api/journal/running/series` body (less `generation`).
#[derive(Debug, Serialize)]
pub struct WireBalanceSeries<'a> {
    account: &'a str,
    commodity: &'a str,
    /// How every figure in the body displays.
    style: WireStyle,
    /// `daily`, `weekly`, `monthly`, `quarterly` or `yearly`.
    interval: &'static str,
    /// The window drawn, both ends resolved and inclusive.
    from: &'a str,
    to: &'a str,
    points: Vec<WireSeriesPoint<'a>>,
}

/// One period on the line.
#[derive(Debug, Serialize)]
pub struct WireSeriesPoint<'a> {
    /// The bucket key.
    bucket: &'a str,
    /// [`bucket_label`] of the key.
    label: String,
    /// The day the balance is taken at.
    end: &'a str,
    balance: WireDec,
    /// The newest transaction counted in the period: what clicking it scrolls
    /// the journal to. Absent for a period with no posting.
    #[serde(skip_serializing_if = "Option::is_none")]
    tindex: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failures: Vec<WireSeriesFailure<'a>>,
}

/// A failing assertion inside a period.
#[derive(Debug, Serialize)]
pub struct WireSeriesFailure<'a> {
    tindex: u32,
    /// The assertion's effective date: the day it is checked on.
    date: &'a str,
    account: &'a str,
    op: &'static str,
    asserted: WireDec,
    calculated: WireDec,
    /// See [`SeriesFailure::expected`].
    expected: WireDec,
}

/// The wire spelling of an interval, as the route's `interval` takes it.
#[must_use]
pub fn interval_code(interval: Interval) -> &'static str {
    match interval {
        Interval::Daily => "daily",
        Interval::Weekly => "weekly",
        Interval::Monthly => "monthly",
        Interval::Quarterly => "quarterly",
        Interval::Yearly => "yearly",
    }
}

impl<'a> WireBalanceSeries<'a> {
    /// `points` (from [`RunningBalance::series`] over `window`) in the wire
    /// shape. One style covers every figure: the commodity's declared style,
    /// else the subtree's as-written one, widened to the most places any
    /// figure here has so no digit is hidden.
    #[must_use]
    pub fn new(
        journal: &'a Journal,
        running: &'a RunningBalance,
        commodity: &'a Commodity,
        window: &'a BalanceWindow,
        points: &'a [SeriesPoint<'a>],
        declared: &DeclaredStyles,
    ) -> Self {
        let places = points
            .iter()
            .flat_map(|point| {
                std::iter::once(point.balance).chain(point.failures.iter().flat_map(|failure| {
                    [failure.mark.asserted, failure.expected]
                        .into_iter()
                        .chain(failure.mark.calculated)
                }))
            })
            .map(|figure| figure.places)
            .max()
            .unwrap_or(0);
        let written = running
            .style(&commodity.0)
            .cloned()
            .unwrap_or_else(super::wire::default_style);
        let precision = places.max(written.precision);
        let style = declared.display_style(&Amount {
            commodity: commodity.clone(),
            quantity: Dec::new(0, precision),
            style: AmountStyle {
                precision,
                ..written
            },
            cost: None,
        });
        let tindex = |position: u32| journal.transactions[position as usize].index.0;
        Self {
            account: running.root(),
            commodity: &commodity.0,
            style: WireStyle::from(&style),
            interval: interval_code(window.interval),
            from: &window.from,
            to: &window.to,
            points: points
                .iter()
                .map(|point| WireSeriesPoint {
                    bucket: &point.key,
                    label: bucket_label(&point.key),
                    end: &point.end,
                    balance: point.balance.into(),
                    tindex: point.last.map(tindex),
                    failures: point
                        .failures
                        .iter()
                        .map(|failure| {
                            let mark = failure.mark;
                            WireSeriesFailure {
                                tindex: tindex(mark.at.position),
                                date: mark.at.date(journal),
                                account: &mark.account,
                                op: mark.operator,
                                asserted: mark.asserted.into(),
                                calculated: mark.calculated.unwrap_or(mark.asserted).into(),
                                expected: failure.expected.into(),
                            }
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
