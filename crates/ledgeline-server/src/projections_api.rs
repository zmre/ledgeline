//! The projections wire: run a what-if, and seed one from the journal.
//!
//! Two routes, and they differ in a way worth stating at the top:
//!
//! - `POST /api/projections/run` takes the WHOLE scenario in its body. That is
//!   the point (decision 3 of `plans/22-projections.md`): live editing has to
//!   project what is on screen, saved or not, and the exact `Dec` money math
//!   lives in the engine rather than being duplicated into TypeScript. The round
//!   trip is to localhost.
//! - `GET /api/projections/seed` reads the journal and answers with a starting
//!   scenario — its `~` rules plus one monthly line per unbudgeted category.
//!
//! # A `POST` that writes nothing
//!
//! This is the first native REPORT with a request body, and a body is the one
//! thing a query string cannot be trusted to bound for us. So: the route carries
//! its own [`MAX_BODY_BYTES`] limit, and the scenario's line, event and posting
//! counts are checked before anything reaches a `compute` slot. Every refusal is
//! a `400` naming the limit, because a caller who sent 10,000 lines needs to
//! know which number to look at.
//!
//! It still lives above the `route_layer` in `lib.rs`, with every other `/api`
//! route. It writes nothing, but it reads the user's journal, and a report of
//! their finances is not less private for being read-only.
//!
//! # Nulls, not omissions
//!
//! Unlike its neighbours, every optional field here serializes as `null` rather
//! than being skipped. A scenario is a round-trip shape — the client is handed
//! one and hands it back — and "the key is absent" versus "the key is null" is
//! exactly the distinction a hand-written decoder gets wrong. Present-and-null
//! is one rule instead of two.

use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use ledgeline_core::decimal::MAX_PARSE_PLACES;
use ledgeline_core::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, PeriodSpec, Posting,
};
use ledgeline_core::parse::parse_period_spec;
use ledgeline_core::projections::serialize::{
    ProjectionDoc, SerializeError, new_file, scenario_from_text, write_scenario,
};
use ledgeline_core::projections::{
    AssetRow, BalanceSeries, CounterDefaults, CreateRefusal, DEFAULT_LOOKBACK,
    DiscoveredProjection, Discovery, ForecastDefaults, Growth, GrowthUnit, History, LineRole,
    LineSection, LineSource, Projection, ProjectionOpts, ProjectionPath, Runway, Scenario,
    ScenarioEvent, ScenarioLine, SeedOpts, discover_in, forecast_cutoff, is_journal_name,
    label_for, latest_real_date, line_section, merged_prices, merged_transactions, project,
    seed_scenario, upgrade_legacy_counters, virtual_posting,
};
use ledgeline_core::reports::account_types::{AccountType, AccountTypes};
use ledgeline_core::reports::periods::MAX_BUCKETS;
use ledgeline_core::reports::{
    BsOpts, BudgetOpts, CfOpts, CfTags, Interval, IsOpts, Valuation, account_decls,
    balance_sheet_columns, cash_flow_statement, declared_types, income_statement_columns,
    periods::bucket_label, pivot_balance_sheets, range_buckets, signed_entry,
};
use ledgeline_core::vault::{Revision, VaultError};
use ledgeline_core::{Dec, Fingerprint};
use serde::{Deserialize, Serialize};

use crate::edit_api::{WireDecIn, dec_from_wire, json_body};
use crate::error::{AppError, editing_disabled};
use crate::reports_api::{
    StatementShape, Window, WireBalanceSheetColumns, WireCashFlow, WireDec,
    WireIncomeStatementReport, WireMixed, checked_date, compute, today_utc, wire_mixed,
};
use crate::security::no_store_json as no_store;
use crate::{AppState, JournalLocation, Snapshot};

// ===========================================================================
// Limits
// ===========================================================================

/// The largest `POST /api/projections/run` or `PUT /api/projections/{*id}` body
/// this server will read.
///
/// 256 KiB is roughly a thousand lines with generous account names and notes —
/// far past any scenario a human edits, and far short of what it costs to hold
/// while a `compute` slot is claimed. Enforced by axum's `DefaultBodyLimit` on
/// those two routes alone, the way `/api/import/stage` sets its own rather than
/// moving the global one; an over-limit body arrives as a `JsonRejection` and
/// [`json_body`] renders it as a `400`.
pub(crate) const MAX_BODY_BYTES: usize = 256 * 1024;

/// The most recurring lines one scenario may carry.
const MAX_LINES: usize = 1000;
/// The most dated events one scenario may carry.
const MAX_EVENTS: usize = 1000;
/// The most postings one event may carry.
///
/// The line and event counts alone do not bound the work: one event with a
/// hundred thousand postings is a single event. This is the third number the
/// body limit would otherwise be the only guard for.
const MAX_EVENT_POSTINGS: usize = 100;

// ===========================================================================
// Wire — outbound
// ===========================================================================

/// A single-commodity amount.
///
/// `precision` travels because the projection engine rounds a growing amount
/// back to it at every step: a client that dropped it and sent the amount back
/// would get a different — and wrong — growth curve. It is the amount's DISPLAY
/// precision, which is not the same thing as `quantity.places` (a `$1,000`
/// written without cents has precision 0 and places 0; one written `$1,000.00`
/// has both at 2, and one computed as an average may have places 2 and
/// precision 0).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAmount {
    commodity: String,
    quantity: WireDec,
    precision: u32,
}

impl From<&Amount> for WireAmount {
    fn from(amount: &Amount) -> Self {
        Self {
            commodity: amount.commodity.0.clone(),
            quantity: WireDec::from(amount.quantity),
            precision: amount.style.precision,
        }
    }
}

/// A recurrence, in the three readings a client needs.
///
/// `raw` is the round-trip field and the only one the server reads back;
/// `simple`, `from` and `to` are derived, and are here so the table can show a
/// period without re-implementing hledger's grammar in TypeScript.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireScenarioPeriod {
    /// The period expression exactly as it would be written in a journal.
    raw: String,
    /// `daily`…`yearly` when this is a BARE fixed interval, else `null`.
    simple: Option<&'static str>,
    /// The `from` date, ISO, or `null`.
    from: Option<String>,
    /// The `to` date, ISO and EXCLUSIVE as hledger writes it, or `null`.
    to: Option<String>,
}

impl From<&PeriodSpec> for WireScenarioPeriod {
    fn from(period: &PeriodSpec) -> Self {
        Self {
            raw: period.raw.clone(),
            simple: period.interval().map(ledgeline_core::periodic::period_word),
            from: period.start.clone(),
            to: period.end.clone(),
        }
    }
}

/// A stepwise growth rate. `rate` is a FRACTION (`0.03`), never a percentage —
/// the `%` belongs to the file format and the UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireGrowth {
    rate: WireDec,
    /// `week` | `month` | `year`.
    unit: &'static str,
}

impl From<&Growth> for WireGrowth {
    fn from(growth: &Growth) -> Self {
        Self {
            rate: WireDec::from(growth.rate),
            unit: growth.unit.as_str(),
        }
    }
}

/// One recurring row of the what-if table.
///
/// # `role` is the asset/flow discriminator, and it is EXPLICIT
///
/// A client must never infer it from `account`. `assets:cash` appears on both
/// sides of the plan's own worked example — as the destination of a `$2M` raise
/// (a flow) and as a balance that compounds (an asset row) — so a reader that
/// guessed from the account's type would reclassify one of them on every round
/// trip. The field is always present outbound; absent inbound reads as `flow`,
/// which is what every body written before asset rows existed means.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireScenarioLine {
    /// The LOGICAL row. Two bounded segments of one step change share it.
    id: String,
    /// The SOURCE RULE. Every posting of one `~` block shares it, which is what
    /// the engine's cash and net-worth guards are scoped to.
    group: String,
    /// `flow` | `asset`.
    role: &'static str,
    account: String,
    /// For a flow, the amount that moves each period. For an ASSET row, the
    /// per-period CONTRIBUTION — zero when the balance only compounds.
    amount: WireAmount,
    period: WireScenarioPeriod,
    growth: Option<WireGrowth>,
    /// Asset rows only: an override of the journal's balance at the projection
    /// start. `null` — the normal case — means "use the journal's", and the
    /// table shows the journal's real figure greyed.
    opening: Option<WireAmount>,
    note: String,
    /// The account the line balances against (plan 28), or `null` for a
    /// pre-plan-28 line whose default the engine picks and a save writes.
    counter: Option<String>,
    /// Asset rows only: the gains account, or `null` for the default.
    gains: Option<String>,
    /// `journal` (authored) | `unbudgeted` (estimated from history).
    source: &'static str,
    /// `income` | `expense` | `asset` | `oneoff` — the table section the
    /// line's role, period and account TYPE put it in
    /// ([`ledgeline_core::projections::line_section`]). A client shows a line
    /// here without carrying the rule; it only needs its own answer for an
    /// account the user is still typing.
    section: LineSection,
}

