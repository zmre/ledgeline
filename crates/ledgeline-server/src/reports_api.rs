//! Native (non-hledger) report + budget endpoints.
//!
//! These routes expose the golden-verified `ledgeline_core::reports` engine over
//! HTTP for the SPA. Unlike the Phase-2 wire endpoints (whose bodies are
//! precomputed once from the journal), reports depend on request query params, so
//! they are computed per request from the parsed journal held in [`AppState`].
//!
//! The JSON contract is the engine's own native shape (NOT hledger's), designed
//! to map 1:1 onto `web/src/lib/reports/types.ts`:
//! - `Dec` → `{"mantissa": <string>, "places": <number>}`. The mantissa is
//!   STRING-encoded (decoded via `BigInt` on the SPA): unlike parsed amounts,
//!   COMPUTED values (e.g. holdings `marketValue = shares × price`, non-
//!   normalized) can exceed the JS safe-integer range, so a JSON number would
//!   silently lose precision.
//! - `MixedAmount` → `{"<commodity>": <Dec>, …}` (TS `Map<string, Dec>`), with
//!   zero commodities dropped (the additive-identity contract).
//! - `SectionedReport`/`PeriodReport`/`BudgetReport` use camelCase keys matching
//!   the TS interfaces (`grandTotal`, `asOf`, …).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Query, State};
use ledgeline_core::Dec;
use ledgeline_core::holdings::engine::prices_any_held_other;
use ledgeline_core::holdings::{
    Holding, HoldingPrice, HoldingsPoint, HoldingsReport, HoldingsScope, HoldingsSeries,
    HoldingsTotals, HoldingsWarning, OtherHolding, OtherHoldingsReport, OtherHoldingsTotals,
    OtherHoldingsWarning, OtherWarningKind, PriceSource, ScopeMode, WarningKind, compute_holdings,
    holdings_series, other_holdings, other_holdings_series, prices_any_held,
};
use ledgeline_core::model::{Commodity, Journal};
use ledgeline_core::reports::periods;
use ledgeline_core::reports::periods::MAX_BUCKETS;
use ledgeline_core::reports::{
    Amounts, BalanceSheetReport, BsGroup, BsOpts, BsSection, BsSectionKind, BsSubsection, BsTerm,
    BudgetCell, BudgetOpts, BudgetReport, BudgetRow, Cadence, ChangeKind, ChangeRow, CostOfLiving,
    DEFAULT_EXCLUDE_DESC, DateRange, GroupSource, IS_GROUP_TAG, IncomeStatementReport,
    InsightsOpts, InsightsPeriod, InsightsReport, Interval, InvestmentPerf, IsGroup, IsOpts, IsRow,
    IsSection, IsSectionKind, IsSubtotal, IsSubtotalKind, MetricDelta, MixedAmount, MoverRow,
    NetWorthOpts, PerfPoint, PeriodReport, PeriodRow, ReportError, ReportMeta, ReportRow, Section,
    SectionedReport, Subscription, SubscriptionOpts, SubscriptionsReport, TopTxn, Valuation,
    account_decls, account_groups, account_sections, balance_sheet, balance_sheet_grouped,
    bs_terms, budget_report, cash_flow, cash_predicate, declared_groups, declared_types,
    detect_subscriptions, income_statement, income_statement_grouped, insights, net_worth,
    prices_any_on_sheet, prices_any_on_statement,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::AppError;

/// Account-depth clamp default (mirrors `ReportParams` in `web/.../params.ts`).
const DEFAULT_DEPTH: usize = 2;
/// Lookback bucket-count default (mirrors `ReportParams`).
const DEFAULT_COUNT: usize = 12;

// ===========================================================================
// Wire representation of the report result types
// ===========================================================================
//
// EVERY `Wire*` struct below carries `#[serde(rename_all = "camelCase")]`,
// including the ones whose fields are all single-word today and for which the
// attribute is therefore a no-op (DRY-3). It is deliberately unconditional: the
// SPA mirror in `web/src/lib/api/nativeDecode.ts` is hand-written and spells
// every key camelCase, so a struct without the attribute is a trap that springs
// the first time somebody adds a multi-word field — serde would emit
// `first_seen`, the decoder would read `firstSeen`, and (for a money field) the
// SPA would render `$0.00` with no error on either side. Adding it everywhere
// costs nothing and removes the trap rather than documenting it.
//
// The `fixtures/native/v1/` goldens (`just snapshot-native`) are the other half
// of that guard: they pin the actual bytes, and are replayed by
// `tests/native_wire_golden.rs` and decoded by `nativeDecode.test.ts`, so a
// renamed field fails on both sides instead of silently zeroing a report.

/// An exact decimal on the wire: `mantissa / 10^places`.
///
/// This is the ONE `Dec` → wire encoding. It used to be written out four times
/// (twice here, once inline inside [`wire_mixed`], and once as `edit_api`'s
/// byte-identical `WireDecOut`); `edit_api` now serializes through this type, so
/// the read and write wires cannot describe a decimal differently.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireDec {
    /// STRING-encoded significand (see the module doc): computed values can
    /// exceed the JS safe-integer range, so a JSON number would lose precision.
    mantissa: String,
    places: u32,
}

impl From<Dec> for WireDec {
    fn from(dec: Dec) -> Self {
        Self {
            mantissa: dec.mantissa.to_string(),
            places: dec.places,
        }
    }
}

/// A commodity-keyed bag of exact quantities → the SPA `Map<string, Dec>`. Zero
/// commodities are dropped, matching the engine's zero-free result contract.
///
/// A plain function rather than a `From` impl: both `BTreeMap` and
/// [`MixedAmount`] are foreign types, so the orphan rule forbids the impl.
type WireMixed = BTreeMap<String, WireDec>;

fn wire_mixed(ma: &MixedAmount) -> WireMixed {
    ma.iter()
        .filter(|(_, dec)| !dec.is_zero())
        .map(|(commodity, dec)| (commodity.0.clone(), WireDec::from(*dec)))
        .collect()
}

/// One row of a sectioned report.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireReportRow {
    account: String,
    depth: usize,
    own: WireMixed,
    inclusive: WireMixed,
}

impl From<&ReportRow> for WireReportRow {
    fn from(row: &ReportRow) -> Self {
        Self {
            account: row.account.clone(),
            depth: row.depth,
            own: wire_mixed(&row.own),
            inclusive: wire_mixed(&row.inclusive),
        }
    }
}

/// A titled group of rows plus its subtree total.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSection {
    title: String,
    rows: Vec<WireReportRow>,
    total: WireMixed,
}

impl From<&Section> for WireSection {
    fn from(section: &Section) -> Self {
        Self {
            title: section.title.clone(),
            rows: section.rows.iter().map(WireReportRow::from).collect(),
            total: wire_mixed(&section.total),
        }
    }
}

/// Balance sheet / income statement.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSectionedReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    as_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    sections: Vec<WireSection>,
    grand_total: WireMixed,
}

impl From<&SectionedReport> for WireSectionedReport {
    fn from(report: &SectionedReport) -> Self {
        Self {
            as_of: report.as_of.clone(),
            from: report.from.clone(),
            to: report.to.clone(),
            sections: report.sections.iter().map(WireSection::from).collect(),
            grand_total: wire_mixed(&report.grand_total),
        }
    }
}

// ---------------------------------------------------------------------------
// Grouped balance sheet (plans/12-balance-sheet-redesign.md)
// ---------------------------------------------------------------------------

/// One collapsible group. `source` names the resolution step that chose it, so
/// the UI can explain a grouping the user did not ask for.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBsGroup {
    name: String,
    /// `"tag" | "type" | "commodity" | "segment" | "computed"`.
    source: &'static str,
    /// `"current" | "noncurrent"`, or null when the split is off (and always
    /// null for the synthetic equity lines).
    term: Option<&'static str>,
    /// Empty for the `computed` lines, which stand for no accounts.
    rows: Vec<WireReportRow>,
    total: WireMixed,
}

impl From<&BsGroup> for WireBsGroup {
    fn from(group: &BsGroup) -> Self {
        Self {
            name: group.name.clone(),
            term: group.term.map(BsTerm::code),
            source: group_source(group.source),
            rows: group.rows.iter().map(WireReportRow::from).collect(),
            total: wire_mixed(&group.total),
        }
    }
}

/// The one `GroupSource` → wire spelling, shared by both grouped statements so
/// a `tag` on the balance sheet cannot become a `tagged` on the P&L.
fn group_source(source: GroupSource) -> &'static str {
    match source {
        GroupSource::Tag => "tag",
        GroupSource::Type => "type",
        GroupSource::Commodity => "commodity",
        GroupSource::Segment => "segment",
        GroupSource::Computed => "computed",
    }
}

/// One half of a box: `{term, heading, label, total}`. `term` is
/// `"current" | "noncurrent"`.
///
/// `heading` and `label` are engine-supplied strings rather than something the
/// SPA derives from `term`, because that mapping would then exist in the view
/// AND in the xlsx exporter — the two-copies shape DRY-3 is about.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBsSubsection {
    term: &'static str,
    heading: String,
    label: String,
    total: WireMixed,
}

impl From<&BsSubsection> for WireBsSubsection {
    fn from(sub: &BsSubsection) -> Self {
        Self {
            term: sub.term.code(),
            heading: sub.heading.clone(),
            label: sub.label.clone(),
            total: wire_mixed(&sub.total),
        }
    }
}

/// One of the three boxes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBsSection {
    /// `"assets" | "liabilities" | "equity"`.
    kind: &'static str,
    title: String,
    groups: Vec<WireBsGroup>,
    /// EMPTY when the journal declares no `bsterm:` — the adaptive guarantee, and
    /// what lets an untagged journal decode to exactly the report it did before
    /// the current/non-current split existed. Always empty for Equity.
    subsections: Vec<WireBsSubsection>,
    total: WireMixed,
}

impl From<&BsSection> for WireBsSection {
    fn from(section: &BsSection) -> Self {
        Self {
            kind: match section.kind {
                BsSectionKind::Assets => "assets",
                BsSectionKind::Liabilities => "liabilities",
                BsSectionKind::Equity => "equity",
            },
            title: section.title.clone(),
            groups: section.groups.iter().map(WireBsGroup::from).collect(),
            subsections: section
                .subsections
                .iter()
                .map(WireBsSubsection::from)
                .collect(),
            total: wire_mixed(&section.total),
        }
    }
}

/// The grouped, valued balance sheet. `check` is `{}` when the journal
/// balances; anything else is a real problem and the UI must show it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBalanceSheetReport {
    as_of: String,
    /// The commodity everything is valued into — `null` on the cost and
    /// unvalued bases, neither of which collapses to a single commodity. Kept
    /// (not omitted) so the decoder always sees the key.
    base: Option<String>,
    /// The basis the numbers are on, echoed back from the request.
    value: &'static str,
    sections: Vec<WireBsSection>,
    net_worth: WireMixed,
    /// The EXACT `A − L − E` residual — what to show when `balanced` is false.
    /// Not the verdict: on a journal holding priced lots this is routinely a
    /// non-zero speck of cost-multiplication dust.
    check: WireMixed,
    /// The verdict. Every client renders its ✓/✗ from THIS, so the page, the
    /// workbook it exports and any other consumer cannot reach three different
    /// conclusions from one residual.
    balanced: bool,
    /// Always present here (unlike `WirePeriodReport`'s), because the balance
    /// sheet's unpriced banner keys off it on every request.
    meta: WireReportMeta,
}

