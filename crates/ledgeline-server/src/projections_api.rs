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

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use ledgeline_core::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, PeriodSpec, Posting,
};
use ledgeline_core::parse::parse_period_spec;
use ledgeline_core::projections::{
    BalanceSeries, Growth, GrowthUnit, LineSource, Projection, ProjectionOpts, Runway, Scenario,
    ScenarioEvent, ScenarioLine, SeedOpts, project, seed_scenario, virtual_posting,
};
use ledgeline_core::reports::{
    BudgetOpts, Interval, account_decls, declared_types, periods::bucket_label,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::edit_api::{WireDecIn, dec_from_wire, json_body};
use crate::error::AppError;
use crate::reports_api::{
    Window, WireDec, WireMixed, WirePeriodReport, checked_date, compute, wire_mixed,
};

// ===========================================================================
// Limits
// ===========================================================================

/// The largest `POST /api/projections/run` body this server will read.
///
/// 256 KiB is roughly a thousand lines with generous account names and notes —
/// far past any scenario a human edits, and far short of what it costs to hold
/// while a `compute` slot is claimed. Enforced by axum's `DefaultBodyLimit` on
/// this route alone, the way `/api/import/stage` raises its own rather than
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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireScenarioLine {
    /// The LOGICAL row. Two bounded segments of one step change share it.
    id: String,
    /// The SOURCE RULE. Every posting of one `~` block shares it, which is what
    /// the engine's cash and net-worth guards are scoped to.
    group: String,
    account: String,
    amount: WireAmount,
    period: WireScenarioPeriod,
    growth: Option<WireGrowth>,
    note: String,
    /// `journal` (authored) | `unbudgeted` (estimated from history).
    source: &'static str,
}

impl From<&ScenarioLine> for WireScenarioLine {
    fn from(line: &ScenarioLine) -> Self {
        Self {
            id: line.id.clone(),
            group: line.group.clone(),
            account: line.account.0.clone(),
            amount: WireAmount::from(&line.amount),
            period: WireScenarioPeriod::from(&line.period),
            growth: line.growth.as_ref().map(WireGrowth::from),
            note: line.note.clone(),
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
}

impl From<&Scenario> for WireScenario {
    fn from(scenario: &Scenario) -> Self {
        Self {
            name: scenario.name.clone(),
            created: scenario.created.clone(),
            updated: scenario.updated.clone(),
            lines: scenario.lines.iter().map(WireScenarioLine::from).collect(),
            events: scenario
                .events
                .iter()
                .map(WireScenarioEvent::from)
                .collect(),
        }
    }
}

/// An opening balance and each bucket's closing balance.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBalanceSeries {
    /// The balance as of the request's `asOf`, before any projected flow.
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
    /// That bucket's key, so a client can label the crossing without indexing
    /// back into `buckets` and hoping the two arrays agree.
    bucket_key: String,
    /// That bucket's human label (`Mar 2028`).
    label: String,
    /// The bucket's last day — the date the closing balance is negative as of.
    date: String,
    /// Whole periods from the start of the projection (`bucket + 1`).
    periods: usize,
}

/// The answer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireProjection {
    /// Bucket keys, oldest → newest.
    buckets: Vec<String>,
    /// The first day of the first bucket — the next WHOLE bucket after `asOf`.
    start: String,
    /// Projected revenue and expenses, in CASH-FLOW ORIENTATION: revenue
    /// positive (an inflow), expenses negative (an outflow), so `totals` is the
    /// sum of `rows` and IS net income. This is the opposite sign convention to
    /// `/api/reports/incomestatement`, deliberately — see
    /// `projections::Projection::net_income`.
    net_income: WirePeriodReport,
    cash: WireBalanceSeries,
    net_worth: WireBalanceSeries,
    runway: Option<WireRunway>,
    /// Everything the projection could not do. Never empty silently: a dropped
    /// line always says so here.
    warnings: Vec<String>,
}

impl From<&Projection> for WireProjection {
    fn from(projection: &Projection) -> Self {
        Self {
            buckets: projection.buckets.clone(),
            start: projection.start.clone(),
            net_income: WirePeriodReport::from(&projection.net_income),
            cash: WireBalanceSeries::from(&projection.cash),
            net_worth: WireBalanceSeries::from(&projection.net_worth),
            runway: projection
                .runway
                .as_ref()
                .map(|runway| wire_runway(runway, &projection.buckets)),
            warnings: projection.warnings.clone(),
        }
    }
}

fn wire_runway(runway: &Runway, buckets: &[String]) -> WireRunway {
    let key = buckets.get(runway.bucket).cloned().unwrap_or_default();
    WireRunway {
        bucket: runway.bucket,
        label: bucket_label(&key),
        bucket_key: key,
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
    account: String,
    amount: AmountIn,
    period: PeriodIn,
    #[serde(default)]
    growth: Option<GrowthIn>,
    #[serde(default)]
    note: String,
    /// `journal` | `unbudgeted`. Absent reads as `journal`; an unrecognized
    /// value is a `400` rather than a silent fallback.
    #[serde(default)]
    source: Option<String>,
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
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    value_in: Option<String>,
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

fn source_from_wire(source: Option<&str>) -> Result<LineSource, AppError> {
    match source.map(str::trim) {
        None | Some("") | Some("journal") => Ok(LineSource::Journal),
        Some("unbudgeted") => Ok(LineSource::Unbudgeted),
        Some(other) => Err(AppError::BadRequest(format!(
            "unknown line source '{other}' (expected journal or unbudgeted)"
        ))),
    }
}

/// Validate a scenario body and turn it into the engine's model.
///
/// The counts are checked FIRST, before a single line is decoded: the point of
/// the limits is to refuse an oversized scenario cheaply, and decoding ten
/// thousand lines in order to discover there are ten thousand of them is not
/// that.
fn scenario_from_wire(scenario: &ScenarioIn) -> Result<Scenario, AppError> {
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
            Ok(ScenarioLine {
                id: line.id.clone(),
                group: line.group.clone(),
                account: account_from_wire(&line.account)?,
                amount: amount_from_wire(&line.amount)?,
                // `simple`/`from`/`to` are echoes; `raw` is the fact.
                period: parse_period_spec(&line.period.raw),
                growth: line.growth.as_ref().map(growth_from_wire).transpose()?,
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
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    Ok(Scenario {
        name: scenario.name.clone(),
        created: scenario.created.clone(),
        updated: scenario.updated.clone(),
        lines,
        events,
    })
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `POST /api/projections/run` — project a scenario forward.
///
/// Body: `{scenario, asOf?, interval?, count?, depth?, valueIn?}`. The scenario
/// travels in the body precisely so that UNSAVED edits project; nothing here
/// touches the filesystem.
///
/// - `asOf=YYYY-MM-DD` (default: today) — opening balances are taken here, and
///   the first projected bucket is the next WHOLE one after it.
/// - `interval`, `count`, `depth` — as `/api/reports/cashflow`, through the same
///   [`Window`].
/// - `valueIn=$` — the commodity opening balances are valued into. The projected
///   FLOWS are never valued (there are no market prices for a future date), so a
///   scenario in another commodity comes back with a warning saying so.
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
    let value_in = request
        .value_in
        .map(|symbol| symbol.trim().to_string())
        .filter(|symbol| !symbol.is_empty())
        .map(Commodity);
    // Decoded (and bounded) BEFORE a `compute` slot is claimed: a scenario we
    // are going to refuse must not first wait for a core.
    let scenario = scenario_from_wire(&request.scenario)?;

    compute(move || {
        let journal = &snapshot.journal;
        let declared = declared_types(&account_decls(journal));
        let projection = project(
            &scenario,
            &journal.transactions,
            &journal.prices,
            &ProjectionOpts {
                as_of: &window.end,
                interval: window.interval,
                count: window.count,
                depth: window.depth,
                declared: &declared,
                value_in,
            },
        )?;
        Ok(WireProjection::from(&projection))
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
        Ok(WireScenario::from(&scenario))
    })
    .await
}