impl WireScenarioLine {
    fn new(line: &ScenarioLine, declared: &BTreeMap<String, AccountType>) -> Self {
        Self {
            section: line_section(line.role, &line.period.raw, &line.account.0, declared),
            id: line.id.clone(),
            group: line.group.clone(),
            role: line.role.as_str(),
            account: line.account.0.clone(),
            amount: WireAmount::from(&line.amount),
            period: WireScenarioPeriod::from(&line.period),
            growth: line.growth.as_ref().map(WireGrowth::from),
            opening: line.opening.as_ref().map(WireAmount::from),
            note: line.note.clone(),
            counter: line.counter.as_ref().map(|account| account.0.clone()),
            gains: line.gains.as_ref().map(|account| account.0.clone()),
            source: line.source.as_str(),
        }
    }
}

/// One account/amount pair of a dated event.
///
/// The model lets a posting hold several amounts (an inferred leg can be
/// mixed); the table's row is one account and one amount, so a multi-amount
/// posting is FLATTENED into one wire posting per amount. Nothing is lost — the
/// engine sums them either way — and the client never has to decide which of
/// two amounts its number box holds.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireEventPosting {
    account: String,
    amount: WireAmount,
}

/// A dated one-off.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireScenarioEvent {
    id: String,
    date: String,
    description: String,
    postings: Vec<WireEventPosting>,
    /// The account the event balances against, or `null`.
    counter: Option<String>,
}

impl From<&ScenarioEvent> for WireScenarioEvent {
    fn from(event: &ScenarioEvent) -> Self {
        Self {
            id: event.id.clone(),
            date: event.date.clone(),
            description: event.description.clone(),
            postings: event
                .postings
                .iter()
                .flat_map(|posting| {
                    posting.amounts.iter().map(|amount| WireEventPosting {
                        account: posting.account.0.clone(),
                        amount: WireAmount::from(amount),
                    })
                })
                .collect(),
            counter: event.counter.as_ref().map(|account| account.0.clone()),
        }
    }
}

/// A whole what-if.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireScenario {
    /// The display name. Empty for a seed: naming it is the Save As dialog's
    /// job, and a placeholder here would become a filename nobody chose.
    name: String,
    created: Option<String>,
    updated: Option<String>,
    lines: Vec<WireScenarioLine>,
    events: Vec<WireScenarioEvent>,
    /// `; cash:` — the scenario's default counter-account, or `null`.
    cash: Option<String>,
}

impl WireScenario {
    fn new(scenario: &Scenario, declared: &BTreeMap<String, AccountType>) -> Self {
        Self {
            name: scenario.name.clone(),
            created: scenario.created.clone(),
            updated: scenario.updated.clone(),
            lines: scenario
                .lines
                .iter()
                .map(|line| WireScenarioLine::new(line, declared))
                .collect(),
            events: scenario
                .events
                .iter()
                .map(WireScenarioEvent::from)
                .collect(),
            cash: scenario.cash.as_ref().map(|account| account.0.clone()),
        }
    }
}

/// An opening balance and each bucket's closing balance.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBalanceSeries {
    /// The real balance the day before the cutoff, before any projected flow.
    opening: WireMixed,
    /// One closing balance per bucket, oldest → newest.
    values: Vec<WireMixed>,
}

impl From<&BalanceSeries> for WireBalanceSeries {
    fn from(series: &BalanceSeries) -> Self {
        Self {
            opening: wire_mixed(&series.opening),
            values: series.values.iter().map(wire_mixed).collect(),
        }
    }
}

/// Where the cash crosses zero.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRunway {
    /// 0-based index into `buckets`.
    bucket: usize,
    /// That bucket's human label (`Mar 2028`).
    label: String,
    /// The bucket's last day — the date the closing balance is negative as of.
    date: String,
    /// Whole periods from the start of the projection (`bucket + 1`).
    periods: usize,
}

/// What one asset row did, attributed back to the row.
///
/// The Balance column's source, and the net-worth tab's breakdown. Both need a
/// figure the scenario does not carry — an opening balance is not part of a
/// what-if — and both must agree with the curve beside them, so the figure comes
/// from the walk that drew the curve rather than from a second report read
/// alongside it (`plans/23-asset-growth.md`, Phase 3).
///
/// Keyed by `group`, the source rule, which is unique per row segment and is
/// already on the row a client is rendering. A row the engine refused to model
/// is ABSENT here and present in `warnings`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAssetRow {
    /// The `group` of the scenario line this was placed from.
    group: String,
    account: String,
    /// The journal's own SUBTREE balance for that account as of the request's
    /// `asOf`, valued the way the opening net worth is. What the table greys out.
    journal_opening: WireMixed,
    /// The balance the row actually compounded from — the `opening` override
    /// when there is one, else `journalOpening`.
    opening: WireMixed,
    /// Total appreciation over the span. Never a contribution, and never the
    /// override's one-off adjustment.
    growth: WireMixed,
}

impl From<&AssetRow> for WireAssetRow {
    fn from(row: &AssetRow) -> Self {
        Self {
            group: row.group.clone(),
            account: row.account.clone(),
            journal_opening: wire_mixed(&row.journal_opening),
            opening: wire_mixed(&row.opening),
            growth: wire_mixed(&row.growth),
        }
    }
}

/// The accounts the expansion books to where the scenario names none — what
/// the what-if table shows greyed until the user overrides it (plan 28,
/// Decisions 4 and 7).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireForecastDefaults {
    /// The journal's own default counter-account: what a scenario with no
    /// `; cash:` header falls back to. The `cash` control's greyed value.
    cash: String,
    /// The gains account of an asset row that names none.
    gains: String,
    /// `group` → the default counter-account of that rule's counter-less
    /// lines. Only rules with at least one such line appear.
    lines: BTreeMap<String, String>,
    /// Event `id` → its default counter-account, for events that name none.
    events: BTreeMap<String, String>,
}

impl From<&ForecastDefaults> for WireForecastDefaults {
    fn from(defaults: &ForecastDefaults) -> Self {
        let names = |map: &BTreeMap<String, AccountName>| {
            map.iter()
                .map(|(key, account)| (key.clone(), account.0.clone()))
                .collect()
        };
        Self {
            cash: defaults.cash.0.clone(),
            gains: defaults.gains.0.clone(),
            lines: names(&defaults.lines),
            events: names(&defaults.events),
        }
    }
}

/// The answer (plan 28, Phase 4).
///
/// Two kinds of column live here, and the difference is load-bearing:
///
/// - `buckets`, `cash` and `runway` cover the FORECAST periods only — the
///   runway counts from the cutoff;
/// - the three statements cover `lookback ++ buckets`: the full actual periods
///   before the cutoff, then the forecast. `projectedFrom` is the index of the
///   first forecast column in them (`lookback.len()`), which is where a client
///   draws the actuals/projected divider.
///
/// Net income and net worth are not series here any more: the income
/// statement's `netIncome` and the balance sheet's `netWorth` are, computed by
/// the same code as on the Reports tab.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireProjection {
    /// Forecast bucket keys, oldest → newest. The first starts at `actualsEnd`.
    buckets: Vec<String>,
    /// The full ACTUAL periods shown before the forecast, oldest → newest.
    lookback: Vec<String>,
    /// The cutoff (Decision 5): the first projected day. Actuals are
    /// everything strictly before it, and no real transaction on or after it
    /// is in any figure here.
    actuals_end: String,
    /// The index of the first forecast column in the statements' columns.
    projected_from: usize,
    cash: WireBalanceSeries,
    runway: Option<WireRunway>,
    /// One entry per asset row the walk modelled, in scenario order. `[]` for a
    /// scenario with no asset rows, and always present.
    assets: Vec<WireAssetRow>,
    /// Everything the projection could not do. Never empty silently: a dropped
    /// line always says so here.
    warnings: Vec<String>,
    /// The balance sheet at the end of every column, market-valued at its own
    /// end with the forecast's in-memory prices.
    balance_sheet: WireBalanceSheetColumns,
    /// The grouped income statement, one column per period plus the Total.
    income_statement: WireIncomeStatementReport,
    /// The statement of cash flows over the same columns.
    cash_flow: WireCashFlow,
    defaults: WireForecastDefaults,
}