impl WireBalanceSheetReport {
    /// `value` is the request's own spelling, so the response says which basis
    /// produced the numbers it carries.
    fn new(report: &BalanceSheetReport, value: &'static str) -> Self {
        Self {
            as_of: report.as_of.clone(),
            base: report.base.as_ref().map(|base| base.0.clone()),
            value,
            sections: report.sections.iter().map(WireBsSection::from).collect(),
            net_worth: wire_mixed(&report.net_worth),
            check: wire_mixed(&report.check),
            balanced: report.balanced,
            meta: WireReportMeta::from(&report.meta),
        }
    }
}

// ---------------------------------------------------------------------------
// Grouped income statement (plans/13-income-statement-redesign.md)
// ---------------------------------------------------------------------------

/// One figure per window. `prior` is ABSENT (not null) when `compare=none`, so
/// a client cannot mistake "not compared" for "the prior period was empty".
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAmounts {
    current: WireMixed,
    #[serde(skip_serializing_if = "Option::is_none")]
    prior: Option<WireMixed>,
}

impl From<&Amounts> for WireAmounts {
    fn from(amounts: &Amounts) -> Self {
        Self {
            current: wire_mixed(&amounts.current),
            prior: amounts.prior.as_ref().map(wire_mixed),
        }
    }
}

/// The window a set of `prior` figures covers, for the column header.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireDateRange {
    from: String,
    to: String,
}

impl From<&DateRange> for WireDateRange {
    fn from(range: &DateRange) -> Self {
        Self {
            from: range.from.clone(),
            to: range.to.clone(),
        }
    }
}

/// One account inside an expanded group.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireIsRow {
    account: String,
    depth: usize,
    amounts: WireAmounts,
}

impl From<&IsRow> for WireIsRow {
    fn from(row: &IsRow) -> Self {
        Self {
            account: row.account.clone(),
            depth: row.depth,
            amounts: WireAmounts::from(&row.amounts),
        }
    }
}

/// One line of a box.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireIsGroup {
    name: String,
    /// `"tag" | "segment"` — this statement has no built-in groups, so the other
    /// three [`GroupSource`] spellings cannot occur here.
    source: &'static str,
    total: WireAmounts,
    rows: Vec<WireIsRow>,
}

impl From<&IsGroup> for WireIsGroup {
    fn from(group: &IsGroup) -> Self {
        Self {
            name: group.name.clone(),
            source: group_source(group.source),
            total: WireAmounts::from(&group.total),
            rows: group.rows.iter().map(WireIsRow::from).collect(),
        }
    }
}

/// A subtotal ruled beneath the box it follows.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireIsSubtotal {
    /// `"grossProfit" | "ebitda" | "operatingIncome" | "pretaxIncome"`.
    ///
    /// camelCase, not the kebab-case `WarningKind` uses for `"missing-basis"`.
    /// The plan pins the section codes (all single words, so identical either
    /// way) and says only "lowercase" about the rest, which does not decide a
    /// multi-word value. camelCase is chosen because it is what every other key
    /// on this wire is, and because these values are read straight into the TS
    /// union `IsSubtotalKind` in `web/src/lib/reports/types.ts` — whose
    /// `decodeEnum` allow-list THROWS on an unrecognised value, so the two
    /// spellings are not merely untidy but mutually exclusive.
    kind: &'static str,
    label: String,
    total: WireAmounts,
}

impl From<&IsSubtotal> for WireIsSubtotal {
    fn from(subtotal: &IsSubtotal) -> Self {
        Self {
            kind: match subtotal.kind {
                IsSubtotalKind::GrossProfit => "grossProfit",
                IsSubtotalKind::Ebitda => "ebitda",
                IsSubtotalKind::OperatingIncome => "operatingIncome",
                IsSubtotalKind::PretaxIncome => "pretaxIncome",
            },
            label: subtotal.label.clone(),
            total: WireAmounts::from(&subtotal.total),
        }
    }
}

/// One box of the statement.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireIsSection {
    /// `"revenue" | "cogs" | "opex" | "depreciation" | "interest" | "tax" | "other"`.
    kind: &'static str,
    title: String,
    groups: Vec<WireIsGroup>,
    total: WireAmounts,
    /// Subtotals printed AFTER this box; `[]` for most of them.
    trailing: Vec<WireIsSubtotal>,
}

impl From<&IsSection> for WireIsSection {
    fn from(section: &IsSection) -> Self {
        Self {
            kind: match section.kind {
                IsSectionKind::Revenue => "revenue",
                IsSectionKind::Cogs => "cogs",
                IsSectionKind::Opex => "opex",
                IsSectionKind::Depreciation => "depreciation",
                IsSectionKind::Interest => "interest",
                IsSectionKind::Tax => "tax",
                IsSectionKind::Other => "other",
            },
            title: section.title.clone(),
            groups: section.groups.iter().map(WireIsGroup::from).collect(),
            total: WireAmounts::from(&section.total),
            trailing: section.trailing.iter().map(WireIsSubtotal::from).collect(),
        }
    }
}

/// The grouped, valued income statement with its adaptive GAAP ladder.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireIncomeStatementReport {
    from: String,
    to: String,
    /// The window the comparison column covers, or an EXPLICIT `null` when
    /// `compare=none`.
    ///
    /// Deliberately not omitted, unlike [`WireAmounts::prior`], and the two
    /// conventions are load-bearing rather than inconsistent. This key is the
    /// SWITCH: the client reads it first, derives "am I comparing?" from it, and
    /// then requires every `Amounts` in the tree to agree. A switch has to be
    /// present to be read, so it is `DateRange | null`; the figures it governs
    /// are absent-or-present, so a missing one is caught instead of defaulting
    /// to an empty period.
    prior: Option<WireDateRange>,
    /// The commodity everything is valued into — `null` on the cost and
    /// unvalued bases. Kept (not omitted) so the decoder always sees the key.
    base: Option<String>,
    /// The basis the numbers are on, echoed back from the request.
    value: &'static str,
    /// Whether the GAAP ladder materialised — which is also what decides whether
    /// `opex` is titled "Expenses" or "Operating expenses".
    multi_step: bool,
    /// Non-empty sections only, in ladder order.
    sections: Vec<WireIsSection>,
    net_income: WireAmounts,
    /// Always present, because the unpriced banner keys off it on every request.
    meta: WireReportMeta,
}

impl WireIncomeStatementReport {
    /// `value` is the request's own spelling, so the response says which basis
    /// produced the numbers it carries.
    fn new(report: &IncomeStatementReport, value: &'static str) -> Self {
        Self {
            from: report.from.clone(),
            to: report.to.clone(),
            prior: report.prior.as_ref().map(WireDateRange::from),
            base: report.base.as_ref().map(|base| base.0.clone()),
            value,
            multi_step: report.multi_step,
            sections: report.sections.iter().map(WireIsSection::from).collect(),
            net_income: WireAmounts::from(&report.net_income),
            meta: WireReportMeta::from(&report.meta),
        }
    }
}

/// One row of a period report: one `MixedAmount` per bucket.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePeriodRow {
    account: String,
    depth: usize,
    values: Vec<WireMixed>,
}

impl From<&PeriodRow> for WirePeriodRow {
    fn from(row: &PeriodRow) -> Self {
        Self {
            account: row.account.clone(),
            depth: row.depth,
            values: row.values.iter().map(wire_mixed).collect(),
        }
    }
}

/// Extra result info (currently only unpriced commodities in net worth).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireReportMeta {
    unpriced: Vec<String>,
}

impl From<&ReportMeta> for WireReportMeta {
    fn from(meta: &ReportMeta) -> Self {
        Self {
            unpriced: meta.unpriced.iter().map(|c| c.0.clone()).collect(),
        }
    }
}

/// Cash flow / net worth: one column per bucket, oldest → newest.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WirePeriodReport {
    buckets: Vec<String>,
    rows: Vec<WirePeriodRow>,
    totals: Vec<WireMixed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<WireReportMeta>,
}

impl From<&PeriodReport> for WirePeriodReport {
    fn from(report: &PeriodReport) -> Self {
        Self {
            buckets: report.buckets.clone(),
            rows: report.rows.iter().map(WirePeriodRow::from).collect(),
            totals: report.totals.iter().map(wire_mixed).collect(),
            meta: report.meta.as_ref().map(WireReportMeta::from),
        }
    }
}

/// One account × bucket budget cell. `goal` is `null` when the account has no
/// goal (e.g. `<unbudgeted>`); an empty object `{}` is a budgeted account with no
/// goal in that bucket.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBudgetCell {
    actual: WireMixed,
    goal: Option<WireMixed>,
}

impl From<&BudgetCell> for WireBudgetCell {
    fn from(cell: &BudgetCell) -> Self {
        Self {
            actual: wire_mixed(&cell.actual),
            goal: cell.goal.as_ref().map(wire_mixed),
        }
    }
}

/// One budget row: an account and its per-bucket cells.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBudgetRow {
    account: String,
    depth: usize,
    cells: Vec<WireBudgetCell>,
}

impl From<&BudgetRow> for WireBudgetRow {
    fn from(row: &BudgetRow) -> Self {
        Self {
            account: row.account.clone(),
            depth: row.depth,
            cells: row.cells.iter().map(WireBudgetCell::from).collect(),
        }
    }
}

/// A budget report: bucket keys, rows, and a grand-total row of cells.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBudgetReport {
    buckets: Vec<String>,
    rows: Vec<WireBudgetRow>,
    totals: Vec<WireBudgetCell>,
}

impl From<&BudgetReport> for WireBudgetReport {
    fn from(report: &BudgetReport) -> Self {
        Self {
            buckets: report.buckets.clone(),
            rows: report.rows.iter().map(WireBudgetRow::from).collect(),
            totals: report.totals.iter().map(WireBudgetCell::from).collect(),
        }
    }
}

// ===========================================================================
// Wire representation of the insights dashboard
// ===========================================================================

/// The resolved comparison window (all camelCase ISO dates).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireInsightsPeriod {
    start: String,
    mid: String,
    end: String,
    prev_start: String,
    prev_end: String,
    curr_start: String,
    curr_end: String,
}

impl From<&InsightsPeriod> for WireInsightsPeriod {
    fn from(period: &InsightsPeriod) -> Self {
        Self {
            start: period.start.clone(),
            mid: period.mid.clone(),
            end: period.end.clone(),
            prev_start: period.prev_start.clone(),
            prev_end: period.prev_end.clone(),
            curr_start: period.curr_start.clone(),
            curr_end: period.curr_end.clone(),
        }
    }
}

/// A current/previous metric with its exact change and a base-commodity percent.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireMetricDelta {
    current: WireMixed,
    previous: WireMixed,
    delta: WireMixed,
    pct: Option<f64>,
}

impl From<&MetricDelta> for WireMetricDelta {
    fn from(metric: &MetricDelta) -> Self {
        Self {
            current: wire_mixed(&metric.current),
            previous: wire_mixed(&metric.previous),
            delta: wire_mixed(&metric.delta),
            pct: metric.pct,
        }
    }
}

/// Cost-of-living totals + month counts (the SPA averages for display).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCostOfLiving {
    current_total: WireMixed,
    previous_total: WireMixed,
    months_current: u32,
    months_previous: u32,
}

impl From<&CostOfLiving> for WireCostOfLiving {
    fn from(col: &CostOfLiving) -> Self {
        Self {
            current_total: wire_mixed(&col.current_total),
            previous_total: wire_mixed(&col.previous_total),
            months_current: col.months_current,
            months_previous: col.months_previous,
        }
    }
}

/// One period's portfolio performance in the base commodity.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePerfPoint {
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
}

impl From<&PerfPoint> for WirePerfPoint {
    fn from(point: &PerfPoint) -> Self {
        Self {
            gain: point.gain.map(WireDec::from),
            gain_pct: point.gain_pct,
        }
    }
}

/// A leaf-account change row (Boxes 7 & 9). `kind` is `"changed" | "new" | "ended"`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireChangeRow {
    account: String,
    current: WireDec,
    previous: WireDec,
    delta: WireDec,
    pct: Option<f64>,
    kind: &'static str,
}

impl From<&ChangeRow> for WireChangeRow {
    fn from(row: &ChangeRow) -> Self {
        Self {
            account: row.account.clone(),
            current: row.current.into(),
            previous: row.previous.into(),
            delta: row.delta.into(),
            pct: row.pct,
            kind: match row.kind {
                ChangeKind::Changed => "changed",
                ChangeKind::Ended => "ended",
            },
        }
    }
}

/// A stock mover row (Box 8).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireMoverRow {
    symbol: String,
    name: String,
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
    /// The window-start value fell back to purchase cost (no market price) — the
    /// move then approximates the all-time gain. Surfaced as a caveat in the UI.
    start_estimated: bool,
}

impl From<&MoverRow> for WireMoverRow {
    fn from(row: &MoverRow) -> Self {
        Self {
            symbol: row.symbol.clone(),
            name: row.name.clone(),
            gain: row.gain.map(WireDec::from),
            gain_pct: row.gain_pct,
            start_estimated: row.start_estimated,
        }
    }
}

/// A top-transaction row (Box 10).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireTopTxn {
    index: u32,
    date: String,
    description: String,
    amount: WireDec,
}

impl From<&TopTxn> for WireTopTxn {
    fn from(txn: &TopTxn) -> Self {
        Self {
            index: txn.index,
            date: txn.date.clone(),
            description: txn.description.clone(),
            amount: txn.amount.into(),
        }
    }
}

/// The full insights dashboard payload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireInsightsReport {
    period: WireInsightsPeriod,
    base: String,
    /// Earliest transaction date in the journal — lets the SPA warn when the
    /// previous period is only partly covered by the data.
    journal_start: Option<String>,
    revenue: WireMetricDelta,
    expenses: WireMetricDelta,
    net_worth: WireMetricDelta,
    cost_of_living: WireCostOfLiving,
    investment: WireInvestmentPerf,
    cash_balance: WireMetricDelta,
    expense_changes: Vec<WireChangeRow>,
    revenue_changes: Vec<WireChangeRow>,
    movers: Vec<WireMoverRow>,
    top_txns: Vec<WireTopTxn>,
    /// Commodities the valuation had to skip, exactly as `/api/reports/networth`
    /// reports them. Without it the dashboard showed an UNDERSTATED net worth
    /// with no indication at all, while the net-worth endpoint warned about the
    /// same journal (DRY-5). Omitted when nothing is unpriced, so a clean
    /// journal's payload is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<WireReportMeta>,
}

/// Investment performance for both periods.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireInvestmentPerf {
    current: WirePerfPoint,
    previous: WirePerfPoint,
}

impl From<&InvestmentPerf> for WireInvestmentPerf {
    fn from(perf: &InvestmentPerf) -> Self {
        Self {
            current: (&perf.current).into(),
            previous: (&perf.previous).into(),
        }
    }
}

impl From<&InsightsReport> for WireInsightsReport {
    fn from(report: &InsightsReport) -> Self {
        Self {
            period: (&report.period).into(),
            base: report.base.clone(),
            journal_start: report.journal_start.clone(),
            revenue: (&report.revenue).into(),
            expenses: (&report.expenses).into(),
            net_worth: (&report.net_worth).into(),
            cost_of_living: (&report.cost_of_living).into(),
            investment: (&report.investment).into(),
            cash_balance: (&report.cash_balance).into(),
            expense_changes: report
                .expense_changes
                .iter()
                .map(WireChangeRow::from)
                .collect(),
            revenue_changes: report
                .revenue_changes
                .iter()
                .map(WireChangeRow::from)
                .collect(),
            movers: report.movers.iter().map(WireMoverRow::from).collect(),
            top_txns: report.top_txns.iter().map(WireTopTxn::from).collect(),
            meta: report.meta.as_ref().map(WireReportMeta::from),
        }
    }
}

// ===========================================================================
// Wire representation of the subscriptions report
// ===========================================================================

/// One detected recurring charge. `cadence` is `"monthly" | "annual"`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSubscription {
    payee: String,
    cadence: &'static str,
    typical_amount: WireDec,
    annualized_cost: WireDec,
    occurrences: usize,
    first_seen: String,
    last_seen: String,
    next_expected: String,
    accounts: Vec<String>,
    /// Hand-added via a `subscription:true` tag rather than detected.
    manual: bool,
}

impl From<&Subscription> for WireSubscription {
    fn from(subscription: &Subscription) -> Self {
        Self {
            payee: subscription.payee.clone(),
            cadence: match subscription.cadence {
                Cadence::Monthly => "monthly",
                Cadence::Annual => "annual",
            },
            typical_amount: subscription.typical_amount.into(),
            annualized_cost: subscription.annualized_cost.into(),
            occurrences: subscription.occurrences,
            first_seen: subscription.first_seen.clone(),
            last_seen: subscription.last_seen.clone(),
            next_expected: subscription.next_expected.clone(),
            accounts: subscription.accounts.clone(),
            manual: subscription.manual,
        }
    }
}

/// Detected subscriptions split by cadence, each sorted by annual cost desc.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSubscriptionsReport {
    as_of: String,
    lookback_start: String,
    monthly: Vec<WireSubscription>,
    annual: Vec<WireSubscription>,
}

impl From<&SubscriptionsReport> for WireSubscriptionsReport {
    fn from(report: &SubscriptionsReport) -> Self {
        Self {
            as_of: report.as_of.clone(),
            lookback_start: report.lookback_start.clone(),
            monthly: report.monthly.iter().map(WireSubscription::from).collect(),
            annual: report.annual.iter().map(WireSubscription::from).collect(),
        }
    }
}

// ===========================================================================
// Wire representation of the holdings result types
// ===========================================================================

/// A holding's resolved price → `{qty, date, source}` (`source` kebab-free:
/// `"directive"` | `"cost"`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHoldingPrice {
    qty: WireDec,
    date: String,
    source: &'static str,
}

impl From<&HoldingPrice> for WireHoldingPrice {
    fn from(price: &HoldingPrice) -> Self {
        Self {
            qty: price.qty.into(),
            date: price.date.clone(),
            source: match price.source {
                PriceSource::Directive => "directive",
                PriceSource::Cost => "cost",
            },
        }
    }
}

/// One holding row. Null-valued keys (basis/price/gain/…) are kept (not omitted),
/// matching the TS `Holding` shape.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHolding {
    symbol: String,
    name: String,
    accounts: Vec<String>,
    shares: WireDec,
    basis: Option<WireDec>,
    first_basis_date: Option<String>,
    price: Option<WireHoldingPrice>,
    market_value: Option<WireDec>,
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
}

impl From<&Holding> for WireHolding {
    fn from(holding: &Holding) -> Self {
        Self {
            symbol: holding.symbol.clone(),
            name: holding.name.clone(),
            accounts: holding.accounts.clone(),
            shares: holding.shares.into(),
            basis: holding.basis.map(WireDec::from),
            first_basis_date: holding.first_basis_date.clone(),
            price: holding.price.as_ref().map(WireHoldingPrice::from),
            market_value: holding.market_value.map(WireDec::from),
            gain: holding.gain.map(WireDec::from),
            gain_pct: holding.gain_pct,
        }
    }
}

/// A scope-local warning → `{symbol, kind, message}` (`kind` kebab-case, matching
/// the TS union: `"missing-basis"` | `"negative-shares"` | `"unpriced"`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireWarning {
    symbol: String,
    kind: &'static str,
    message: String,
}

impl From<&HoldingsWarning> for WireWarning {
    fn from(warning: &HoldingsWarning) -> Self {
        Self {
            symbol: warning.symbol.clone(),
            kind: match warning.kind {
                WarningKind::MissingBasis => "missing-basis",
                WarningKind::NegativeShares => "negative-shares",
                WarningKind::Unpriced => "unpriced",
            },
            message: warning.message.clone(),
        }
    }
}

/// Portfolio totals: `marketValue` always present; `basis`/`gain`/`gainPct`
/// null when refused.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHoldingsTotals {
    market_value: WireDec,
    basis: Option<WireDec>,
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
}

impl From<&HoldingsTotals> for WireHoldingsTotals {
    fn from(totals: &HoldingsTotals) -> Self {
        Self {
            market_value: totals.market_value.into(),
            basis: totals.basis.map(WireDec::from),
            gain: totals.gain.map(WireDec::from),
            gain_pct: totals.gain_pct,
        }
    }
}

/// The full holdings report.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireHoldingsReport {
    as_of: String,
    base: String,
    holdings: Vec<WireHolding>,
    accounts: Vec<String>,
    totals: WireHoldingsTotals,
    top_gainers: Vec<WireHolding>,
    top_losers: Vec<WireHolding>,
    warnings: Vec<WireWarning>,
}

impl From<&HoldingsReport> for WireHoldingsReport {
    fn from(report: &HoldingsReport) -> Self {
        Self {
            as_of: report.as_of.clone(),
            base: report.base.clone(),
            holdings: report.holdings.iter().map(WireHolding::from).collect(),
            accounts: report.accounts.clone(),
            totals: (&report.totals).into(),
            top_gainers: report.top_gainers.iter().map(WireHolding::from).collect(),
            top_losers: report.top_losers.iter().map(WireHolding::from).collect(),
            warnings: report.warnings.iter().map(WireWarning::from).collect(),
        }
    }
}

/// One point of the holdings trend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHoldingsPoint {
    date: String,
    bucket: String,
    label: String,
    market_value: WireDec,
    basis: Option<WireDec>,
}

impl From<&HoldingsPoint> for WireHoldingsPoint {
    fn from(point: &HoldingsPoint) -> Self {
        Self {
            date: point.date.clone(),
            bucket: point.bucket.clone(),
            label: point.label.clone(),
            market_value: point.market_value.into(),
            basis: point.basis.map(WireDec::from),
        }
    }
}

/// The holdings-over-time series.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireHoldingsSeries {
    base: String,
    points: Vec<WireHoldingsPoint>,
    has_basis: bool,
}