/// The three official statements over `real ++ forecast`, built exactly as the
/// Reports tab builds them — the same [`StatementShape`], the same
/// [`CfTags::from_journal`], the same functions — over the merged transactions
/// and prices, across `lookback ++ buckets`.
///
/// Valued (balance sheet, income statement) at market in the projection's own
/// valuation commodity, each column at its own end, so a column past the last
/// real price reads the forecast's in-memory prices (Decision 3) and the
/// balance sheet's valuation adjustment keeps it balanced.
fn projected_statements(
    journal: &ledgeline_core::Journal,
    projection: &Projection,
    shape: &StatementShape,
    depth: usize,
) -> Result<
    (
        WireBalanceSheetColumns,
        WireIncomeStatementReport,
        WireCashFlow,
    ),
    AppError,
> {
    let forecast = &projection.forecast;
    let window = &forecast.window;
    let txns = merged_transactions(&journal.transactions, forecast);
    let prices = merged_prices(&journal.prices, forecast);
    let spans = range_buckets(&window.from, &window.end, Some(window.interval))?;
    let ends: Vec<String> = spans.iter().map(|span| span.to.clone()).collect();
    let value_in = forecast.target.clone();

    let sheets = balance_sheet_columns(
        &txns,
        &prices,
        &BsOpts {
            as_of: &window.end,
            depth: None,
            value: Valuation::Market,
            value_in: value_in.clone(),
        },
        &ends,
        &shape.declared,
        &shape.bs_groups,
        &shape.bs_terms,
    )?;
    let income = income_statement_columns(
        &txns,
        &prices,
        &IsOpts {
            from: &window.from,
            to: &window.end,
            value: Valuation::Market,
            value_in,
            compare: false,
        },
        window.interval,
        &shape.declared,
        &shape.is_sections,
        &shape.is_groups,
    )?;
    let cash_flow = cash_flow_statement(
        &txns,
        &CfOpts {
            from: &window.from,
            to: &window.end,
            interval: Some(window.interval),
            depth,
        },
        &CfTags::from_journal(journal),
    )?;
    Ok((
        WireBalanceSheetColumns::new(&pivot_balance_sheets(&sheets), &spans, "market"),
        WireIncomeStatementReport::columns(&income, "market"),
        WireCashFlow::from(&cash_flow),
    ))
}

impl WireProjection {
    fn new(
        projection: &Projection,
        (balance_sheet, income_statement, cash_flow): (
            WireBalanceSheetColumns,
            WireIncomeStatementReport,
            WireCashFlow,
        ),
    ) -> Self {
        Self {
            buckets: projection.buckets.clone(),
            lookback: projection.lookback.clone(),
            actuals_end: projection.start.clone(),
            projected_from: projection.lookback.len(),
            cash: WireBalanceSeries::from(&projection.cash),
            runway: projection
                .runway
                .as_ref()
                .map(|runway| wire_runway(runway, &projection.buckets)),
            assets: projection.assets.iter().map(WireAssetRow::from).collect(),
            warnings: projection.warnings.clone(),
            balance_sheet,
            income_statement,
            cash_flow,
            defaults: WireForecastDefaults::from(&projection.forecast.defaults),
        }
    }
}

fn wire_runway(runway: &Runway, buckets: &[String]) -> WireRunway {
    let key = buckets.get(runway.bucket).cloned().unwrap_or_default();
    WireRunway {
        bucket: runway.bucket,
        label: bucket_label(&key),
        date: runway.date.clone(),
        periods: runway.periods,
    }
}

// ===========================================================================
// Wire — inbound
// ===========================================================================

/// A single-commodity amount on the way in.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AmountIn {
    commodity: String,
    quantity: WireDecIn,
    /// Display precision. Absent means "as many places as the quantity has",
    /// which is what an amount typed straight into the table means.
    #[serde(default)]
    precision: Option<u32>,
}

/// A recurrence on the way in: `{"raw": "monthly from 2027-04-01"}`.
///
/// **The one type here without `deny_unknown_fields`, deliberately.**
/// [`WireScenarioPeriod`] goes out with three DERIVED fields beside `raw`, and a
/// client should be able to hand the object straight back rather than stripping
/// it first — so `simple`, `from` and `to` are accepted and dropped. Declaring
/// them as ignored fields would be the same contract spelled as dead code.
///
/// The server re-parses `raw` through the JOURNAL's own grammar
/// ([`parse_period_spec`]), which is what guarantees a projected line fires on
/// the days the same line fires on once it is saved to disk and read back. A
/// body with no `raw` at all is still a `400` (a missing required field), so
/// the typo this guard usually catches is still caught.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PeriodIn {
    raw: String,
}

/// A growth rate on the way in. `rate` is a fraction; `unit` is
/// `week`|`month`|`year`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrowthIn {
    rate: WireDecIn,
    unit: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioLineIn {
    id: String,
    group: String,
    /// `flow` | `asset`. Absent reads as `flow`, so a body written before asset
    /// rows existed means exactly what it used to; an unrecognized value is a
    /// `400` rather than a silent fallback, because the two roles project
    /// entirely different numbers.
    #[serde(default)]
    role: Option<String>,
    account: String,
    amount: AmountIn,
    /// The MAGNITUDE the user typed, when this line was edited (plan 26 B6).
    ///
    /// Present: the server signs it by the account's TYPE — negative for
    /// revenue, as hledger writes income ([`signed_entry`]) — and it replaces
    /// `amount.quantity`; the commodity and precision still come from
    /// `amount`. A client therefore never carries the sign rule, and moving a
    /// line from `expenses:rent` to `income:rent` re-signs it here.
    ///
    /// Absent: `amount.quantity` is taken as already signed, exactly as every
    /// body written before this field existed meant — which is also how an
    /// unedited line keeps a deliberate sign the type rule would not give it
    /// (a refund written negative under an expense account).
    #[serde(default)]
    entry: Option<WireDecIn>,
    period: PeriodIn,
    #[serde(default)]
    growth: Option<GrowthIn>,
    /// Asset rows only; a flow line that carries one is a `400`, because it
    /// would be a field the engine reads from nowhere and the file cannot
    /// write.
    #[serde(default)]
    opening: Option<AmountIn>,
    #[serde(default)]
    note: String,
    /// The counter-account. Absent or `null` is a pre-plan-28 line.
    #[serde(default)]
    counter: Option<String>,
    /// Asset rows only; a flow line that carries one is a `400`.
    #[serde(default)]
    gains: Option<String>,
    /// `journal` | `unbudgeted`. Absent reads as `journal`; an unrecognized
    /// value is a `400` rather than a silent fallback.
    #[serde(default)]
    source: Option<String>,
    /// The outbound line's derived `section`, accepted and DROPPED so a client
    /// can hand a line straight back — the same courtesy [`PeriodIn`] extends
    /// to `simple`/`from`/`to`. The server re-derives it; a section is never
    /// an instruction.
    #[serde(default, rename = "section")]
    _section: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventPostingIn {
    account: String,
    amount: AmountIn,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioEventIn {
    id: String,
    date: String,
    #[serde(default)]
    description: String,
    postings: Vec<EventPostingIn>,
    #[serde(default)]
    counter: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioIn {
    #[serde(default)]
    name: String,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    lines: Vec<ScenarioLineIn>,
    #[serde(default)]
    events: Vec<ScenarioEventIn>,
    #[serde(default)]
    cash: Option<String>,
}

/// `POST /api/projections/run`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunRequest {
    scenario: ScenarioIn,
    /// The day opening balances are taken; the first projected bucket is the
    /// next WHOLE one after it. Defaults to today.
    #[serde(default)]
    as_of: Option<String>,
    #[serde(default)]
    interval: Option<String>,
    #[serde(default)]
    count: Option<usize>,
    /// How many full ACTUAL periods the statements show before the forecast
    /// (Decision 6). Default [`DEFAULT_LOOKBACK`]; `0` is allowed.
    #[serde(default)]
    lookback: Option<usize>,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    value_in: Option<String>,
}

/// `lookback`, defaulted and bounded: the statements carry `lookback + count`
/// columns, so together they are held to the same [`MAX_BUCKETS`] every
/// other report's columns are.
fn checked_lookback(lookback: Option<usize>, count: usize) -> Result<usize, AppError> {
    let lookback = lookback.unwrap_or(DEFAULT_LOOKBACK);
    match lookback.checked_add(count) {
        Some(columns) if columns <= MAX_BUCKETS => Ok(lookback),
        _ => Err(AppError::BadRequest(format!(
            "lookback {lookback} plus count {count} is more than {MAX_BUCKETS} columns"
        ))),
    }
}

/// `?end=&count=&depth=` — the seed window.
///
/// Deliberately NO `interval`: the seeded lines are MONTHLY and their figures
/// are monthly averages, so an interval param could only offer a window whose
/// buckets do not match the lines it produces. The three that remain are
/// resolved through the same [`Window`] every other report uses, so an
/// out-of-range `count` is refused here exactly as it is there.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SeedQuery {
    end: Option<String>,
    count: Option<usize>,
    depth: Option<usize>,
}

// ===========================================================================
// Decoding a request body into the engine's model
// ===========================================================================

fn amount_from_wire(amount: &AmountIn) -> Result<Amount, AppError> {
    let commodity = amount.commodity.trim();
    if commodity.is_empty() {
        return Err(AppError::BadRequest(
            "a scenario amount needs a commodity".to_string(),
        ));
    }
    let quantity = dec_from_wire(&amount.quantity)?;
    let precision = amount.precision.unwrap_or(quantity.places);
    // The same bound `dec_from_wire` puts on `places` (SEC-5), for the same
    // reason: the engine rounds to this at every step and the writer pads to it,
    // so it is a scale the client chooses, and no journal amount re-reads wider.
    if precision > MAX_PARSE_PLACES {
        return Err(AppError::BadRequest(format!(
            "amount precision {precision} is out of range (expected 0..={MAX_PARSE_PLACES})"
        )));
    }
    Ok(Amount {
        commodity: Commodity(commodity.to_string()),
        quantity,
        // A style with only `precision` load-bearing. The engine reads nothing
        // else from it (the file WRITER in Phase 3 will, and takes the
        // journal's own style then) — see `WireAmount::precision`.
        style: AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision,
        },
        cost: None,
    })
}