impl From<&HoldingsSeries> for WireHoldingsSeries {
    fn from(series: &HoldingsSeries) -> Self {
        Self {
            base: series.base.clone(),
            points: series.points.iter().map(WireHoldingsPoint::from).collect(),
            has_basis: series.has_basis,
        }
    }
}

/// One Other-holdings row: an ACCOUNT you own that is neither a security nor
/// cash. `commodities` is the balance as written, so the UI can print `1 HOUSE`
/// beside the dollar value. Null-valued keys are kept, matching `WireHolding`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireOtherHolding {
    account: String,
    name: String,
    commodities: WireMixed,
    value: Option<WireDec>,
    cost: Option<WireDec>,
    change: Option<WireDec>,
    change_pct: Option<f64>,
}

impl From<&OtherHolding> for WireOtherHolding {
    fn from(holding: &OtherHolding) -> Self {
        Self {
            account: holding.account.clone(),
            name: holding.name.clone(),
            commodities: wire_mixed(&holding.commodities),
            value: holding.value.map(WireDec::from),
            cost: holding.cost.map(WireDec::from),
            change: holding.change.map(WireDec::from),
            change_pct: holding.change_pct,
        }
    }
}

/// An Other-holdings warning → `{account, kind, message}`. Keyed by ACCOUNT
/// rather than by symbol, because that is what a row is here.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireOtherWarning {
    account: String,
    kind: &'static str,
    message: String,
}

impl From<&OtherHoldingsWarning> for WireOtherWarning {
    fn from(warning: &OtherHoldingsWarning) -> Self {
        Self {
            account: warning.account.clone(),
            kind: match warning.kind {
                OtherWarningKind::Unpriced => "unpriced",
                OtherWarningKind::UnpricedCost => "unpriced-cost",
            },
            message: warning.message.clone(),
        }
    }
}

/// Other-holdings totals: `value` always present; the rest null when refused.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireOtherHoldingsTotals {
    value: WireDec,
    cost: Option<WireDec>,
    change: Option<WireDec>,
    change_pct: Option<f64>,
}

impl From<&OtherHoldingsTotals> for WireOtherHoldingsTotals {
    fn from(totals: &OtherHoldingsTotals) -> Self {
        Self {
            value: totals.value.into(),
            cost: totals.cost.map(WireDec::from),
            change: totals.change.map(WireDec::from),
            change_pct: totals.change_pct,
        }
    }
}

/// The full Other-holdings report.
///
/// The matching TREND has no wire type of its own: it is a
/// [`WireHoldingsSeries`], byte for byte, so the SPA decodes both trends with
/// one function and draws them with one chart component.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireOtherHoldingsReport {
    as_of: String,
    base: String,
    holdings: Vec<WireOtherHolding>,
    accounts: Vec<String>,
    totals: WireOtherHoldingsTotals,
    warnings: Vec<WireOtherWarning>,
}

impl From<&OtherHoldingsReport> for WireOtherHoldingsReport {
    fn from(report: &OtherHoldingsReport) -> Self {
        Self {
            as_of: report.as_of.clone(),
            base: report.base.clone(),
            holdings: report.holdings.iter().map(WireOtherHolding::from).collect(),
            accounts: report.accounts.clone(),
            totals: (&report.totals).into(),
            warnings: report.warnings.iter().map(WireOtherWarning::from).collect(),
        }
    }
}

// ===========================================================================
// Query params, defaults, and helpers
// ===========================================================================

/// Current UTC date as `YYYY-MM-DD`, from the system clock.
///
/// The report engine is deliberately clock-free (see `reports::periods`);
/// "today" is a server-side request default only, so it lives here rather than
/// in `ledgeline-core`, and needs no third-party date dependency.
fn today_utc() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    // The calendar itself lives in `reports::periods` — this file used to carry a
    // verbatim copy of Howard Hinnant's `civil_from_days` (DRY-2). The clock read
    // stays here, because `reports` is deliberately clock-free.
    periods::iso_from_days(days)
}

/// Parse a report interval, defaulting to monthly when absent. Returns a `400`
/// for an unrecognized value.
fn parse_interval(raw: Option<&str>) -> Result<Interval, AppError> {
    match raw {
        None => Ok(Interval::Monthly),
        Some("daily") => Ok(Interval::Daily),
        Some("weekly") => Ok(Interval::Weekly),
        Some("monthly") => Ok(Interval::Monthly),
        Some("quarterly") => Ok(Interval::Quarterly),
        Some("yearly") => Ok(Interval::Yearly),
        Some(other) => Err(AppError::BadRequest(format!(
            "unknown interval '{other}' (expected daily|weekly|monthly|quarterly|yearly)"
        ))),
    }
}

/// Validate a bucket count, defaulting to [`DEFAULT_COUNT`] when absent. Returns
/// a `400` for anything outside `1..=MAX_BUCKETS`.
///
/// This REJECTS rather than clamps, deliberately. `count` drives how many
/// periods a chart plots, so silently turning `count=1000000` into 1200 would
/// answer a question the caller did not ask with a plausible-looking chart —
/// exactly the "displays a plausible number instead of an error" failure mode
/// this review is against. `count=0` is likewise a caller bug, not a request for
/// an empty chart. A 400 with the accepted range is the honest answer, and it
/// matches [`parse_interval`]'s established handling of an unrecognized value.
///
/// Note `count` deserializes as `usize`, so a negative or non-integer value is
/// already rejected by serde as a `400` before reaching here; this covers the
/// in-range-for-`usize`-but-nonsensical values (`0`, `u64::MAX`) that reached
/// `Vec::with_capacity` and aborted the request with a `capacity overflow`
/// panic. See SEC-2.
fn parse_count(raw: Option<usize>) -> Result<usize, AppError> {
    match raw {
        None => Ok(DEFAULT_COUNT),
        Some(count) if (1..=MAX_BUCKETS).contains(&count) => Ok(count),
        Some(other) => Err(AppError::BadRequest(format!(
            "count {other} is out of range (expected 1..={MAX_BUCKETS})"
        ))),
    }
}

/// The deepest account a `depth` param may ask for.
///
/// Nothing in the engine breaks past it — `at_depth` is a filter, and hledger
/// imposes no limit of its own — so this is not a crash guard. It is
/// [`parse_count`]'s judgement applied to the other clamp: no chart of accounts
/// is a hundred segments deep, so `depth=1000000` is a typo, and serving it the
/// same report `depth=100` would give hides the typo behind a plausible-looking
/// answer.
const MAX_DEPTH: usize = 100;

/// Validate an account-depth clamp, passing ABSENT through as "no clamp".
///
/// `depth=0` stays legal: it is hledger's "totals only", and every total this
/// crate serves is summed from the unclamped accounts, so it reports figures
/// rather than zeros (RPT-4). A negative or non-integer value is already a `400`
/// from serde before it reaches here.
///
/// Because `0` is already spoken for, "unlimited" cannot be a depth VALUE at
/// all — so it is the absence of one, which is also what an omitted clamp
/// naturally reads as. The alternative, some large sentinel, would make the
/// contract "a number nobody's chart of accounts exceeds" instead of a stated
/// fact, and `MAX_DEPTH` below exists precisely to reject numbers like that as
/// the typos they usually are.
///
/// Only the NEW `/api/reports/balancesheet/grouped` route runs this. The older
/// report routes still take `depth` unchecked; tightening them would change
/// answers the committed `fixtures/native/v1/` goldens pin, so that is a
/// separate change with its own regeneration.
fn parse_depth(raw: Option<usize>) -> Result<Option<usize>, AppError> {
    match raw {
        None => Ok(None),
        Some(depth) if depth <= MAX_DEPTH => Ok(Some(depth)),
        Some(other) => Err(AppError::BadRequest(format!(
            "depth {other} is out of range (expected 0..={MAX_DEPTH})"
        ))),
    }
}

/// Parse the grouped balance sheet's `value` basis, defaulting to market.
///
/// Returns the spelling alongside the basis so the handler can echo back
/// exactly what it computed from ONE table — the request and the response
/// cannot end up disagreeing about which basis produced the numbers.
fn parse_valuation(raw: Option<&str>) -> Result<(Valuation, &'static str), AppError> {
    match raw {
        None | Some("market") => Ok((Valuation::Market, "market")),
        Some("cost") => Ok((Valuation::Cost, "cost")),
        Some("none") => Ok((Valuation::None, "none")),
        Some(other) => Err(AppError::BadRequest(format!(
            "unknown value '{other}' (expected market|cost|none)"
        ))),
    }
}

/// Parse the grouped income statement's `compare` mode, defaulting to the
/// preceding equal-length window.
///
/// Rejects rather than defaults, exactly as [`parse_valuation`] does: `compare`
/// decides whether a whole column of figures is on the page, so quietly serving
/// the default for `compare=yoy` would answer a question the caller did not ask.
fn parse_compare(raw: Option<&str>) -> Result<bool, AppError> {
    match raw {
        None | Some("previous") => Ok(true),
        Some("none") => Ok(false),
        Some(other) => Err(AppError::BadRequest(format!(
            "unknown compare '{other}' (expected previous|none)"
        ))),
    }
}

/// Validate one date and normalize it to ISO `YYYY-MM-DD`, or return the reason
/// it is not a date. [`parse_date`] wraps the reason in a `400`.
///
/// **Why this has to exist (RPT-4).** The whole report engine orders and windows
/// dates by comparing the strings themselves (`reports::aggregate`,
/// `reports::periods`), which is only chronological because every date it has
/// ever seen came out of the journal parser as a fixed-width `YYYY-MM-DD`. A
/// query param bypassed that parser entirely, so a hand-typed `?end=2026-7-1`
/// stayed `"2026-7-1"` — a string that sorts ABOVE `"2026-12-31"` and whose
/// month slice is `"7-"`, i.e. bucket `2026-00`. Nothing errored: the balance
/// sheet quietly served the all-time total, the period reports grew a garbage
/// trailing bucket, and `?asOf=` (empty) sorted below everything and served an
/// empty report with a `200`. Validating the shape here restores the invariant
/// the engine assumes rather than teaching every comparison to distrust it.
///
/// **What is accepted** is exactly what hledger's own `-b`/`-e` accept, verified
/// against `hledger 1.52`: a four-digit year, `-`, `/` or `.` separators, and
/// unpadded month/day components. `2026-7-1`, `2026/7/1` and `2026.7.1` all
/// normalize to `2026-07-01` — the same normalization Ledgeline's journal parser
/// (`parse::normalize_date`) already applies to journal dates, so a date that is
/// legal INSIDE a journal is also legal in a URL, and a URL answer agrees with
/// `hledger -e <same-date>`. hledger rejects `26-07-01`, `2026-13-01`,
/// `2026-02-30` and `garbage`; so do we.
fn normalize_iso_date(raw: &str) -> Result<String, String> {
    const SHAPE: &str = "expected YYYY-MM-DD (a four-digit year, then month and day, \
                         separated by `-`, `/` or `.`)";

    let parts: Vec<&str> = raw.split(['-', '/', '.']).collect();
    let [year_src, month_src, day_src] = parts[..] else {
        return Err(SHAPE.to_string());
    };
    // Digits only, and width-checked: a 4-digit year is what keeps the ISO form
    // fixed-width, which is what makes the engine's lexical ordering correct.
    let component = |src: &str, widths: std::ops::RangeInclusive<usize>| -> Option<i32> {
        (widths.contains(&src.len()) && src.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| src.parse().ok())
            .flatten()
    };
    let (Some(year), Some(month), Some(day)) = (
        component(year_src, 4..=4),
        component(month_src, 1..=2),
        component(day_src, 1..=2),
    ) else {
        return Err(SHAPE.to_string());
    };
    // The day must be valid FOR ITS MONTH: `2026-02-30` and `2023-02-29` are
    // shaped like dates but are not dates, and hledger rejects both. Mirrors the
    // PARSE-6 check the journal parser applies (`parse::normalize_date`).
    if !(1..=12).contains(&month) {
        return Err(format!("there is no month {month}"));
    }
    if !(1..=days_in_month(year, month)).contains(&day) {
        return Err(format!(
            "month {month:02} of {year:04} has no day {day} (it has {})",
            days_in_month(year, month)
        ));
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

/// The number of days in `month` (1-12) of `year`.
///
/// Duplicated from `ledgeline_core::parse` (and, in `i64`, from
/// `reports::periods`) only because both copies are private to their modules and
/// neither crate exports a date validator. Folding all three into one place is
/// the `IsoDate` newtype of DRY-2; this copy is confined to the HTTP boundary
/// until then.
fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The proleptic Gregorian leap-year rule hledger's `Data.Time` calendar uses.
fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Validate `raw` as an ISO date, naming `field` in the `400` message.
fn checked_date(field: &str, raw: &str) -> Result<String, AppError> {
    normalize_iso_date(raw)
        .map_err(|reason| AppError::BadRequest(format!("invalid {field} date '{raw}': {reason}")))
}

/// Validate a date param, falling back to `default` when it is absent.
///
/// An EXPLICIT but malformed value — including an explicit empty one, which used
/// to sort below every real date and produce an empty report — is a `400`,
/// matching [`parse_interval`] and [`parse_count`]: the caller asked a question
/// we cannot answer, and a report for some other date is not the answer. (The
/// SPA's `queryString` omits empty params rather than sending them, so this
/// costs it nothing.)
fn parse_date(
    field: &str,
    raw: Option<String>,
    default: impl FnOnce() -> String,
) -> Result<String, AppError> {
    match raw {
        None => Ok(default()),
        Some(value) => checked_date(field, &value),
    }
}

/// Validate an OPTIONAL date param, where absent AND empty both mean `None`.
///
/// Only `gainSince` uses this: an empty `gainSince` is its documented sentinel
/// for "all-time gain" (see [`holdings`]), so unlike the defaulted params above
/// an empty value here is a real request, not a malformed date.
fn parse_opt_date(field: &str, raw: Option<String>) -> Result<Option<String>, AppError> {
    match raw.filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => checked_date(field, &value).map(Some),
    }
}

/// The "biggest change" floor used when `changeMin` is absent: $10 in the base
/// commodity.
const DEFAULT_CHANGE_MIN: Dec = Dec::new(1000, 2);

/// Validate a `changeMin` magnitude, defaulting to [`DEFAULT_CHANGE_MIN`] when
/// absent or empty. Returns a `400` for anything [`Dec::parse`] rejects.
///
/// This REJECTS rather than falling back, for the same reason as [`parse_count`]:
/// `changeMin` is the floor that decides which rows appear in the "biggest
/// change" boxes, so quietly substituting $10 for a value the caller wrote (say
/// `changeMin=$500`, or a comma-decimal `changeMin=500,00`) answers a different
/// question with a shorter, plausible-looking list and no way to tell.
///
/// Note the decimal mark is `.`, so `,`/`_`/space are digit-group separators —
/// `changeMin=1,000` is a well-formed 1000 and stays accepted.
fn parse_change_min(raw: Option<&str>) -> Result<Dec, AppError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(DEFAULT_CHANGE_MIN),
        Some(value) => Dec::parse(value, '.')
            .map_err(|err| AppError::BadRequest(format!("invalid changeMin '{value}': {err}"))),
    }
}

/// Parse a holdings scope mode, defaulting to `include` when absent. Returns a
/// `400` for an unrecognized value.
fn parse_mode(raw: Option<&str>) -> Result<ScopeMode, AppError> {
    match raw {
        None | Some("include") => Ok(ScopeMode::Include),
        Some("exclude") => Ok(ScopeMode::Exclude),
        Some(other) => Err(AppError::BadRequest(format!(
            "unknown mode '{other}' (expected include|exclude)"
        ))),
    }
}

/// Which Holdings tab a request serves — and so which held set its `valueIn`
/// (and demoted `D` default) admission test must measure price coverage over.
///
/// The two tabs hold DISJOINT rows (`holdings::engine::scope_predicate` drops
/// every `holdings: other`/`none` account), so `/api/holdings/other[/series]`
/// validating through the stocks-scoped `prices_any_held` answered a question
/// about rows those endpoints never serve: a commodity pricing every Other row
/// was a 400 because it priced no stock, and a journal with no stocks at all
/// vacuously admitted any typo — the plausible-zero HOLD-3 exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldingsTab {
    /// `/api/holdings[/series]` — commodity-keyed securities.
    Stocks,
    /// `/api/holdings/other[/series]` — account-keyed everything else.
    Other,
}

/// Resolve the commodity a holdings request is valued in, in precedence order:
/// the explicit `valueIn` param, then the journal's own `D` default-commodity
/// directive, then `None` — which hands the choice to the engine (see
/// `holdings::valuation_base`, which picks the candidate that actually prices
/// the portfolio).
///
/// An explicit `valueIn` that prices NONE of the in-scope holdings is a `400`,
/// matching [`parse_interval`]/[`parse_count`]/[`parse_date`]: a typo, or a real
/// commodity with no route to the portfolio, would otherwise be answered with an
/// all-zero total and one `unpriced` warning per row — a plausible-looking
/// number in place of an error, which is exactly the failure HOLD-3 is about.
///
/// The `D` fallback is held to the same test but DEMOTED rather than rejected:
/// nobody asked for it on this request, so a journal whose declared commodity
/// happens not to price its securities falls through to the engine's own choice
/// instead of being refused. `scope` is the request's scope with `value_in` not
/// yet filled in; only its accounts/mode/`as_of` are read.
///
/// `tab` picks WHICH held set the test (and the `D` demotion, which is the same
/// test) measures coverage over — see [`HoldingsTab`].
fn resolve_value_in(
    journal: &Journal,
    scope: &HoldingsScope,
    tab: HoldingsTab,
    raw: Option<&str>,
) -> Result<Option<Commodity>, AppError> {
    let prices_anything = |target: &Commodity| -> Result<bool, AppError> {
        let admits = match tab {
            HoldingsTab::Stocks => prices_any_held(
                &journal.transactions,
                &journal.prices,
                &journal.accounts,
                scope,
                target,
            )?,
            HoldingsTab::Other => prices_any_held_other(
                &journal.transactions,
                &journal.prices,
                &journal.accounts,
                scope,
                target,
            )?,
        };
        Ok(admits)
    };
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(symbol) => {
            let target = Commodity(symbol.to_string());
            if prices_anything(&target)? {
                Ok(Some(target))
            } else {
                Err(AppError::BadRequest(format!(
                    "cannot value these holdings in '{symbol}': no price directive or cost \
                     annotation connects any holding in scope to it"
                )))
            }
        }
        None => match journal.default_commodity.clone() {
            Some(declared) if prices_anything(&declared)? => Ok(Some(declared)),
            _ => Ok(None),
        },
    }
}

/// [`resolve_value_in`]'s contract for the two grouped statements, with the
/// admission question asked by `prices_anything` — which measures price
/// coverage over the requesting REPORT's own rows
/// ([`prices_any_on_sheet`] / [`prices_any_on_statement`]), the same way each
/// Holdings tab measures its own held set (see [`HoldingsTab`] for why a
/// borrowed set answers the wrong question).
///
/// Same precedence, same demotion: an explicit `valueIn` (trimmed, as holdings
/// trims — `valueIn=%20$` is `$`, not the unpriceable `Commodity(" $")`) that
/// prices nothing the report values is a `400` naming the value, because the
/// alternative is a `200` echoing `base: "USDD"` over numbers the basis never
/// touched, with every commodity in `meta.unpriced`. The journal's own `D`
/// default is held to the same test but DEMOTED rather than rejected — nobody
/// asked for it on this request — and `None` hands the choice to the report's
/// own default, the price table's base commodity.
///
/// The refusal names the statement (`what`) and, unlike holdings', mentions
/// only price directives: the grouped statements value through explicit `P`
/// directives alone (see [`balance_sheet_grouped`]'s valuation notes), so a
/// cost annotation genuinely cannot connect anything here.
fn resolve_grouped_value_in(
    journal: &Journal,
    what: &str,
    raw: Option<&str>,
    prices_anything: impl Fn(&Commodity) -> Result<bool, ReportError>,
) -> Result<Option<Commodity>, AppError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(symbol) => {
            let target = Commodity(symbol.to_string());
            if prices_anything(&target)? {
                Ok(Some(target))
            } else {
                Err(AppError::BadRequest(format!(
                    "cannot value {what} in '{symbol}': no price directive connects any \
                     commodity on it to it"
                )))
            }
        }
        None => match journal.default_commodity.clone() {
            Some(declared) if prices_anything(&declared)? => Ok(Some(declared)),
            _ => Ok(None),
        },
    }
}

/// The scope shared by a tab's report and series endpoints: the same account
/// selection, date and valuation commodity, validated the same way — with the
/// `valueIn` admission test measured over `tab`'s own rows.
fn holdings_scope(
    journal: &Journal,
    tab: HoldingsTab,
    accounts: Option<&str>,
    mode: Option<&str>,
    as_of: Option<String>,
    gain_since: Option<String>,
    value_in: Option<&str>,
) -> Result<HoldingsScope, AppError> {
    let scope = HoldingsScope {
        accounts: parse_accounts(accounts),
        mode: parse_mode(mode)?,
        as_of: parse_date("asOf", as_of, today_utc)?,
        gain_since: parse_opt_date("gainSince", gain_since)?,
        value_in: None,
    };
    Ok(HoldingsScope {
        value_in: resolve_value_in(journal, &scope, tab, value_in)?,
        ..scope
    })
}

/// Split a comma-separated `accounts` param into a set of subtree roots, trimming
/// whitespace and dropping empties. `None`/empty ⇒ the empty set = all accounts.
fn parse_accounts(raw: Option<&str>) -> BTreeSet<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// The default cost-of-living exclusion list when the request omits `exclude`:
/// tax accounts. (Mortgage principal and investment/savings transfers are not
/// expense postings, so they never enter the expense total in the first place.)
/// Overridable per request; a config file will supersede this later.
const DEFAULT_COST_EXCLUDE: &[&str] = &["expenses:tax", "expenses:taxes"];