fn account_from_wire(account: &str) -> Result<AccountName, AppError> {
    let trimmed = account.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "a scenario line needs an account".to_string(),
        ));
    }
    Ok(AccountName(trimmed.to_string()))
}

/// An optional account: absent, `null` and blank all mean "none".
fn optional_account(account: Option<&String>) -> Result<Option<AccountName>, AppError> {
    match account.map(|account| account.trim()) {
        None | Some("") => Ok(None),
        Some(account) => account_from_wire(account).map(Some),
    }
}

fn growth_from_wire(growth: &GrowthIn) -> Result<Growth, AppError> {
    let unit = GrowthUnit::parse(&growth.unit).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unknown growth unit '{}' (expected week, month or year)",
            growth.unit
        ))
    })?;
    Ok(Growth {
        rate: dec_from_wire(&growth.rate)?,
        unit,
    })
}

fn role_from_wire(role: Option<&str>) -> Result<LineRole, AppError> {
    match role.map(str::trim) {
        None | Some("") => Ok(LineRole::Flow),
        Some(other) => LineRole::parse(other).ok_or_else(|| {
            AppError::BadRequest(format!(
                "unknown line role '{other}' (expected flow or asset)"
            ))
        }),
    }
}

fn source_from_wire(source: Option<&str>) -> Result<LineSource, AppError> {
    match source.map(str::trim) {
        None | Some("") | Some("journal") => Ok(LineSource::Journal),
        Some("unbudgeted") => Ok(LineSource::Unbudgeted),
        Some(other) => Err(AppError::BadRequest(format!(
            "unknown line source '{other}' (expected journal or unbudgeted)"
        ))),
    }
}

/// A decoded scenario body whose edited lines still await their sign.
///
/// The sign of an `entry` depends on the journal's declared account types,
/// which are read inside a `compute` slot, while decoding (and bounding) a body
/// happens before one is claimed. So decoding keeps the magnitudes aside and
/// [`DecodedScenario::signed`] applies them once the types are at hand.
struct DecodedScenario {
    scenario: Scenario,
    /// One per line, in order: the typed magnitude, or `None` for a line whose
    /// `amount.quantity` is already signed.
    entries: Vec<Option<Dec>>,
}

impl DecodedScenario {
    /// The scenario, each `entry` signed by its account's type.
    fn signed(self, declared: &BTreeMap<String, AccountType>) -> Result<Scenario, AppError> {
        let Self {
            mut scenario,
            entries,
        } = self;
        for (line, entry) in scenario.lines.iter_mut().zip(entries) {
            if let Some(entry) = entry {
                line.amount.quantity =
                    signed_entry(&line.account.0, entry, declared).map_err(|error| {
                        AppError::BadRequest(format!(
                            "line '{}' has an amount out of range: {error}",
                            line.id
                        ))
                    })?;
            }
        }
        Ok(scenario)
    }
}