/// Split a comma-separated exclusion param, falling back to `defaults` when the
/// param is absent. An explicit empty value (`exclude=`) means "exclude
/// nothing" — distinct from omitting it, which keeps the defaults.
fn parse_csv(raw: Option<&str>, defaults: &[&str]) -> Vec<String> {
    match raw {
        None => defaults.iter().map(|item| (*item).to_string()).collect(),
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

/// The default comparison-span start when the request omits `start`: the first
/// day of the month 24 months before `end` (a trailing two-year "Year-over-
/// year" window). The SPA normally sends explicit month-aligned dates.
///
/// This was a third copy of the month-index arithmetic, with its OWN malformed-
/// date fallback (`1970`/`1`, where `periods::parts` falls back to `0`). The
/// fallback is unreachable — `end` has already been through `parse_date` →
/// `normalize_iso_date`, so a malformed value is a 400 and the default is
/// `today_utc()` — so unforking it costs nothing and removes the divergence.
fn default_insights_start(end: &str) -> String {
    format!("{}-01", &periods::add_months(end, -24)[..7])
}

/// The `end`/`interval`/`count`/`depth` window shared by the three bucketed
/// reports (`cashflow`, `networth`, `budget`), resolved and validated once.
///
/// The query STRUCTS stay separate and flat on purpose. The obvious collapse —
/// one `Window` struct `#[serde(flatten)]`ed into each query — does not work
/// here: `axum::extract::Query` deserializes through `serde_urlencoded`, and
/// `flatten` forces every value through serde's internal `Content` buffer,
/// which for urlencoded input is always a string. `?depth=2` then fails with
/// `invalid type: string "2", expected usize`, turning five working endpoints
/// into `400`s. (Verified against `serde_urlencoded 0.7.1`; the golden fixtures
/// pin `depth=`/`count=`, so the suite catches it — but the deduplication has to
/// happen on THIS side of the extractor, not in the derive.)
#[derive(Debug)]
struct Window {
    end: String,
    interval: Interval,
    count: usize,
    depth: usize,
}

impl Window {
    /// Validate the four params together, in the order the handlers used to.
    fn resolve(
        end: Option<String>,
        interval: Option<&str>,
        count: Option<usize>,
        depth: Option<usize>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            end: parse_date("end", end, today_utc)?,
            interval: parse_interval(interval)?,
            count: parse_count(count)?,
            depth: depth.unwrap_or(DEFAULT_DEPTH),
        })
    }
}

/// `?asOf=&depth=` — balance sheet.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BalanceSheetQuery {
    as_of: Option<String>,
    depth: Option<usize>,
}

/// `?asOf=&depth=&value=&valueIn=` — grouped balance sheet.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BalanceSheetGroupedQuery {
    as_of: Option<String>,
    depth: Option<usize>,
    /// `market` (default) | `cost` | `none`.
    value: Option<String>,
    /// Commodity to value into; trimmed and admitted against the sheet's own
    /// rows (a value pricing nothing there is a `400`), defaulting to the
    /// journal's `D` commodity when that prices something, else the price
    /// table's base commodity. See [`resolve_grouped_value_in`].
    value_in: Option<String>,
}

/// `?from=&to=&depth=` — income statement.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IncomeStatementQuery {
    from: Option<String>,
    to: Option<String>,
    depth: Option<usize>,
}

/// `?from=&to=&value=&valueIn=&compare=` — grouped income statement.
///
/// Deliberately NO `depth`: this report has none. Groups are the reading and the
/// accounts inside one are a drill-down, so a clamp could only hide accounts the
/// reader has no remaining control to ask for — the same correction that took
/// the slider off the balance-sheet tab.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IncomeStatementGroupedQuery {
    from: Option<String>,
    to: Option<String>,
    /// `market` (default) | `cost` | `none`.
    value: Option<String>,
    /// Commodity to value into; same trim + admission + `D` default contract
    /// as [`BalanceSheetGroupedQuery`]'s, measured over THIS statement's own
    /// windows. See [`resolve_grouped_value_in`].
    value_in: Option<String>,
    /// `previous` (default) | `none`.
    compare: Option<String>,
}

/// `?end=&interval=&count=&depth=` — cash flow.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CashFlowQuery {
    end: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    depth: Option<usize>,
}

/// `?end=&interval=&count=&depth=&valueIn=` — net worth.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetWorthQuery {
    end: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    depth: Option<usize>,
    value_in: Option<String>,
}

/// `?end=&interval=&count=&depth=&budgetDesc=` — budget report.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BudgetQuery {
    end: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    depth: Option<usize>,
    budget_desc: Option<String>,
}

/// `?start=&end=&exclude=` — insights dashboard.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InsightsQuery {
    start: Option<String>,
    end: Option<String>,
    /// Comma-separated account-name prefixes excluded from cost of living.
    exclude: Option<String>,
    /// Minimum base-commodity magnitude for a "biggest change" row (e.g. `10`).
    change_min: Option<String>,
}

/// `?asOf=&lookback=&minMonthly=&minAnnual=` — subscription detection.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionsQuery {
    as_of: Option<String>,
    /// Months of history to scan (default 24).
    lookback: Option<i64>,
    /// Charges needed before a monthly cadence is believed (default 5).
    min_monthly: Option<usize>,
    /// Charges needed before an annual cadence is believed (default 2).
    min_annual: Option<usize>,
    /// Months a charge may go unseen — measured against its funding account's
    /// own latest activity — before it counts as cancelled (default 3).
    stale_months: Option<i64>,
    /// Comma-separated case-insensitive description substrings to exclude
    /// (default [`DEFAULT_EXCLUDE_DESC`]). An explicit empty value excludes
    /// nothing.
    exclude_desc: Option<String>,
}

/// `?asOf=&accounts=&mode=&gainSince=&valueIn=` — holdings snapshot.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HoldingsQuery {
    as_of: Option<String>,
    accounts: Option<String>,
    mode: Option<String>,
    /// Commodity to value the portfolio in (e.g. `$`, `EUR`), overriding both
    /// the journal's `D` directive and the engine's own choice. See
    /// [`resolve_value_in`].
    value_in: Option<String>,
    /// Gain-measurement window start (`YYYY-MM-DD`). Absent/empty = all-time
    /// average-cost gain (unchanged). When set, `gain`/`gainPct` (and totals +
    /// gainers/losers) become `marketValue(asOf) − valueAtStart`; `basis` stays
    /// all-time. See [`holdings`] for the full contract.
    gain_since: Option<String>,
}

/// `?asOf=&accounts=&mode=&interval=&count=&valueIn=` — holdings trend.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HoldingsSeriesQuery {
    as_of: Option<String>,
    accounts: Option<String>,
    mode: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    /// Commodity to value the trend in. Same contract as [`HoldingsQuery`]'s,
    /// and validated against the same scope, so the chart and the table beside
    /// it can never end up in different commodities.
    value_in: Option<String>,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// How many report computations may run at once.
///
/// The report engine is synchronous and CPU-bound, so this is really "how many
/// cores may a pile of open tabs claim". Without a ceiling, N tabs each polling
/// `/api/holdings/series` spawn N blocking threads, and at 200k transactions
/// each one wants a core and hundreds of megabytes. The tokio *worker* threads
/// are protected by `spawn_blocking` alone; this is what keeps the machine
/// responsive as well.
fn report_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        tokio::sync::Semaphore::new(cores)
    })
}

/// Run one report on the blocking pool and wrap the result as JSON (PERF-3).
///
/// Every handler below calls straight into the synchronous engine, which at 200k
/// transactions is 0.5–1.6 seconds of solid CPU. Run directly on a tokio worker
/// that blocks the whole runtime — 20 concurrent `/api/holdings/series` made a
/// trivial `/version` take 1,051 ms instead of 0.3 ms, and because the desktop
/// GUI hosts this runtime in-process, it stalls the app rather than merely an
/// API. `spawn_blocking` moves the work to the blocking pool, where blocking is
/// what the threads are for.
///
/// A panic inside `job` arrives here as a `JoinError` rather than unwinding
/// through `CatchPanicLayer`, so it is mapped to the same `500` that layer would
/// have produced (SEC-2: a panic must never drop the connection).
///
/// [`rules_api`](crate::rules_api) shares it for a related but distinct reason:
/// its work is a DIRECTORY WALK plus file reads, not CPU, and a cold or
/// network-mounted journal directory is exactly the blocking I/O
/// `spawn_blocking` exists for. The semaphore bounds concurrent walks for the
/// same reason it bounds concurrent reports — a pile of open tabs must not be
/// able to claim every blocking thread.
pub(crate) async fn compute<T, F>(job: F) -> Result<Json<T>, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    let Ok(_permit) = report_slots().acquire().await else {
        return Err(AppError::Unavailable(
            "the report scheduler is shutting down".to_string(),
        ));
    };
    match tokio::task::spawn_blocking(job).await {
        Ok(result) => result.map(Json),
        Err(error) => Err(AppError::Internal(format!(
            "the report task failed: {error}"
        ))),
    }
}

/// `GET /api/reports/balancesheet` — assets + liabilities as of a date.
pub(crate) async fn balancesheet(
    State(state): State<AppState>,
    Query(query): Query<BalanceSheetQuery>,
) -> Result<Json<WireSectionedReport>, AppError> {
    let snapshot = state.snapshot();
    let as_of = parse_date("asOf", query.as_of, today_utc)?;
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    compute(move || {
        let declared = declared_types(&account_decls(&snapshot.journal));
        let report = balance_sheet(&snapshot.journal.transactions, &as_of, depth, &declared)?;
        Ok(WireSectionedReport::from(&report))
    })
    .await
}

/// `GET /api/reports/balancesheet/grouped` — the three-box balance sheet:
/// assets, liabilities and equity, each split into collapsible groups, with a
/// `check` line that is `{}` exactly when the journal balances.
///
/// - `asOf=YYYY-MM-DD` (default: today)
/// - `depth=N` (range `0..=100`; ABSENT means no clamp at all) — clamps the
///   expandable ROWS only; every total is summed over members, so none of them
///   move with it. The SPA omits it: the balance sheet reads as groups, the
///   accounts inside one are a drill-down, and a drill-down that stops part-way
///   down the tree just hides accounts with no way to ask for them.
/// - `value=market|cost|none` (default: `market`)
/// - `valueIn=$` (default: the journal's `D` commodity when it prices
///   something on the sheet, else the price table's own base commodity). A
///   `valueIn` pricing NOTHING the sheet displays is a `400`; see
///   [`resolve_grouped_value_in`].
///
/// Prices come from the journal's `P` directives alone, matching `hledger bs
/// -V`. Commodities no price reaches stay on the row as share counts and are
/// listed in `meta.unpriced`, rather than being valued from a stale cost
/// annotation — `/api/reports/networth` makes the opposite choice on purpose,
/// because it is modelling `--infer-market-prices`.
///
/// `/api/reports/balancesheet` is untouched and still serves the flat
/// hledger-parity shape.
pub(crate) async fn balancesheet_grouped(
    State(state): State<AppState>,
    Query(query): Query<BalanceSheetGroupedQuery>,
) -> Result<Json<WireBalanceSheetReport>, AppError> {
    let snapshot = state.snapshot();
    let as_of = parse_date("asOf", query.as_of, today_utc)?;
    let depth = parse_depth(query.depth)?;
    let (value, label) = parse_valuation(query.value.as_deref())?;
    let raw_value_in = query.value_in;

    compute(move || {
        let journal = &snapshot.journal;
        let declared = declared_types(&account_decls(journal));
        // Like `holdings_scope`, admitting `valueIn` scans the whole journal
        // for a price route, so it belongs on the blocking side with the
        // report.
        let value_in = resolve_grouped_value_in(
            journal,
            "this balance sheet",
            raw_value_in.as_deref(),
            |target| {
                prices_any_on_sheet(
                    &journal.transactions,
                    &journal.prices,
                    &as_of,
                    &declared,
                    target,
                )
            },
        )?;
        let report = balance_sheet_grouped(
            &journal.transactions,
            &journal.prices,
            &BsOpts {
                as_of: &as_of,
                depth,
                value,
                value_in,
            },
            &declared,
            &account_groups(journal),
            &bs_terms(journal)?,
        )?;
        Ok(WireBalanceSheetReport::new(&report, label))
    })
    .await
}

/// `GET /api/reports/incomestatement` — revenues + expenses over a range.
pub(crate) async fn incomestatement(
    State(state): State<AppState>,
    Query(query): Query<IncomeStatementQuery>,
) -> Result<Json<WireSectionedReport>, AppError> {
    let snapshot = state.snapshot();
    let today = today_utc();
    // `bucket_start(bucket_key(today, Yearly))`, not `&today[..4]`: a raw byte
    // slice of a date string is the shape DRY-2 exists to stop, and it would
    // panic rather than degrade on a short input.
    let from = parse_date("from", query.from, || {
        format!("{}-01-01", periods::bucket_key(&today, Interval::Yearly))
    })?;
    let to = parse_date("to", query.to, || today)?;
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    compute(move || {
        let declared = declared_types(&account_decls(&snapshot.journal));
        let report =
            income_statement(&snapshot.journal.transactions, &from, &to, depth, &declared)?;
        Ok(WireSectionedReport::from(&report))
    })
    .await
}

/// `GET /api/reports/incomestatement/grouped` — the adaptive-GAAP income
/// statement: revenue and expense accounts collapsed into named lines, valued
/// into one commodity, with a subtotal ladder that materialises rung by rung as
/// the journal's `issection:` tags ask for it.
///
/// - `from=YYYY-MM-DD` (default: Jan 1 of the current year)
/// - `to=YYYY-MM-DD` (default: today) — both INCLUSIVE
/// - `value=market|cost|none` (default: `market`)
/// - `valueIn=$` (default: the journal's `D` commodity when it prices
///   something on the statement, else the price table's own base commodity).
///   A `valueIn` pricing NOTHING in either window is a `400`; see
///   [`resolve_grouped_value_in`].
/// - `compare=previous|none` (default: `previous`) — the immediately preceding
///   window of EQUAL length, each period valued at its OWN end so the prior
///   column agrees with the `hledger is -V` that would have been run over it.
///
/// An untagged journal gets two boxes and a Net income figure, with no ladder at
/// all; `sections` carries only the boxes that have members, so there are never
/// empty headings. `prior` keys are absent rather than null when `compare=none`.
///
/// A `400` here can come from the JOURNAL as well as the query: an `account`
/// directive declaring an `issection:` outside the closed vocabulary is refused
/// by name rather than silently misfiled (see [`AppError`]'s `From<ReportError>`).
///
/// `/api/reports/incomestatement` is untouched and still serves the flat
/// hledger-parity shape.
pub(crate) async fn incomestatement_grouped(
    State(state): State<AppState>,
    Query(query): Query<IncomeStatementGroupedQuery>,
) -> Result<Json<WireIncomeStatementReport>, AppError> {
    let snapshot = state.snapshot();
    let today = today_utc();
    let from = parse_date("from", query.from, || {
        format!("{}-01-01", periods::bucket_key(&today, Interval::Yearly))
    })?;
    let to = parse_date("to", query.to, || today)?;
    let (value, label) = parse_valuation(query.value.as_deref())?;
    let compare = parse_compare(query.compare.as_deref())?;
    let raw_value_in = query.value_in;

    compute(move || {
        let journal = &snapshot.journal;
        let declared = declared_types(&account_decls(journal));
        let sections = account_sections(journal)?;
        let window = DateRange {
            from: from.clone(),
            to: to.clone(),
        };
        let value_in = resolve_grouped_value_in(
            journal,
            "this income statement",
            raw_value_in.as_deref(),
            |target| {
                prices_any_on_statement(
                    &journal.transactions,
                    &journal.prices,
                    &window,
                    compare,
                    &declared,
                    &sections,
                    target,
                )
            },
        )?;
        let report = income_statement_grouped(
            &journal.transactions,
            &journal.prices,
            &IsOpts {
                from: &from,
                to: &to,
                value,
                value_in,
                compare,
            },
            &declared,
            &sections,
            &declared_groups(journal, IS_GROUP_TAG),
        )?;
        Ok(WireIncomeStatementReport::new(&report, label))
    })
    .await
}

/// `GET /api/reports/cashflow` — per-bucket cash-like-asset changes. The cash
/// predicate honors the journal's declared account types (same as the SPA).
pub(crate) async fn cashflow(
    State(state): State<AppState>,
    Query(query): Query<CashFlowQuery>,
) -> Result<Json<WirePeriodReport>, AppError> {
    let snapshot = state.snapshot();
    let window = Window::resolve(
        query.end,
        query.interval.as_deref(),
        query.count,
        query.depth,
    )?;

    compute(move || {
        let decls = account_decls(&snapshot.journal);
        let predicate = cash_predicate(&decls);
        let is_cash: &dyn Fn(&str) -> bool = &predicate;
        let report = cash_flow(
            &snapshot.journal.transactions,
            &window.end,
            window.interval,
            window.count,
            window.depth,
            Some(is_cash),
        )?;
        Ok(WirePeriodReport::from(&report))
    })
    .await
}

/// `GET /api/reports/networth` — market-valued net worth per bucket. Prices come
/// from the journal's explicit `P` directives PLUS prices inferred from `@`/`@@`
/// cost annotations (hledger `--infer-market-prices`); `depth` clamps the account
/// rows; `valueIn` overrides the target commodity.
pub(crate) async fn networth(
    State(state): State<AppState>,
    Query(query): Query<NetWorthQuery>,
) -> Result<Json<WirePeriodReport>, AppError> {
    let snapshot = state.snapshot();
    let window = Window::resolve(
        query.end,
        query.interval.as_deref(),
        query.count,
        query.depth,
    )?;
    let value_in = query
        .value_in
        .filter(|symbol| !symbol.is_empty())
        .map(Commodity);

    compute(move || {
        let declared = declared_types(&account_decls(&snapshot.journal));
        let report = net_worth(
            &snapshot.journal.transactions,
            &snapshot.journal.prices,
            &NetWorthOpts {
                end: &window.end,
                interval: window.interval,
                count: window.count,
                depth: window.depth,
                value_in,
                declared: &declared,
            },
        )?;
        Ok(WirePeriodReport::from(&report))
    })
    .await
}

/// `GET /api/budget` — actuals vs. periodic-rule goals per bucket.
pub(crate) async fn budget(
    State(state): State<AppState>,
    Query(query): Query<BudgetQuery>,
) -> Result<Json<WireBudgetReport>, AppError> {
    let snapshot = state.snapshot();
    let window = Window::resolve(
        query.end,
        query.interval.as_deref(),
        query.count,
        query.depth,
    )?;
    let budget_desc = query.budget_desc.filter(|pattern| !pattern.is_empty());

    compute(move || {
        let opts = BudgetOpts {
            end: &window.end,
            interval: window.interval,
            count: window.count,
            depth: window.depth,
            budget_desc: budget_desc.as_deref(),
        };
        let report = budget_report(
            &snapshot.journal.transactions,
            &snapshot.journal.periodic_transactions,
            &opts,
        )?;
        Ok(WireBudgetReport::from(&report))
    })
    .await
}

/// `GET /api/insights` — the period-over-period dashboard. `start`/`end` bound
/// the whole comparison span (default: a trailing 24 months ending today); the
/// engine splits it at its midpoint into a previous and current period.
/// `exclude` overrides the cost-of-living exclusion list.
pub(crate) async fn insights_report(
    State(state): State<AppState>,
    Query(query): Query<InsightsQuery>,
) -> Result<Json<WireInsightsReport>, AppError> {
    let snapshot = state.snapshot();
    let end = parse_date("end", query.end, today_utc)?;
    let start = parse_date("start", query.start, || default_insights_start(&end))?;
    let cost_exclude = parse_csv(query.exclude.as_deref(), DEFAULT_COST_EXCLUDE);
    let change_min = parse_change_min(query.change_min.as_deref())?;
    compute(move || {
        let opts = InsightsOpts {
            start: &start,
            end: &end,
            cost_exclude: &cost_exclude,
            change_min,
        };
        let report = insights(&snapshot.journal, &opts)?;
        Ok(WireInsightsReport::from(&report))
    })
    .await
}

/// `GET /api/subscriptions` — recurring monthly/annual charges inferred from
/// the journal's expense history. Independent of the insights comparison period:
/// it always scans the trailing `lookback` months ending at `asOf`.
pub(crate) async fn subscriptions(
    State(state): State<AppState>,
    Query(query): Query<SubscriptionsQuery>,
) -> Result<Json<WireSubscriptionsReport>, AppError> {
    let snapshot = state.snapshot();
    let as_of = parse_date("asOf", query.as_of, today_utc)?;
    let defaults = SubscriptionOpts::default();
    let exclude_desc = parse_csv(query.exclude_desc.as_deref(), DEFAULT_EXCLUDE_DESC);
    let lookback_months = query.lookback.unwrap_or(defaults.lookback_months).max(1);
    let min_monthly = query.min_monthly.unwrap_or(defaults.min_monthly).max(2);
    let min_annual = query.min_annual.unwrap_or(defaults.min_annual).max(2);
    let stale_months = query.stale_months.unwrap_or(defaults.stale_months).max(1);
    compute(move || {
        let opts = SubscriptionOpts {
            as_of: &as_of,
            lookback_months,
            min_monthly,
            min_annual,
            stale_months,
            exclude_desc: &exclude_desc,
            ..SubscriptionOpts::default()
        };
        let report = detect_subscriptions(&snapshot.journal, &opts)?;
        Ok(WireSubscriptionsReport::from(&report))
    })
    .await
}