/// Validate a scenario body and turn it into the engine's model.
///
/// The counts are checked FIRST, before a single line is decoded: the point of
/// the limits is to refuse an oversized scenario cheaply, and decoding ten
/// thousand lines in order to discover there are ten thousand of them is not
/// that.
fn scenario_from_wire(scenario: &ScenarioIn) -> Result<DecodedScenario, AppError> {
    if scenario.lines.len() > MAX_LINES {
        return Err(AppError::BadRequest(format!(
            "a scenario may have at most {MAX_LINES} lines (received {})",
            scenario.lines.len()
        )));
    }
    if scenario.events.len() > MAX_EVENTS {
        return Err(AppError::BadRequest(format!(
            "a scenario may have at most {MAX_EVENTS} events (received {})",
            scenario.events.len()
        )));
    }

    let lines = scenario
        .lines
        .iter()
        .map(|line| {
            let role = role_from_wire(line.role.as_deref())?;
            if role == LineRole::Flow && line.opening.is_some() {
                return Err(AppError::BadRequest(format!(
                    "line '{}' states an opening balance, which only an asset row has: a flow \
                     line has no balance to open",
                    line.id
                )));
            }
            let gains = optional_account(line.gains.as_ref())?;
            if role == LineRole::Flow && gains.is_some() {
                return Err(AppError::BadRequest(format!(
                    "line '{}' names a gains account, which only an asset row has",
                    line.id
                )));
            }
            Ok(ScenarioLine {
                counter: optional_account(line.counter.as_ref())?,
                gains,
                id: line.id.clone(),
                group: line.group.clone(),
                role,
                account: account_from_wire(&line.account)?,
                amount: amount_from_wire(&line.amount)?,
                // `simple`/`from`/`to` are echoes; `raw` is the fact.
                period: parse_period_spec(&line.period.raw),
                growth: line.growth.as_ref().map(growth_from_wire).transpose()?,
                opening: line.opening.as_ref().map(amount_from_wire).transpose()?,
                note: line.note.clone(),
                source: source_from_wire(line.source.as_deref())?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    let events = scenario
        .events
        .iter()
        .map(|event| {
            if event.postings.len() > MAX_EVENT_POSTINGS {
                return Err(AppError::BadRequest(format!(
                    "an event may have at most {MAX_EVENT_POSTINGS} postings (event '{}' \
                     has {})",
                    event.id,
                    event.postings.len()
                )));
            }
            let postings: Vec<Posting> = event
                .postings
                .iter()
                .map(|posting| {
                    Ok(virtual_posting(
                        account_from_wire(&posting.account)?,
                        amount_from_wire(&posting.amount)?,
                    ))
                })
                .collect::<Result<_, AppError>>()?;
            Ok(ScenarioEvent {
                id: event.id.clone(),
                // The one field here that reaches bucket math, so it gets the
                // same validation a `?end=` param does (RPT-4).
                date: checked_date("event date", &event.date)?,
                description: event.description.clone(),
                postings,
                counter: optional_account(event.counter.as_ref())?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    let entries = scenario
        .lines
        .iter()
        .map(|line| line.entry.as_ref().map(dec_from_wire).transpose())
        .collect::<Result<Vec<_>, AppError>>()?;

    Ok(DecodedScenario {
        scenario: Scenario {
            name: scenario.name.clone(),
            created: scenario.created.clone(),
            updated: scenario.updated.clone(),
            lines,
            events,
            cash: optional_account(scenario.cash.as_ref())?,
        },
        entries,
    })
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `POST /api/projections/run` — project a scenario forward.
///
/// Body: `{scenario, asOf?, interval?, count?, lookback?, depth?, valueIn?}`.
/// The scenario travels in the body precisely so that UNSAVED edits project;
/// nothing here touches the filesystem.
///
/// - `asOf=YYYY-MM-DD` (default: today) — the caller's today. The projection
///   starts at the CUTOFF (plan 28 Decision 5): the start of the period
///   containing the earlier of this and the day after the latest real
///   transaction. It comes back as `actualsEnd`.
/// - `interval`, `count`, `depth` — as `/api/reports/cashflow`, through the same
///   [`Window`]. `depth` rolls up the cash flow statement's account lines.
/// - `lookback` (default 2) — full actual periods the statements show before
///   the forecast; `lookback + count` is held to `MAX_BUCKETS`.
/// - `valueIn=$` — the commodity balances are valued into. The cash series
///   adds generated flows unvalued, and warns about a scenario in another
///   commodity; the statements value every column as the Reports tab does.
pub(crate) async fn run(
    State(state): State<AppState>,
    payload: Result<Json<RunRequest>, JsonRejection>,
) -> Result<Json<WireProjection>, AppError> {
    let request = json_body(payload)?;
    let snapshot = state.snapshot();
    let window = Window::resolve_named(
        "asOf",
        request.as_of,
        request.interval.as_deref(),
        request.count,
        request.depth,
    )?;
    let lookback = checked_lookback(request.lookback, window.count)?;
    let value_in = request
        .value_in
        .map(|symbol| symbol.trim().to_string())
        .filter(|symbol| !symbol.is_empty())
        .map(Commodity);
    // Decoded (and bounded) BEFORE a `compute` slot is claimed: a scenario we
    // are going to refuse must not first wait for a core.
    let decoded = scenario_from_wire(&request.scenario)?;

    compute(move || {
        let journal = &snapshot.journal;
        let shape = StatementShape::of(journal);
        let scenario = decoded.signed(&shape.declared)?;
        // ONE expansion serves every reading: the cash series and runway are
        // read off it inside `project`, and the statements below are built
        // over the same generated transactions and prices.
        let projection = project(
            &scenario,
            &journal.transactions,
            &journal.prices,
            &ProjectionOpts {
                today: &window.end,
                lookback,
                interval: window.interval,
                count: window.count,
                depth: window.depth,
                declared: &shape.declared,
                value_in,
            },
        )?;
        let statements = projected_statements(journal, &projection, &shape, window.depth)?;
        Ok(WireProjection::new(&projection, statements))
    })
    .await
}

/// `GET /api/projections/seed` — a starting scenario built from the journal.
///
/// The journal's `~` rules become authored lines; every revenue or expense
/// category those rules do not mention becomes one MONTHLY line at its average
/// over the window, flagged `source: "unbudgeted"` so the table can say it is an
/// estimate from history rather than something the user wrote.
///
/// - `end=YYYY-MM-DD` (default: today) — the last day of the averaging window.
/// - `count=N` (default: 12) — how many monthly buckets to average over.
/// - `depth=N` (default: 2) — how finely the unbudgeted categories are named.
///
/// The unbudgeted half is [`ledgeline_core::reports::budget_gaps`], unchanged —
/// the same function, over the same window, that `/api/budget/gaps` serves.
pub(crate) async fn seed(
    State(state): State<AppState>,
    Query(query): Query<SeedQuery>,
) -> Result<Json<WireScenario>, AppError> {
    let snapshot = state.snapshot();
    // `None` for the interval, so it resolves to monthly — which is the only
    // interval a monthly average can be read over.
    let window = Window::resolve(query.end, None, query.count, query.depth)?;
    debug_assert_eq!(window.interval, Interval::Monthly);

    compute(move || {
        let journal = &snapshot.journal;
        let declared: BTreeMap<String, _> = declared_types(&account_decls(journal));
        let scenario = seed_scenario(
            &journal.transactions,
            &journal.periodic_transactions,
            &SeedOpts {
                window: &BudgetOpts {
                    end: &window.end,
                    interval: window.interval,
                    count: window.count,
                    depth: window.depth,
                    budget_desc: None,
                },
                declared: &declared,
                styles: &journal.commodity_styles,
            },
        )?;
        Ok(WireScenario::new(&scenario, &declared))
    })
    .await
}

// ===========================================================================
// Scenario FILES (plan 22, Phase 3)
// ===========================================================================
//
// Three routes over the `*.journal` files in the open journal's own directory
// tree: list them, read one as a scenario, write one back.
//
// # Five things this write path does NOT do, and `budget_api` does
//
// They are the reason this is its own pipeline rather than a call into that
// one, and each is a consequence of the same fact: **a projection file is by
// definition not included from the main journal.**
//
//  1. **Resolution is discovery-set membership, not `Journal::source_files`.**
//     `budget_api::journal_path` resolves a handle by membership in the set of
//     files the journal was parsed from, and a projection file is not in it —
//     so that approach would `404` every single one. The set here is scanned in
//     THIS request and matched by exact string equality, exactly as `rules_api`
//     does and for the same reason its header gives.
//  2. **Validation is a STANDALONE parse.** `parse_journal_with_overrides`
//     against the main journal cannot see a file the main journal does not
//     include, so the overridden text would be parsed and then ignored. The
//     engine re-parses the written text on its own
//     (`serialize::write_scenario`) and requires the scenario to read back as
//     the numbers that were asked for.
//  3. **`state.reopen_editor()` is never called.** It re-opens the main
//     journal, which a non-included file cannot have changed. Calling it would
//     buy a full reparse and republish for nothing.
//  4. **No directory is ever created.** `Discovery::resolve_new` requires a
//     real, non-symlink parent that already exists; the Save As dialog offers
//     the directories the scan found, and making new ones is the user's job.
//  5. **A create is `O_EXCL`, not `atomic_write`.** The refusal to overwrite is
//     the KERNEL's, decided atomically at the open — `atomic_write`'s
//     temp-and-rename would happily replace an existing file, which is the
//     property that makes it right for an update and wrong for a create.
//     `revision: ""` means create, the same `NEW_FILE_REVISION` spelling the
//     rules editor and `hledger.conf` already use.
//
// # Decision 5 is enforced HERE
//
// "Saving is always Save As. The Projections tab never writes to
// `budget.journal` or to anything reachable by `include` from the main
// journal." The discovery scan cannot enforce that — it walks a directory tree
// and has no idea which files the journal includes — so the `PUT` checks the
// resolved path against `Journal::source_files` and refuses. A `GET` does not:
// the ask says to "default to using the active budget file", so reading one is
// the feature working.
//
// # Write serialization is `state.import_writes()`
//
// Deliberately not a new mutex. A projection file IS a journal file, and
// nothing stops a user from `include`-ing one from their main journal later;
// sharing the lock costs a little contention and removes a whole class of
// future bug.

/// The longest id accepted, in bytes — `PATH_MAX` on macOS. An id longer than
/// the platform's own path limit cannot name a file that exists, so this
/// refuses at no cost what the filesystem would refuse anyway, and it bounds
/// the string every error message below quotes.
const MAX_ID_BYTES: usize = 1024;

/// How many `/`-separated components an id may have.
///
/// **This is the scan's `MAX_PROJECTION_DEPTH + 1`**, and the `+ 1` is
/// load-bearing: the deepest file the scan can return has eight directory
/// components plus a file name, and a cap of eight would refuse an id the scan
/// itself had just handed out — the file would appear in the index and then
/// not open.
const MAX_ID_COMPONENTS: usize = 9;

/// The `revision` that means **"there is no file yet"**.
///
/// A `PUT` carrying it is a create; anything else is an edit against bytes that
/// exist. It can never collide with a real one — [`Fingerprint`] tokens are
/// always `LEN-HASH` in hex — and it is the spelling `rules_api` and
/// `import_api` already use, so the SPA has one convention rather than three.
const NEW_FILE_REVISION: &str = "";

/// The largest scenario file this module will read into memory.
///
/// Deliberately larger than the rules editor's cap, because a journal is a
/// journal: a user may point the tab at a file with a year of transactions in
/// it and only a handful of `~` rules. 8 MiB is a very large hand-maintained
/// journal and still a bounded read.
const MAX_SCENARIO_BYTES: usize = 8 << 20;

// ---------------------------------------------------------------------------
// Wire
// ---------------------------------------------------------------------------

/// One `*.journal` file in the listing.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireProjectionFile {
    /// The file's path relative to the journal directory — the only handle a
    /// client ever gets, and never an absolute path.
    id: String,
    /// The display fallback, from the filename.
    label: String,
    /// `; projection: <name>`, for a `projection-*.journal` that has one.
    name: Option<String>,
    created: Option<String>,
    updated: Option<String>,
    /// Whether this is a `projection-*.journal` — the group the picker shows
    /// first, under its own heading.
    is_projection: bool,
    size_bytes: u64,
    /// Whether a `PUT` to this id could succeed. `false` for a file the main
    /// journal includes: decision 5 forbids the Projections tab from writing to
    /// a plan of record, and a picker that offered Save over one would be
    /// offering something that is about to be refused.
    writable: bool,
}

/// `GET /api/projections`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireProjectionIndex {
    /// The journal directory's own final component, for a heading. Never a
    /// path.
    root_label: String,
    /// Whether this server has an editor bound at all.
    editable: bool,
    /// A scan cap was hit and the list is a subset. Surfaced so a user is never
    /// silently shown one.
    truncated: bool,
    files: Vec<WireProjectionFile>,
    /// The directories the scan found a journal in, relative, `""` for the
    /// root — what the Save As dialog offers instead of a free-text path.
    directories: Vec<String>,
    warnings: Vec<String>,
}

impl WireProjectionIndex {
    /// The answer when no journal is open, so there is no scan root.
    fn without_journal(editable: bool) -> Self {
        Self {
            root_label: "journal".to_string(),
            editable,
            truncated: false,
            files: Vec::new(),
            directories: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// `GET`/`PUT /api/projections/{*id}` — one file, read as a scenario.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireScenarioFile {
    id: String,
    label: String,
    /// [`Fingerprint::token`] over the file's **raw bytes** — the value a save
    /// sends back as its `revision`, so it cannot land on top of an edit made
    /// elsewhere. Never a hash of rendered text, which would be blind to
    /// exactly the bytes this writer preserves but does not model.
    revision: String,
    /// Whether a `PUT` to this id could succeed. See
    /// [`WireProjectionFile::writable`].
    writable: bool,
    scenario: WireScenario,
}

/// `PUT /api/projections/{*id}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveScenarioRequest {
    /// The revision read with the file, or `""` to CREATE.
    revision: String,
    scenario: ScenarioIn,
}

// ---------------------------------------------------------------------------
// Ids, and the sentence each refusal gets
// ---------------------------------------------------------------------------

/// Syntactic validation, before ANY filesystem call.
///
/// This is why the 400-versus-404 split is decided on SHAPE rather than on
/// existence: a route that answered differently for `../../etc/passwd.journal`
/// depending on what is on disk would be an existence oracle.
fn validate_id(id: &str) -> Result<(), AppError> {
    let components: Vec<&str> = id.split('/').collect();
    let well_formed = !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && !id.starts_with('/')
        && !id.contains('\\')
        && !id.contains(':')
        && !id.chars().any(|c| c.is_ascii_control())
        && is_journal_name(id)
        && components.len() <= MAX_ID_COMPONENTS
        && components
            .iter()
            .all(|part| !part.is_empty() && *part != "." && *part != "..");
    if well_formed {
        Ok(())
    } else {
        Err(malformed_id(id))
    }
}

/// The caller's own string, escaped and clipped, ready to go in an error body.
///
/// `{:?}` escapes control characters, so a NUL or an ANSI escape sequence in a
/// hostile id reaches a dialog as `\u{0}` rather than as itself.
fn quoted(value: &str) -> String {
    /// Long enough for any real id, short enough that a hostile one cannot make
    /// a response large.
    const MAX_QUOTED_CHARS: usize = 120;
    let clipped: String = value.chars().take(MAX_QUOTED_CHARS).collect();
    if clipped.len() < value.len() {
        format!("{clipped:?}…")
    } else {
        format!("{clipped:?}")
    }
}

/// The one `400` every syntactic rejection returns.
///
/// One sentence for all of them on purpose: the differences between them are
/// about the caller's own input, and spelling each out buys a client nothing
/// while giving anyone probing the route a finer-grained signal.
fn malformed_id(id: &str) -> AppError {
    AppError::BadRequest(format!(
        "{} is not a usable projection id: an id is the file's path relative to the journal \
         directory, forward-slash separated, at most {MAX_ID_COMPONENTS} plain components and \
         {MAX_ID_BYTES} bytes, and it must end in `.journal`",
        quoted(id)
    ))
}

/// The one `404` every resolution failure returns.
///
/// **Identical for every cause** — not scanned, not there, not a regular file,
/// a symlink, outside the root, skipped by a cap — so the route cannot be used
/// to tell any of those apart. It names the caller's own id and nothing else.
fn unresolved(id: &str) -> AppError {
    AppError::NotFound(format!(
        "no journal file {} is available beside this journal",
        quoted(id)
    ))
}

/// The one `409`, shared by all three staleness checks (the revision the client
/// sent, the re-read immediately before the write, and the inode identity).
///
/// All three mean the same thing to the user and call for the same action, and
/// distinguishing them would leak the timing of somebody else's write.
fn stale(id: &str) -> AppError {
    AppError::Conflict(format!(
        "{} changed on disk since you opened it, so nothing was written. Re-open it and re-apply \
         your changes.",
        quoted(id)
    ))
}

/// A `500` for an I/O failure while READING.
///
/// The [`std::io::Error`] is surfaced verbatim, which is safe for the reason
/// `rules_api::read_failed` records at length: an `io::Error` that std itself
/// produced carries no path, and every error on this path comes straight from
/// `File::open` or `read_to_end`.
fn read_failed(id: &str, error: &std::io::Error) -> AppError {
    AppError::unless_not_downloaded(
        error,
        AppError::Internal(format!("could not read {}: {error}", quoted(id))),
    )
}

/// A `500` for an I/O failure while WRITING — and deliberately only the
/// [`std::io::ErrorKind`], because `atomic_write` can fail with an error *we*
/// built that names the journal directory, which is precisely the disclosure
/// the error sentences here exist to prevent.
fn write_failed(id: &str, error: &std::io::Error) -> AppError {
    AppError::unless_not_downloaded(
        error,
        AppError::Internal(format!(
            "could not write {}: {}. Nothing was changed.",
            quoted(id),
            error.kind()
        )),
    )
}

/// How a [`CreateRefusal`] is reported.
///
/// `OutsideRoot` and `DirectoryMissing` collapse into the ordinary `404`: both
/// are about places this server declined to look, and a message that changed
/// when a path outside the root happened to exist is a filesystem oracle.
/// `Exists` is safe to report as itself — it is only reachable for a confined,
/// non-hidden `.journal` name below the root, which is the exact set
/// `GET /api/projections` already publishes — and the Save As dialog shows it
/// as "a file already exists there".
fn create_refused(id: &str, refusal: CreateRefusal) -> AppError {
    match refusal {
        CreateRefusal::Malformed => malformed_id(id),
        CreateRefusal::OutsideRoot | CreateRefusal::DirectoryMissing => unresolved(id),
        CreateRefusal::Exists => AppError::Conflict(format!(
            "a file already exists at {}. Choose another name, or open that file and save over it.",
            quoted(id)
        )),
    }
}

/// A [`SerializeError`] as an HTTP failure.
///
/// `Invalid` is the CALLER's — a name with a comma in it, an account that would
/// split at a double space — so it is a `400` carrying the engine's own
/// sentence, which already explains what hledger would have done with it. The
/// other two are OURS: the renderer produced something that did not read back,
/// which is a bug in this server and not something a caller can fix by sending
/// different bytes.
fn serialize_failed(id: &str, error: &SerializeError) -> AppError {
    match error {
        SerializeError::Invalid(message) => AppError::BadRequest(format!(
            "{} cannot be written as you asked: {message}",
            quoted(id)
        )),
        other => AppError::Internal(format!(
            "{} was NOT written, because the result did not read back correctly: {other}",
            quoted(id)
        )),
    }
}

// ---------------------------------------------------------------------------
// Reading a file
// ---------------------------------------------------------------------------

/// Read a discovered file's bytes, bounded and UTF-8 checked, and fingerprint
/// them.
///
/// The [`Fingerprint`] is over the **raw bytes**, before the UTF-8 decode,
/// which is the only hash a save may gate on: a hash of rendered text is blind
/// to trailing whitespace, CRLF and everything else this writer preserves but
/// does not represent, so it would let a save clobber someone else's edit and
/// report success.
fn read_file(path: &ProjectionPath, id: &str) -> Result<(String, Fingerprint), AppError> {
    // The cap bounds the READ itself rather than trimming afterwards, so an
    // enormous file is never held in memory even briefly: the vault reads at
    // most one byte past it and refuses the file. The open never follows a
    // symlink and insists on a regular file (`pinned.rs` under a `LocalVault`).
    let cap = u64::try_from(MAX_SCENARIO_BYTES).unwrap_or(u64::MAX);
    let raw = match path.vault().read_pinned(path.vault_path(), Some(cap)) {
        Ok((raw, _)) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => {
            return Err(AppError::BadRequest(format!(
                "{} is larger than {MAX_SCENARIO_BYTES} bytes, so it is listed but cannot be \
                 opened as a projection",
                quoted(id)
            )));
        }
        Err(error) => return Err(read_failed(id, &path_free(error))),
    };
    let fingerprint = Fingerprint::of_bytes(&raw);
    let text = String::from_utf8(raw).map_err(|_| {
        AppError::BadRequest(format!(
            "{} is not valid UTF-8. Ledgeline reads and writes UTF-8 journals only; converting it \
             first (e.g. `iconv -f latin1 -t utf-8`) is what keeps a character from being silently \
             rewritten.",
            quoted(id)
        ))
    })?;
    Ok((text, fingerprint))
}

/// Whether a `PUT` to `path` is allowed — **decision 5's guard**.
///
/// A file the main journal was parsed from is a plan of record, and the
/// Projections tab never writes to one. Compared against
/// `Journal::source_files`, which holds the vault's keys (canonical paths on the
/// desktop), as does [`ProjectionPath::as_path`] — both built by
/// [`ledgeline_core::vault::Vault::key`] — so this is an equality test between
/// two keys and not path arithmetic.
fn writable_path(sources: &[PathBuf], path: &FsPath) -> bool {
    !sources.iter().any(|source| source.as_path() == path)
}

/// One discovered file, as the listing renders it.
fn wire_file(found: &DiscoveredProjection, sources: &[PathBuf]) -> WireProjectionFile {
    WireProjectionFile {
        id: found.id.clone(),
        label: found.label.clone(),
        name: found.name.clone(),
        created: found.created.clone(),
        updated: found.updated.clone(),
        is_projection: found.is_projection,
        size_bytes: found.size_bytes,
        writable: writable_path(sources, found.path().as_path()),
    }
}

/// Read one discovered file as a scenario, with its revision.
fn open_file(
    found: &DiscoveredProjection,
    id: &str,
    writable: bool,
    declared: &BTreeMap<String, AccountType>,
) -> Result<WireScenarioFile, AppError> {
    let (text, fingerprint) = read_file(found.path(), id)?;
    let name = found.path().as_path().to_string_lossy().to_string();
    // The FALLBACK display name is the filename's label, so a file with no
    // `; projection:` marker still opens with a name — "a file without it still
    // loads, and takes its name from its filename".
    let scenario = scenario_from_text(&text, &name, &found.label, declared)
        .map_err(|error| serialize_failed(id, &error))?;
    Ok(WireScenarioFile {
        id: id.to_string(),
        label: found.label.clone(),
        revision: fingerprint.token(),
        writable,
        scenario: WireScenario::new(&scenario, declared),
    })
}

/// Where the journal lives — the vault the scan walks and the main file whose
/// directory is its root, and the only thing that makes a discovery set exist
/// at all. Taken from `snapshot`, so it is the storage of the journal served.
fn main_journal_file(snapshot: &Snapshot) -> Option<Arc<JournalLocation>> {
    snapshot.storage()
}

/// A storage failure as an [`std::io::Error`] that carries no path: the
/// underlying OS error when there is one, else just its kind. The vault's own
/// errors name a (relative) path, and [`read_failed`]/[`write_failed`] put
/// only the caller's id in a sentence.
fn path_free(error: VaultError) -> std::io::Error {
    match error {
        VaultError::Io { source, .. } => source,
        other => std::io::Error::from(other.kind()),
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// The whole of a CREATE, synchronously, on the blocking pool.
fn create_scenario(
    discovery: &Discovery,
    id: &str,
    scenario: &Scenario,
    declared: &BTreeMap<String, AccountType>,
) -> Result<WireScenarioFile, AppError> {
    // `resolve_new`, since no scan can have found a file that is not there. It
    // is the only place in either crate that joins a caller's string onto the
    // root, and it does so under every guard its docs list.
    let path: ProjectionPath = discovery
        .resolve_new(id)
        .map_err(|refusal| create_refused(id, refusal))?;
    let name = path.as_path().to_string_lossy().to_string();
    let text = new_file(scenario, &name, declared).map_err(|error| serialize_failed(id, &error))?;

    // A brand-new file, refusing to touch one that is already there:
    // `Vault::create_new` is `O_CREAT | O_EXCL | O_NOFOLLOW` under a
    // `LocalVault`, so the refusal is the **kernel's**, decided atomically at
    // the moment of the open. That matters more than it looks:
    // [`Discovery::resolve_new`]'s own existence check expires the instant it
    // returns, so a create that leant on it would have a window in which
    // another process could put a file there and have it silently truncated.
    // Deliberately **not** `write_atomic`, whose temp-file-and-rename would
    // happily replace an existing file. Durable (`fsync`ed) before it returns.
    // The mode is the process umask's (`create_new_shared`), unlike a save,
    // which carries an existing file's mode forward: there is no previous mode
    // to carry, and a stricter one than the user's own editor would give them
    // is a surprise rather than a protection.
    path.vault()
        .create_new_shared(path.vault_path(), text.as_bytes())
        .map(|_| ())
        .map_err(path_free)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                // Lost the race with another writer between `resolve_new` and the
                // open. The kernel refused it; this is only how that is reported.
                create_refused(id, CreateRefusal::Exists)
            } else {
                write_failed(id, &error)
            }
        })?;

    let label = label_for(id);
    let written = scenario_from_text(&text, &name, &label, declared)
        .map_err(|error| serialize_failed(id, &error))?;
    // The revision comes from what we WROTE, never from a re-read: a re-read
    // could pick up somebody else's write and hand this client a token for
    // bytes it has never seen, which is the precise way to make the next save
    // clobber that person silently.
    Ok(WireScenarioFile {
        id: id.to_string(),
        revision: Fingerprint::of_bytes(text.as_bytes()).token(),
        writable: true,
        scenario: WireScenario::new(&written, declared),
        label,
    })
}

/// The whole of an UPDATE, synchronously, on the blocking pool.
///
/// Every `?` here is a decision not to write. The single pinned replace is the
/// last statement that can have an effect, and everything above it is either a
/// read or a pure computation.
fn update_scenario(
    sources: &[PathBuf],
    discovery: &Discovery,
    id: &str,
    revision: &str,
    scenario: &Scenario,
    declared: &BTreeMap<String, AccountType>,
) -> Result<WireScenarioFile, AppError> {
    // Security layer 2: a set scanned in THIS request, matched by exact string
    // equality. `root.join(id)` is unreachable from here.
    let found = discovery.resolve(id).ok_or_else(|| unresolved(id))?;
    let path = found.path().as_path();
    if !writable_path(sources, path) {
        // Decision 5. A `GET` of this same id works, deliberately — reading the
        // active budget file is what the ask asked for.
        return Err(AppError::BadRequest(format!(
            "{} is part of your main journal, and a projection is never saved over a plan of \
             record. Save it under a new name instead.",
            quoted(id)
        )));
    }
    let name = path.to_string_lossy().to_string();

    let (text, fingerprint) = read_file(found.path(), id)?;
    // Checked BEFORE anything is rendered, so a client editing an older parse is
    // told the file moved rather than being handed a rendering failure that
    // describes the wrong problem and suggests the wrong fix.
    if fingerprint.token() != revision {
        return Err(stale(id));
    }

    let doc = ProjectionDoc::parse(&text, &name, declared)
        .map_err(|error| serialize_failed(id, &error))?;
    // The engine's own second opinion: splice, re-parse the whole result as a
    // journal, and require the scenario to read back as the numbers asked for.
    let new_text =
        write_scenario(&doc, scenario, &name).map_err(|error| serialize_failed(id, &error))?;

    if new_text == text {
        // A no-op writes NOTHING. Writing byte-identical content still bumps
        // mtime, and a user's own `entr` or `hledger import` watch loop would
        // see a spurious change — the lesson `rules_api` and `budget_api` both
        // record. The unchanged document is returned, so the client still gets
        // a fresh (identical) revision.
        //
        // Built from the `text` and `fingerprint` already in scope rather than
        // through `open_file`, which would re-open, re-read and re-parse this
        // file inside the write lock to reach the same two values. It is also
        // the more honest answer: the revision returned is the one that was
        // just checked against `revision`, over the very bytes this scenario
        // was rendered from.
        let unchanged = scenario_from_text(&text, &name, &found.label, declared)
            .map_err(|error| serialize_failed(id, &error))?;
        return Ok(WireScenarioFile {
            id: id.to_string(),
            label: found.label.clone(),
            revision: fingerprint.token(),
            writable: true,
            scenario: WireScenario::new(&unchanged, declared),
        });
    }

    // Pinned to the scan's identity and to the revision the client edited.
    // The scan proved this name was a regular file inside the root; that proof
    // expired the moment the scan ended, and a name can become a symlink, a
    // FIFO or a different file entirely in between. `Vault::replace`
    // (`pinned::replace` under a `LocalVault`) re-checks both through the
    // descriptor it replaces, right up to the rename, and never follows a
    // link. A replace and not `create_new`: this file exists, and its mode is
    // carried forward (a projection carries account names and figures, so
    // `0600` staying `0600` is not hypothetical).
    let expected = Revision {
        fingerprint,
        identity: found.identity(),
    };
    found
        .path()
        .vault()
        .replace(found.path().vault_path(), new_text.as_bytes(), &expected)
        .map_err(|error| match error {
            VaultError::Changed { .. } => stale(id),
            other => write_failed(id, &path_free(other)),
        })?;

    // Deliberately NO `state.reopen_editor()`: it re-opens the main journal,
    // which a file the main journal does not include cannot have changed.
    let written = scenario_from_text(&new_text, &name, &found.label, declared)
        .map_err(|error| serialize_failed(id, &error))?;
    Ok(WireScenarioFile {
        id: id.to_string(),
        label: found.label.clone(),
        revision: Fingerprint::of_bytes(new_text.as_bytes()).token(),
        writable: true,
        scenario: WireScenario::new(&written, declared),
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/projections` — every `*.journal` file in the open journal's own
/// directory tree, `projection-*` first.
pub(crate) async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let editable = state.editing_enabled();
    let snapshot = state.snapshot();
    let Some(main) = main_journal_file(&snapshot) else {
        return Ok(no_store(WireProjectionIndex::without_journal(editable)));
    };
    let sources = snapshot.journal.source_files.clone();
    // Through `compute`: a directory walk on a cold or network-mounted journal
    // directory is exactly the blocking work its semaphore and `spawn_blocking`
    // exist for, and running it on a tokio worker would stall the runtime the
    // desktop GUI is hosted in.
    let Json(body) = compute(move || {
        let discovery = discover_in(Arc::clone(main.vault()), main.main());
        Ok(WireProjectionIndex {
            root_label: discovery.root_label(),
            editable,
            truncated: discovery.truncated,
            files: discovery
                .files
                .iter()
                .map(|found| wire_file(found, &sources))
                .collect(),
            directories: discovery.directories(),
            warnings: discovery.warnings.clone(),
        })
    })
    .await?;
    Ok(no_store(body))
}

/// `GET /api/projections/{*id}` — one journal file, read as a scenario.
pub(crate) async fn document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    validate_id(&id)?;
    // No journal file means no scan root, so no id resolves. Answering the
    // ordinary `404` keeps this route from distinguishing "no journal open"
    // from "no such file".
    let snapshot = state.snapshot();
    let Some(main) = main_journal_file(&snapshot) else {
        return Err(unresolved(&id));
    };
    let sources = snapshot.journal.source_files.clone();
    let Json(body) = compute(move || {
        // The MAIN journal's account types, because a projection file declares
        // none of its own and the asset/flow discriminator needs them
        // (`serialize`'s header says why a name test will not do).
        //
        // Inside the closure, where `run` and `seed` already do it: walking
        // every declared account and cloning its name twice is blocking work,
        // and on a tokio worker it stalls the runtime the desktop GUI is
        // hosted in — the same reason the scan below is in here.
        let declared = declared_types(&account_decls(&snapshot.journal));
        // `without_headers`: this route resolves exactly one id and never looks
        // at a listing's display name, so opening the other 199 files to read
        // four comment lines out of each is work thrown away.
        let discovery = Discovery::without_headers_in(Arc::clone(main.vault()), main.main());
        let found = discovery.resolve(&id).ok_or_else(|| unresolved(&id))?;
        let writable = writable_path(&sources, found.path().as_path());
        open_file(found, &id, writable, &declared)
    })
    .await?;
    Ok(no_store(body))
}

/// `PUT /api/projections/{*id}` — save a scenario. `revision: ""` creates.
///
/// The header comments are rewritten on every save: `created:` is carried
/// through from what the client loaded (or set to today for a new file), and
/// `updated:` is set to today. Neither is the client's to choose — a timestamp
/// a caller supplies is a timestamp that can say anything.
pub(crate) async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<SaveScenarioRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    validate_id(&id)?;
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    let snapshot = state.snapshot();
    let Some(main) = main_journal_file(&snapshot) else {
        return Err(unresolved(&id));
    };
    let sources = snapshot.journal.source_files.clone();
    // Decoded (and bounded) BEFORE the lock is taken and before a `compute`
    // slot is claimed: a scenario we are going to refuse must not first wait
    // for a core, and must not hold the write lock while it is refused.
    let mut decoded = scenario_from_wire(&request.scenario)?;
    let today = today_utc();
    decoded.scenario.created = Some(
        decoded
            .scenario
            .created
            .take()
            .unwrap_or_else(|| today.clone()),
    );
    decoded.scenario.updated = Some(today);
    let revision = request.revision;

    // `import_writes`, not a mutex of this route's own — see the section header.
    // A `tokio` mutex, because the guard is held across the blocking-pool
    // `.await` below.
    let _guard = state.import_writes().lock().await;
    let Json(body) = compute(move || {
        // The main journal's declared account types, inside the closure where
        // `run` and `seed` already do it: walking every declared account and
        // cloning its name twice is blocking work, and on a tokio worker it
        // stalls the runtime the desktop GUI is hosted in.
        //
        // It does now happen under `import_writes`, which the version above
        // this line did not. That is the right side to pay on: the lock
        // serializes writers to one journal directory, the runtime serializes
        // the whole application.
        let declared = declared_types(&account_decls(&snapshot.journal));
        let scenario = decoded.signed(&declared)?;
        // Plan 28: a pre-plan-28 group that does not balance is written with
        // the default counter-account the engine already books it to, so the
        // file becomes plain balanced hledger and no number moves.
        let history = History {
            txns: &snapshot.journal.transactions,
            prices: &snapshot.journal.prices,
        };
        let cutoff = forecast_cutoff(
            &today_utc(),
            latest_real_date(history.txns),
            Interval::Monthly,
        )?;
        let defaults = CounterDefaults::new(
            &history,
            &AccountTypes::from_declared(declared.clone()),
            &cutoff,
            scenario.cash.as_ref(),
            None,
        )?;
        let scenario = upgrade_legacy_counters(&scenario, &defaults);
        // Scanned in THIS request, never cached. That is the whole of security
        // layer 2: an id resolves against a set built from `read_dir` names
        // moments ago, so a cached set would be one that no longer describes
        // the disk, and a save would be authorized by a scan that happened
        // arbitrarily long ago.
        //
        // `without_headers` skips only the four comment lines at the top of
        // each *other* file in the tree, which this route never reads. The walk
        // and every guard on it are exactly as fresh — and this one ran up to
        // 200 pointless opens while holding that lock.
        let discovery = Discovery::without_headers_in(Arc::clone(main.vault()), main.main());
        if revision == NEW_FILE_REVISION {
            create_scenario(&discovery, &id, &scenario, &declared)
        } else {
            update_scenario(&sources, &discovery, &id, &revision, &scenario, &declared)
        }
    })
    .await?;
    Ok(no_store(body))
}