/// `GET /api/holdings` — average-cost stock positions as of a date. `accounts`
/// is a comma-separated set of subtree roots; `mode` selects include vs. exclude.
/// Prices come from the journal's `P` directives (and cost-annotation fallbacks).
///
/// `gainSince=YYYY-MM-DD` (optional) changes the gain start. Absent or empty →
/// all-time average-cost gain, byte-identical to before. When set, each row's
/// `gain` = `marketValue − valueAtStart` and `gainPct` = `gain / valueAtStart ×
/// 100`, where `valueAtStart` is the position's market value at `gainSince`
/// (shares held then, priced as of then; `0` when not held, `null`-propagating
/// when held-but-unpriced then). `basis` stays the all-time average-cost basis;
/// `totals.gain`/`totals.gainPct` are windowed while `totals.basis` stays all-
/// time; `topGainers`/`topLosers` rank by the windowed `gainPct`. The JSON keys
/// are unchanged — only the meaning of `gain`/`gainPct` shifts.
///
/// `valueIn=COMMODITY` (optional) fixes the commodity everything is reported in
/// — the `base` field, every `marketValue`, every `basis`. Absent, the journal's
/// `D` default-commodity directive is used, and failing that the engine picks
/// the price target that actually prices the portfolio. A commodity that prices
/// nothing in scope is a `400`; see [`resolve_value_in`].
pub(crate) async fn holdings(
    State(state): State<AppState>,
    Query(query): Query<HoldingsQuery>,
) -> Result<Json<WireHoldingsReport>, AppError> {
    let snapshot = state.snapshot();
    compute(move || {
        // `holdings_scope` validates `valueIn` by scanning the whole journal for
        // a price route, so it belongs on the blocking side with the report.
        let scope = holdings_scope(
            &snapshot.journal,
            HoldingsTab::Stocks,
            query.accounts.as_deref(),
            query.mode.as_deref(),
            query.as_of,
            query.gain_since,
            query.value_in.as_deref(),
        )?;
        let report = compute_holdings(
            &snapshot.journal.transactions,
            &snapshot.journal.prices,
            &snapshot.journal.accounts,
            &snapshot.journal.commodity_tags,
            &scope,
        )?;
        Ok(WireHoldingsReport::from(&report))
    })
    .await
}

/// `GET /api/holdings/series` — portfolio market value (and basis) at each of the
/// last `count` period boundaries ending at `asOf`. Same scope — and same
/// `valueIn` contract — as `/api/holdings`.
pub(crate) async fn holdings_series_report(
    State(state): State<AppState>,
    Query(query): Query<HoldingsSeriesQuery>,
) -> Result<Json<WireHoldingsSeries>, AppError> {
    let snapshot = state.snapshot();
    let interval = parse_interval(query.interval.as_deref())?;
    let count = parse_count(query.count)?;
    compute(move || {
        let scope = holdings_scope(
            &snapshot.journal,
            HoldingsTab::Stocks,
            query.accounts.as_deref(),
            query.mode.as_deref(),
            query.as_of,
            // The trend tracks market value/basis only — no per-point gain window.
            None,
            query.value_in.as_deref(),
        )?;
        let series = holdings_series(
            &snapshot.journal.transactions,
            &snapshot.journal.prices,
            &snapshot.journal.accounts,
            &snapshot.journal.commodity_tags,
            &scope,
            interval,
            count,
        )?;
        Ok(WireHoldingsSeries::from(&series))
    })
    .await
}

/// `GET /api/holdings/other` — the assets you own that are neither securities
/// nor cash: a house, a car, a partnership interest.
///
/// Same scope, same `asOf`/`gainSince`/`valueIn` contract as `/api/holdings`, so
/// the page's one scope bar drives both tabs — but `valueIn` is admitted against
/// THIS tab's rows, not the stock portfolio's (see [`HoldingsTab`]). What differs
/// is the KEY: rows here are accounts, not commodities — see `holdings::other`
/// for why that cannot be a filter over the stock engine.
pub(crate) async fn other_holdings_report(
    State(state): State<AppState>,
    Query(query): Query<HoldingsQuery>,
) -> Result<Json<WireOtherHoldingsReport>, AppError> {
    let snapshot = state.snapshot();
    compute(move || {
        let scope = holdings_scope(
            &snapshot.journal,
            HoldingsTab::Other,
            query.accounts.as_deref(),
            query.mode.as_deref(),
            query.as_of,
            query.gain_since,
            query.value_in.as_deref(),
        )?;
        let report = other_holdings(
            &snapshot.journal.transactions,
            &snapshot.journal.prices,
            &snapshot.journal.accounts,
            &scope,
        )?;
        Ok(WireOtherHoldingsReport::from(&report))
    })
    .await
}

/// `GET /api/holdings/other/series` — total Other-holdings value (and cost) at
/// each of the last `count` period boundaries ending at `asOf`.
///
/// Returns a `WireHoldingsSeries`, the same shape `/api/holdings/series` does,
/// so one decoder and one chart component serve both tabs.
pub(crate) async fn other_holdings_series_report(
    State(state): State<AppState>,
    Query(query): Query<HoldingsSeriesQuery>,
) -> Result<Json<WireHoldingsSeries>, AppError> {
    let snapshot = state.snapshot();
    let interval = parse_interval(query.interval.as_deref())?;
    let count = parse_count(query.count)?;
    compute(move || {
        let scope = holdings_scope(
            &snapshot.journal,
            HoldingsTab::Other,
            query.accounts.as_deref(),
            query.mode.as_deref(),
            query.as_of,
            // The trend tracks value/cost only — no per-point change window.
            None,
            query.value_in.as_deref(),
        )?;
        let series = other_holdings_series(
            &snapshot.journal.transactions,
            &snapshot.journal.prices,
            &snapshot.journal.accounts,
            &scope,
            interval,
            count,
        )?;
        Ok(WireHoldingsSeries::from(&series))
    })
    .await
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    /// The forms `hledger 1.52` itself accepts for `-b`/`-e`, all normalizing to
    /// the same fixed-width ISO date the report engine's lexical ordering needs.
    #[test]
    fn accepts_and_normalizes_every_hledger_date_spelling() {
        for raw in [
            "2026-07-01",
            "2026-7-1",
            "2026/7/1",
            "2026/07/01",
            "2026.7.1",
            "2026-07-1",
            "2026-7-01",
        ] {
            assert_eq!(
                normalize_iso_date(raw).as_deref(),
                Ok("2026-07-01"),
                "{raw} should normalize to 2026-07-01"
            );
        }
    }

    /// Month/day boundaries and the leap-year rule, including the century cases.
    #[test]
    fn accepts_real_calendar_boundaries() {
        for raw in [
            "2024-02-29", // leap year
            "2000-02-29", // 400-divisible century IS a leap year
            "2026-01-31",
            "2026-12-31",
            "0001-01-01",
            "9999-12-31",
        ] {
            assert!(
                normalize_iso_date(raw).is_ok(),
                "{raw} is a real date and must be accepted"
            );
        }
    }

    /// RPT-4: every one of these used to flow straight into a `&str` comparison.
    #[test]
    fn rejects_malformed_dates() {
        for raw in [
            "",             // sorted below every real date → an empty report
            "garbage",      // sorted above every real date → the all-time total
            "2026",         // too few components
            "2026-07",      // too few components
            "2026-07-01-1", // too many components
            "07-01",        // yearless: needs a `Y` directive, meaningless in a URL
            "26-07-01",     // two-digit year (hledger rejects it too)
            "12026-01-01",  // five-digit year would break the fixed-width slices
            "2026-007-01",  // over-wide month
            "2026-07-001",  // over-wide day
            "2026--7-1",    // empty component
            "2026-x7-01",   // non-digit
            "2026-+7-01",   // signed component
            " 2026-07-01",  // untrimmed
            "2026-07-01 ",  // untrimmed
            "-2026-07-01",  // leading separator (a negative year)
        ] {
            assert!(
                normalize_iso_date(raw).is_err(),
                "{raw:?} is not a date and must be rejected"
            );
        }
    }

    /// Shaped like a date, but not a date. hledger rejects all of these.
    #[test]
    fn rejects_impossible_calendar_dates() {
        for raw in [
            "2026-02-30",
            "2026-02-29", // 2026 is not a leap year
            "2023-02-29",
            "1900-02-29", // century, not 400-divisible → not a leap year
            "2026-04-31",
            "2026-13-01",
            "2026-00-01",
            "2026-01-00",
            "2026-01-32",
        ] {
            assert!(
                normalize_iso_date(raw).is_err(),
                "{raw} is not a real date and must be rejected"
            );
        }
    }

    /// The `400` body names the parameter, echoes the value, and says why —
    /// enough to fix the request without reading the source.
    #[test]
    fn rejection_message_names_the_field_and_the_reason() {
        let error = checked_date("asOf", "2026-02-30").expect_err("not a date");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        let message = error.to_string();
        assert!(message.contains("asOf"), "{message}");
        assert!(message.contains("2026-02-30"), "{message}");
        assert!(message.contains("no day 30"), "{message}");

        let message = checked_date("end", "nope")
            .expect_err("not a date")
            .to_string();
        assert!(message.contains("YYYY-MM-DD"), "{message}");
    }

    /// The window the three bucketed reports share validates its four params
    /// exactly as the handlers used to, and rejects each of them.
    #[test]
    fn window_resolves_defaults_and_rejects_each_bad_param() {
        let window = Window::resolve(Some("2026-6-30".to_string()), None, None, None)
            .expect("a valid window");
        assert_eq!(window.end, "2026-06-30");
        assert_eq!(window.interval, Interval::Monthly);
        assert_eq!(window.count, DEFAULT_COUNT);
        assert_eq!(window.depth, DEFAULT_DEPTH);

        for (end, interval, count) in [
            (Some("2026-02-30".to_string()), None, None),
            (None, Some("hourly"), None),
            (None, None, Some(0)),
            (None, None, Some(MAX_BUCKETS + 1)),
        ] {
            let error = Window::resolve(end, interval, count, None).expect_err("must be rejected");
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{error}");
        }
    }

    /// An absent param keeps its default; an explicit malformed one is a `400`.
    #[test]
    fn parse_date_defaults_only_when_absent() {
        assert_eq!(
            parse_date("asOf", None, || "2026-06-30".to_string()),
            Ok("2026-06-30".to_string())
        );
        assert_eq!(
            parse_date("asOf", Some("2026-6-30".to_string()), String::new),
            Ok("2026-06-30".to_string())
        );
        // An explicit empty value is NOT "absent": it used to sort below every
        // real date and serve an empty report with a 200.
        assert!(parse_date("asOf", Some(String::new()), String::new).is_err());
    }

    /// `gainSince` keeps its documented empty-means-all-time sentinel, but a
    /// non-empty value still has to be a real date.
    #[test]
    fn parse_opt_date_keeps_the_empty_sentinel() {
        assert_eq!(parse_opt_date("gainSince", None), Ok(None));
        assert_eq!(parse_opt_date("gainSince", Some(String::new())), Ok(None));
        assert_eq!(
            parse_opt_date("gainSince", Some("2026-1-2".to_string())),
            Ok(Some("2026-01-02".to_string()))
        );
        assert!(parse_opt_date("gainSince", Some("2026-02-30".to_string())).is_err());
    }

    /// `changeMin` keeps hledger's digit-group separators but no longer swallows
    /// a value it cannot read.
    #[test]
    fn parse_change_min_rejects_instead_of_defaulting() {
        assert_eq!(parse_change_min(None), Ok(DEFAULT_CHANGE_MIN));
        assert_eq!(parse_change_min(Some("")), Ok(DEFAULT_CHANGE_MIN));
        assert_eq!(parse_change_min(Some("25.50")), Ok(Dec::new(2550, 2)));
        // `,` is a digit-group separator when the decimal mark is `.`, so this
        // is a well-formed 1000 — NOT the $10.00 the finding predicted.
        assert_eq!(parse_change_min(Some("1,000")), Ok(Dec::new(1000, 0)));
        for raw in ["zzz", "$10", "10%", "1.0.0.x"] {
            assert!(
                parse_change_min(Some(raw)).is_err(),
                "{raw} must be a 400, not a silent $10.00"
            );
        }
    }
}
