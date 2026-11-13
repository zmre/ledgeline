//! **Automatic price updates** (plan 41): the Holdings tab's "Update prices"
//! button, run by the engine in the background when the user turns on the
//! `autoPrices` setting.
//!
//! # What it does
//!
//! Shortly after launch, and then about hourly for as long as the app stays
//! open, it looks at the journal — LOCALLY, no network — and asks two things of
//! every security the Stocks tab's engine has ever seen held:
//!
//! 1. Is there a `P` directive for the **latest completed close** (or later)?
//! 2. For each of the last N Monday–Sunday weeks (`autoPricesWeeks`, default
//!    6), is there a `P` directive dated anywhere in that week?
//!
//! Each question is only asked of a symbol that NEEDS a price on that date:
//! one held at some point between that date and today
//! (`ledgeline_core::holdings::last_held_dates`). So a position bought last
//! week still gets its six weeks of history — what it did before you bought it
//! is what a chart of it shows — while one sold in September gets nothing after
//! the week it was sold, and one sold yesterday does not want today's close.
//!
//! Only a symbol with a "no" anywhere is fetched, with ONE Yahoo chart request
//! covering all N weeks plus a week of slack. The latest close is recorded,
//! and so is each missing week's last trading day (its Friday close, or
//! Thursday's when Friday was a holiday). Lines are appended through the same
//! pipeline the button writes through ([`prices_api::plan_update`] /
//! [`prices_api::apply_update`]): the journal write mutex, a pinned read, a
//! reparse-and-verify, and `line_exists` dedupe.
//!
//! # Why in the engine and not the SPA
//!
//! The preferences already live server-side ([`crate::ui_prefs`]), the write
//! pipeline is here, and a WebView that is hidden or throttled must not stop
//! the schedule. One `tokio` task per process ([`run_forever`]), started by the
//! two binaries that serve a journal (the desktop app and `--server`), never
//! by the integration tests, which drive single ticks through
//! `AppState::run_auto_prices_once` with a fixed clock.
//!
//! # Never hammer Yahoo
//!
//! - The hourly tick is a local check. A journal that already has what it
//!   wants makes no request at all, however often the tick runs.
//! - One request per symbol per run, whatever N is.
//! - [`FETCH_CONCURRENCY`] requests at once (the button uses 5), and request
//!   starts at least [`REQUEST_GAP`] apart.
//! - A `429` or `999` stops the run on the spot and suppresses auto runs for
//!   an exponential backoff ([`BACKOFF_FIRST`] doubling to [`BACKOFF_MAX`]).
//! - A memo of `(symbol, target close, weeks)` already asked about. Without
//!   it, a symbol whose latest candle is OLDER than the heuristic's target — an
//!   exchange holiday, a delisted ticker, a mutual fund that prices weekly —
//!   would look missing on every tick and be fetched hourly forever. With it,
//!   such a symbol is asked once per target date. In memory only: a relaunch
//!   asks once more, which is the right amount of retrying.
//!
//! # What it never does
//!
//! - Create `prices.journal`, or choose a file for a journal's FIRST price
//!   ([`prices_api::auto_target`]). Until some file holds a `P` line, the
//!   updater does nothing and says so; the setting's help text says to click
//!   Update prices once.
//! - Write to a read-only journal.
//! - Hold the journal write mutex across the network: unlike the button, it
//!   fetches FIRST and only then takes the lock, reads, merges and writes. A
//!   background task must never make a user's save wait on Yahoo. The read
//!   happens after the fetch, under the lock, so nothing is lost by it, and
//!   the dedupe runs against the bytes actually written over.
//! - Complain. A write that loses a race with an edit (`409`) is skipped
//!   quietly; the next tick tries again against the edited file.
//!
//! # "Latest completed close"
//!
//! The LOCAL check needs a date without asking anyone. It uses a US-centric
//! heuristic ([`latest_completed_close`]): today if it is a weekday and the
//! time is past 21:30 UTC (16:00 New York in both EST and EDT, plus margin),
//! else the most recent weekday before today. Being wrong costs little: too
//! early a target just means a fetch that brings back yesterday (and the memo
//! stops it repeating), and the fetch itself is exchange-aware — a candle for
//! a session that has not closed is dropped by `yahoo::parse_chart_closes`, so
//! an intraday price is never recorded as a close.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Json;
use futures::stream::{self, StreamExt};
use ledgeline_core::holdings::{HoldingsScope, ScopeMode, last_held_dates};
use ledgeline_core::model::Journal;
use ledgeline_core::reports::periods::{days_from_iso, iso_from_days, weekday_on_or_before};
use serde::Serialize;
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::AppState;
use crate::error::AppError;
use crate::prices_api::{self, FetchOutcome, Outcome, WirePriceSymbol};
use crate::reports_api::{compute, unix_now};
use crate::ui_prefs::{self, AutoPricePrefs};
use crate::yahoo::{FetchedPrice, YahooError};

/// How long after start-up the first check runs. Launch is busy — the journal
/// parse, the first page's reports — and a price check is never urgent enough
/// to compete with it.
const FIRST_RUN_DELAY: Duration = Duration::from_secs(30);

/// How often the check repeats, in wall-clock seconds.
const RUN_EVERY_SECONDS: i64 = 60 * 60;

/// How often the loop wakes to compare the wall clock with the next due time.
///
/// Why not simply sleep for an hour: `tokio`'s timers run on the monotonic
/// clock, which on macOS does not advance while the machine sleeps. A laptop
/// closed overnight would otherwise wake to a timer that still has most of an
/// hour to go. Waking every few minutes and asking the WALL clock costs one
/// comparison.
const WAKE_EVERY: Duration = Duration::from_secs(5 * 60);

/// Requests in flight at once. Lower than the button's 5: nobody is waiting on
/// this, so there is no reason to be anything but polite.
const FETCH_CONCURRENCY: usize = 2;

/// The least time between two request STARTS.
const REQUEST_GAP: Duration = Duration::from_millis(250);

/// The first backoff after a rate limit, in seconds; each further one in a row
/// doubles, up to [`BACKOFF_MAX`].
const BACKOFF_FIRST: i64 = 60 * 60;
/// See [`BACKOFF_FIRST`]. A day: by then whatever Yahoo objected to is long
/// past, and an updater that has gone quiet for longer than that looks broken.
const BACKOFF_MAX: i64 = 24 * 60 * 60;

/// 21:30 UTC, in seconds after midnight: the US close (16:00 ET is 20:00 UTC
/// in summer, 21:00 in winter) plus half an hour for Yahoo to settle the day's
/// candle.
const CLOSE_CUTOFF: i64 = 21 * 60 * 60 + 30 * 60;

const SECONDS_PER_DAY: i64 = 86_400;

// ===========================================================================
// The pure planner
// ===========================================================================

/// One security, as the local check sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeldSymbol {
    pub(crate) symbol: WirePriceSymbol,
    /// Every date a `P` directive prices this symbol on, in any file.
    pub(crate) priced_on: BTreeSet<String>,
    /// The last date it was held: today while the position is open, else the
    /// day it was sold out of. It needs a price dated D only when D is on or
    /// before this — see the module docs.
    pub(crate) last_held: String,
}

/// What the planner needs to know about the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalFacts {
    /// Whether this server may write at all.
    pub(crate) editable: bool,
    /// Where prices go ([`prices_api::auto_target`]); `None` when no file
    /// holds a `P` line yet.
    pub(crate) target: Option<String>,
    pub(crate) symbols: Vec<HeldSymbol>,
}

/// One Monday–Sunday week, as ISO dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Week {
    pub(crate) monday: String,
    pub(crate) sunday: String,
}

impl Week {
    fn holds(&self, date: &str) -> bool {
        self.monday.as_str() <= date && date <= self.sunday.as_str()
    }
}

/// What one symbol is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Want {
    pub(crate) held: HeldSymbol,
    /// It was held on or after the latest completed close, and has no price
    /// on or after it.
    pub(crate) latest: bool,
    /// The backfill weeks it was held on or after the Monday of, and has no
    /// price in at all.
    pub(crate) weeks: Vec<Week>,
}

/// A symbol already asked about for this target, so not asked again until the
/// target date moves (see the module docs). `weeks` is in the key so raising
/// the setting backfills straight away rather than tomorrow.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MemoKey {
    symbol: String,
    close: String,
    weeks: u32,
}

impl MemoKey {
    fn new(symbol: &str, close: &str, weeks: u32) -> Self {
        Self {
            symbol: symbol.to_string(),
            close: close.to_string(),
            weeks,
        }
    }
}

/// What one tick should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoPlan {
    /// The setting is off.
    Off,
    /// The journal cannot be written.
    ReadOnly,
    /// No file holds prices yet; the user has to choose one (by clicking
    /// Update prices once).
    NoPricesFile,
    /// Every held symbol has what it wants (or was already asked about).
    UpToDate,
    /// Something is missing, but Yahoo told us to slow down until `until`.
    BackingOff {
        until: i64,
    },
    Fetch(FetchPlan),
}

/// The fetches one tick makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchPlan {
    /// The file new lines go into, in date order.
    pub(crate) target: String,
    /// The latest completed close, by [`latest_completed_close`].
    pub(crate) close: String,
    /// The setting's week count, for the memo.
    pub(crate) weeks: u32,
    /// The first date each request asks for: the oldest backfill week's
    /// Monday, less a week of slack.
    pub(crate) from: String,
    pub(crate) wants: Vec<Want>,
}

/// Decide what a tick does at `now` (Unix seconds). Pure: the clock, the
/// journal, the settings, the memo and the backoff all come in as arguments,
/// so every scheduling rule is unit-tested with fixed clocks.
pub(crate) fn plan_auto(
    now: i64,
    facts: &JournalFacts,
    prefs: AutoPricePrefs,
    memo: &HashSet<MemoKey>,
    backoff_until: Option<i64>,
) -> AutoPlan {
    if !prefs.enabled {
        return AutoPlan::Off;
    }
    if !facts.editable {
        return AutoPlan::ReadOnly;
    }
    let Some(target) = facts.target.clone() else {
        return AutoPlan::NoPricesFile;
    };
    let close = latest_completed_close(now);
    let weeks = backfill_weeks(&close, prefs.weeks);
    let wants: Vec<Want> = facts
        .symbols
        .iter()
        .filter(|held| !memo.contains(&MemoKey::new(&held.symbol.symbol, &close, prefs.weeks)))
        .filter_map(|held| want_for(held, &close, &weeks))
        .collect();
    if wants.is_empty() {
        return AutoPlan::UpToDate;
    }
    // Checked only once something is actually missing: an up-to-date journal
    // says so even while backing off.
    if let Some(until) = backoff_until.filter(|until| now < *until) {
        return AutoPlan::BackingOff { until };
    }
    AutoPlan::Fetch(FetchPlan {
        target,
        from: fetch_from(&close, prefs.weeks),
        close,
        weeks: prefs.weeks,
        wants,
    })
}

/// The latest US close that has certainly happened by `now`: today after
/// 21:30 UTC on a weekday, else the weekday before today. See the module docs
/// for why a heuristic is enough here.
pub(crate) fn latest_completed_close(now: i64) -> String {
    let day = now.div_euclid(SECONDS_PER_DAY);
    let today = iso_from_days(day);
    let past_cutoff = now.rem_euclid(SECONDS_PER_DAY) >= CLOSE_CUTOFF;
    if past_cutoff && weekday_on_or_before(&today) == today {
        today
    } else {
        weekday_on_or_before(&iso_from_days(day - 1))
    }
}

/// The Monday of `date`'s week, as a day number.
fn monday_of(date: &str) -> i64 {
    let day = days_from_iso(date);
    // 1970-01-01 was a Thursday: +3 makes Monday 0.
    day - (day + 3).rem_euclid(7)
}

/// The `count` complete weeks before the one `close` falls in, newest first.
/// `close`'s own week is not one of them: its latest close is the other half
/// of the check.
fn backfill_weeks(close: &str, count: u32) -> Vec<Week> {
    let this_monday = monday_of(close);
    (1..=i64::from(count))
        .map(|back| {
            let monday = this_monday - 7 * back;
            Week {
                monday: iso_from_days(monday),
                sunday: iso_from_days(monday + 6),
            }
        })
        .collect()
}

/// The first date a fetch asks for: N weeks back from `close`'s week, plus a
/// week of slack so the oldest week is covered in full and `0` weeks still
/// spans a long weekend.
fn fetch_from(close: &str, weeks: u32) -> String {
    iso_from_days(monday_of(close) - 7 * i64::from(weeks) - 7)
}

/// What `held` is missing, if anything. A date is only "missing" for a symbol
/// held at some point between it and today: on or before `last_held`.
fn want_for(held: &HeldSymbol, close: &str, weeks: &[Week]) -> Option<Want> {
    let latest = held.last_held.as_str() >= close
        && held.priced_on.range(close.to_string()..).next().is_none();
    let weeks: Vec<Week> = weeks
        .iter()
        .filter(|week| held.last_held >= week.monday)
        .filter(|week| {
            held.priced_on
                .range(week.monday.clone()..=week.sunday.clone())
                .next()
                .is_none()
        })
        .cloned()
        .collect();
    (latest || !weeks.is_empty()).then(|| Want {
        held: held.clone(),
        latest,
        weeks,
    })
}

/// Out of one symbol's fetched closes (ascending), the ones to record: the
/// newest, when the latest close was wanted and the journal does not already
/// price that date, and the last close in each wanted week. Ascending, one per
/// date.
pub(crate) fn pick_prices(want: &Want, closes: &[FetchedPrice]) -> Vec<FetchedPrice> {
    let latest = closes
        .last()
        .filter(|_| want.latest)
        .filter(|price| !want.held.priced_on.contains(&price.date));
    let weekly = want
        .weeks
        .iter()
        .filter_map(|week| closes.iter().rev().find(|price| week.holds(&price.date)));
    latest
        .into_iter()
        .chain(weekly)
        .map(|price| (price.date.clone(), price.clone()))
        .collect::<BTreeMap<String, FetchedPrice>>()
        .into_values()
        .collect()
}

// ===========================================================================
// Per-process state: memo, backoff, and what the last run did
// ===========================================================================

/// How a run ended, for `GET /api/prices/status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RunOutcome {
    /// New `P` lines were written (`lastAdded` says how many).
    Updated,
    /// The local check found nothing missing; no request was made.
    UpToDate,
    /// Fetched, but nothing new came back (a holiday, a delisted ticker).
    NothingNew,
    /// No file holds prices yet, so nothing was done.
    NoPricesFile,
    /// Yahoo asked us to slow down; see `backoffUntil`.
    RateLimited,
    /// The file changed while the run was writing; the next run retries.
    Conflict,
    /// Some fetch or the write failed; the next run retries.
    Failed,
}

/// `GET /api/prices/status`'s `auto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAutoStatus {
    /// The effective `autoPrices` setting for this ledger.
    enabled: bool,
    /// When the last run that did anything finished, Unix seconds.
    last_run: Option<i64>,
    last_outcome: Option<RunOutcome>,
    /// How many `P` lines that run wrote.
    last_added: usize,
    /// Until when auto runs are suppressed after a rate limit, Unix seconds.
    backoff_until: Option<i64>,
}

/// A rate-limit backoff in force.
#[derive(Debug, Clone, Copy)]
struct Backoff {
    until: i64,
    /// The length of this backoff, so the next one can double it.
    step: i64,
}

#[derive(Debug, Default)]
struct RunState {
    /// The journal the memo and the last run are about; a different one (File
    /// → Open) starts both afresh.
    ledger: Option<String>,
    memo: HashSet<MemoKey>,
    backoff: Option<Backoff>,
    last_run: Option<i64>,
    last_outcome: Option<RunOutcome>,
    last_added: usize,
}

/// The updater's per-process state, shared by every `AppState` clone. A
/// `std::sync::Mutex`: it is only ever held for a few field reads and writes,
/// never across an `.await`.
#[derive(Debug, Default)]
pub(crate) struct AutoPrices {
    run: Mutex<RunState>,
    /// Wakes the loop early: the settings page turned the updater on, or
    /// changed its week count, and should not have to wait up to an hour.
    wake: Notify,
}

impl AutoPrices {
    fn run(&self) -> std::sync::MutexGuard<'_, RunState> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ask the loop to run now rather than at its next hourly slot.
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    /// The `auto` block of `GET /api/prices/status`.
    pub(crate) fn wire_status(&self, enabled: bool) -> WireAutoStatus {
        let run = self.run();
        WireAutoStatus {
            enabled,
            last_run: run.last_run,
            last_outcome: run.last_outcome,
            last_added: run.last_added,
            backoff_until: run.backoff.map(|backoff| backoff.until),
        }
    }

    /// [`plan_auto`] against this process's memo and backoff, first starting
    /// both afresh if `ledger` is not the journal they were about.
    fn plan(
        &self,
        now: i64,
        ledger: &str,
        facts: &JournalFacts,
        prefs: AutoPricePrefs,
    ) -> AutoPlan {
        let mut run = self.run();
        if run.ledger.as_deref() != Some(ledger) {
            // The backoff is about Yahoo, not the journal, so it stays.
            let backoff = run.backoff;
            *run = RunState {
                ledger: Some(ledger.to_string()),
                backoff,
                ..RunState::default()
            };
        }
        plan_auto(
            now,
            facts,
            prefs,
            &run.memo,
            run.backoff.map(|backoff| backoff.until),
        )
    }

    fn record(&self, now: i64, outcome: RunOutcome, added: usize) {
        let mut run = self.run();
        run.last_run = Some(now);
        run.last_outcome = Some(outcome);
        run.last_added = added;
    }

    fn remember(&self, keys: impl IntoIterator<Item = MemoKey>) {
        self.run().memo.extend(keys);
    }

    /// Yahoo rate-limited a run at `now`: back off, twice as long as last time
    /// in a row.
    fn back_off(&self, now: i64) {
        let mut run = self.run();
        let step = run
            .backoff
            .map_or(BACKOFF_FIRST, |backoff| (backoff.step * 2).min(BACKOFF_MAX));
        run.backoff = Some(Backoff {
            until: now + step,
            step,
        });
    }

    /// A run got through without being rate-limited: the next limit starts
    /// from [`BACKOFF_FIRST`] again.
    fn clear_backoff(&self) {
        self.run().backoff = None;
    }
}

// ===========================================================================
// One run
// ===========================================================================

/// One tick at `now` (Unix seconds): the local check, then — only if something
/// is missing — the fetch and the write. Never fails: every outcome is
/// recorded for the status route, and the next tick is the retry.
pub(crate) async fn run_once(state: &AppState, now: i64) {
    let reader = state.clone();
    let gathered = compute(move || {
        let prefs = ui_prefs::auto_price_prefs(&reader);
        // The journal is only walked when the setting is on.
        let facts = prefs
            .enabled
            .then(|| journal_facts(&reader, now))
            .transpose()?;
        Ok((prefs, facts))
    })
    .await;
    let Ok(Json((prefs, Some((ledger, facts))))) = gathered else {
        return;
    };

    let auto = state.auto_prices();
    let fetch = match auto.plan(now, &ledger, &facts, prefs) {
        AutoPlan::Off | AutoPlan::ReadOnly | AutoPlan::BackingOff { .. } => return,
        AutoPlan::NoPricesFile => return auto.record(now, RunOutcome::NoPricesFile, 0),
        AutoPlan::UpToDate => return auto.record(now, RunOutcome::UpToDate, 0),
        AutoPlan::Fetch(fetch) => fetch,
    };

    let fetched = fetch_all(state, &fetch).await;
    let rate_limited = fetched.rate_limited;
    if rate_limited {
        auto.back_off(now);
    } else {
        auto.clear_backoff();
    }

    let written = if fetched.outcomes.is_empty() {
        Ok(0)
    } else {
        write(state, fetch.target.clone(), fetched.outcomes).await
    };
    let (outcome, added) = match written {
        Ok(added) => {
            // Only once the closes are safely in the file: a symbol whose
            // write failed must be asked about again.
            auto.remember(fetched.asked);
            let outcome = match (rate_limited, added, fetched.failed) {
                (true, _, _) => RunOutcome::RateLimited,
                (false, 0, 0) => RunOutcome::NothingNew,
                (false, 0, _) => RunOutcome::Failed,
                (false, _, _) => RunOutcome::Updated,
            };
            (outcome, added)
        }
        // The user edited the file mid-run: quietly, the next tick retries.
        Err(AppError::Conflict(_)) => (RunOutcome::Conflict, 0),
        Err(error) => {
            eprintln!("ledgeline: the automatic price update could not be written: {error}");
            (RunOutcome::Failed, 0)
        }
    };
    auto.record(now, outcome, added);
}

/// The journal as the planner sees it, plus a name for which journal it is
/// (for [`AutoPrices::plan`]'s memo reset).
fn journal_facts(state: &AppState, now: i64) -> Result<(String, JournalFacts), AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let today = iso_from_days(now.div_euclid(SECONDS_PER_DAY));
    let target = snapshot
        .storage()
        .and_then(|location| prices_api::auto_target(&location, journal));
    // Unscoped, like the button: a price is a journal-wide fact.
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: today,
        gain_since: None,
        value_in: None,
    };
    let last_held = last_held_dates(&journal.transactions, &journal.accounts, &scope)?;
    Ok((
        journal.source_name.clone(),
        JournalFacts {
            editable: state.editing_enabled(),
            target,
            symbols: held_symbols(journal, last_held),
        },
    ))
}

/// Each security ever held, with its Yahoo ticker and the dates it is priced
/// on, in one pass over the prices.
fn held_symbols(journal: &Journal, last_held: BTreeMap<String, String>) -> Vec<HeldSymbol> {
    let mut priced_on: HashMap<&str, BTreeSet<String>> =
        journal
            .prices
            .iter()
            .fold(HashMap::new(), |mut dates, price| {
                if last_held.contains_key(&price.commodity.0) {
                    dates
                        .entry(price.commodity.0.as_str())
                        .or_insert_with(BTreeSet::new)
                        .insert(price.date.clone());
                }
                dates
            });
    last_held
        .into_iter()
        .map(|(symbol, last_held)| HeldSymbol {
            priced_on: priced_on.remove(symbol.as_str()).unwrap_or_default(),
            symbol: WirePriceSymbol {
                yahoo_ticker: prices_api::yahoo_ticker(&journal.commodity_tags, &symbol),
                symbol,
            },
            last_held,
        })
        .collect()
}

/// What the fetch phase brought back.
struct Fetched {
    /// The closes to record, per symbol that has any.
    outcomes: Vec<FetchOutcome>,
    /// Every symbol Yahoo answered for, to memo once the write lands.
    asked: Vec<MemoKey>,
    /// How many requests failed for a reason other than a rate limit.
    failed: usize,
    rate_limited: bool,
}

/// Spaces request STARTS at least `gap` apart, across however many are in
/// flight. The lock is held only to claim a slot, never across the sleep.
struct Pacer {
    next: Mutex<Option<Instant>>,
    gap: Duration,
}

impl Pacer {
    fn new(gap: Duration) -> Self {
        Self {
            next: Mutex::new(None),
            gap,
        }
    }

    async fn wait(&self) {
        let slot = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            let slot = next.map_or_else(Instant::now, |at| at.max(Instant::now()));
            *next = Some(slot + self.gap);
            slot
        };
        tokio::time::sleep_until(slot).await;
    }
}

/// What Yahoo said about one symbol's history.
type Answer = Result<Vec<FetchedPrice>, YahooError>;

/// One request per wanted symbol, [`FETCH_CONCURRENCY`] at a time and
/// [`REQUEST_GAP`] apart, stopping at the first rate limit: requests not yet
/// started are never sent.
async fn fetch_all(state: &AppState, plan: &FetchPlan) -> Fetched {
    // Everything the request futures touch is owned (or `Arc`-shared), not
    // borrowed from this frame: the loop runs inside a spawned task, and a
    // `buffer_unordered` of futures borrowing locals does not prove `Send`
    // for every lifetime the spawn needs.
    let source = Arc::clone(state.price_source());
    let pacer = Arc::new(Pacer::new(REQUEST_GAP));
    let stop = Arc::new(AtomicBool::new(false));
    let from: Arc<str> = Arc::from(plan.from.as_str());
    let answers: Vec<(Want, Option<Answer>)> = stream::iter(plan.wants.clone())
        .map(|want| {
            let (source, pacer, stop, from) = (
                Arc::clone(&source),
                Arc::clone(&pacer),
                Arc::clone(&stop),
                Arc::clone(&from),
            );
            async move {
                if stop.load(Ordering::Relaxed) {
                    return (want, None);
                }
                pacer.wait().await;
                if stop.load(Ordering::Relaxed) {
                    return (want, None);
                }
                let result = source
                    .daily_closes(&want.held.symbol.yahoo_ticker, &from)
                    .await;
                if matches!(result, Err(YahooError::RateLimited(_))) {
                    stop.store(true, Ordering::Relaxed);
                }
                (want, Some(result))
            }
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await;

    let mut fetched = Fetched {
        outcomes: Vec::new(),
        asked: Vec::new(),
        failed: 0,
        rate_limited: stop.load(Ordering::Relaxed),
    };
    for (want, answer) in answers {
        match answer {
            // Never sent, or the rate limit itself: neither is about the symbol.
            None | Some(Err(YahooError::RateLimited(_))) => {}
            Some(Err(_)) => fetched.failed += 1,
            Some(Ok(closes)) => {
                fetched.asked.push(MemoKey::new(
                    &want.held.symbol.symbol,
                    &plan.close,
                    plan.weeks,
                ));
                let picked = pick_prices(&want, &closes);
                if !picked.is_empty() {
                    fetched.outcomes.push(FetchOutcome {
                        symbol: want.held.symbol.clone(),
                        result: Ok(picked),
                    });
                }
            }
        }
    }
    fetched
}

/// Append `outcomes` to `target` through the button's own pipeline, under the
/// journal write mutex, and count the lines written.
async fn write(
    state: &AppState,
    target: String,
    outcomes: Vec<FetchOutcome>,
) -> Result<usize, AppError> {
    let _write = state.import_writes().lock().await;
    let writer = state.clone();
    let Json(added) = compute(move || {
        let plan = prices_api::plan_update(&writer, &target)?;
        let response = prices_api::apply_update(&writer, plan, outcomes)?;
        Ok(response
            .results
            .iter()
            .filter(|result| result.outcome == Outcome::Updated)
            .count())
    })
    .await?;
    Ok(added)
}

// ===========================================================================
// The loop
// ===========================================================================

/// The background task: a first run [`FIRST_RUN_DELAY`] after start-up, then
/// one roughly every [`RUN_EVERY_SECONDS`] of wall-clock time, or straight
/// away when [`AutoPrices::wake`] is called. Runs until the runtime shuts
/// down. Each run re-reads the setting, so turning it off takes effect at the
/// next tick with nothing to cancel.
pub(crate) async fn run_forever(state: AppState) {
    let auto = state.auto_prices_arc();
    tokio::select! {
        () = tokio::time::sleep(FIRST_RUN_DELAY) => {}
        () = auto.wake.notified() => {}
    }
    loop {
        let started = now_seconds();
        run_once(&state, started).await;
        let due = started + RUN_EVERY_SECONDS;
        loop {
            tokio::select! {
                () = tokio::time::sleep(WAKE_EVERY) => {}
                () = auto.wake.notified() => break,
            }
            if now_seconds() >= due {
                break;
            }
        }
    }
}

/// The wall clock, Unix seconds.
fn now_seconds() -> i64 {
    i64::try_from(unix_now()).unwrap_or(i64::MAX)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::Dec;

    /// Unix seconds for `date` at `hh:mm` UTC.
    fn at(date: &str, hh: i64, mm: i64) -> i64 {
        days_from_iso(date) * SECONDS_PER_DAY + hh * 3_600 + mm * 60
    }

    /// A symbol priced on `priced_on`, last held on `last_held`.
    fn held(symbol: &str, priced_on: &[&str], last_held: &str) -> HeldSymbol {
        HeldSymbol {
            symbol: WirePriceSymbol {
                symbol: symbol.to_string(),
                yahoo_ticker: symbol.to_string(),
            },
            priced_on: priced_on.iter().map(|d| (*d).to_string()).collect(),
            last_held: last_held.to_string(),
        }
    }

    /// Still held today (2026-11-13).
    const OPEN: &str = "2026-11-13";

    fn facts(symbols: Vec<HeldSymbol>) -> JournalFacts {
        JournalFacts {
            editable: true,
            target: Some("prices.journal".to_string()),
            symbols,
        }
    }

    const ON: AutoPricePrefs = AutoPricePrefs {
        enabled: true,
        weeks: 6,
    };

    fn fetch_plan(plan: AutoPlan) -> FetchPlan {
        match plan {
            AutoPlan::Fetch(fetch) => fetch,
            other => panic!("expected a fetch, got {other:?}"),
        }
    }

    fn price(date: &str, value: &str) -> FetchedPrice {
        FetchedPrice {
            date: date.to_string(),
            quantity: Dec::parse(value, '.').unwrap(),
        }
    }

    // --- the latest completed close -------------------------------------

    /// 2026-11-13 is a Friday. Before 21:30 UTC its close is not yet
    /// certain, so the target is Thursday; from 21:30 on it is Friday.
    #[test]
    fn the_cutoff_is_2130_utc_on_a_weekday() {
        assert_eq!(
            latest_completed_close(at("2026-11-13", 21, 29)),
            "2026-11-12"
        );
        assert_eq!(
            latest_completed_close(at("2026-11-13", 21, 30)),
            "2026-11-13"
        );
        assert_eq!(
            latest_completed_close(at("2026-11-13", 21, 31)),
            "2026-11-13"
        );
        assert_eq!(latest_completed_close(at("2026-11-13", 0, 5)), "2026-11-12");
    }

    /// All weekend, and Monday before the cutoff, the target is Friday.
    #[test]
    fn weekends_and_monday_morning_target_friday() {
        for now in [
            at("2026-11-14", 12, 0),
            at("2026-11-15", 23, 59),
            at("2026-11-16", 14, 0),
        ] {
            assert_eq!(latest_completed_close(now), "2026-11-13", "{now}");
        }
        assert_eq!(
            latest_completed_close(at("2026-11-16", 22, 0)),
            "2026-11-16"
        );
    }

    // --- the weeks -------------------------------------------------------

    #[test]
    fn backfill_weeks_are_the_complete_weeks_before_the_closes_week() {
        // Friday 2026-11-13's week starts Monday 2026-11-09.
        assert_eq!(
            backfill_weeks("2026-11-13", 2),
            vec![
                Week {
                    monday: "2026-11-02".to_string(),
                    sunday: "2026-11-08".to_string()
                },
                Week {
                    monday: "2026-10-26".to_string(),
                    sunday: "2026-11-01".to_string()
                },
            ]
        );
        assert!(backfill_weeks("2026-11-13", 0).is_empty());
        // A Monday is its own week's Monday.
        assert_eq!(backfill_weeks("2026-11-09", 1)[0].monday, "2026-11-02");
    }

    #[test]
    fn the_fetch_window_is_n_weeks_plus_seven_days() {
        assert_eq!(fetch_from("2026-11-13", 6), "2026-09-21");
        assert_eq!(fetch_from("2026-11-13", 0), "2026-11-02");
    }

    // --- the planner -----------------------------------------------------

    /// Off, read-only and no-prices-file all stop before anything else.
    #[test]
    fn nothing_runs_when_off_read_only_or_without_a_prices_file() {
        let now = at("2026-11-13", 22, 0);
        let symbols = vec![held("AAPL", &[], OPEN)];
        let memo = HashSet::new();
        let off = AutoPricePrefs {
            enabled: false,
            weeks: 6,
        };
        assert_eq!(
            plan_auto(now, &facts(symbols.clone()), off, &memo, None),
            AutoPlan::Off
        );
        let read_only = JournalFacts {
            editable: false,
            ..facts(symbols.clone())
        };
        assert_eq!(
            plan_auto(now, &read_only, ON, &memo, None),
            AutoPlan::ReadOnly
        );
        let no_file = JournalFacts {
            target: None,
            ..facts(symbols)
        };
        assert_eq!(
            plan_auto(now, &no_file, ON, &memo, None),
            AutoPlan::NoPricesFile
        );
    }

    /// The target close already priced, and every week covered by SOME `P`
    /// (a Wednesday counts as much as a Friday): no request at all.
    #[test]
    fn a_covered_journal_makes_no_request() {
        let now = at("2026-11-13", 22, 0);
        let symbols = vec![held(
            "AAPL",
            &["2026-11-13", "2026-11-04", "2026-10-30"],
            OPEN,
        )];
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 2,
        };
        assert_eq!(
            plan_auto(now, &facts(symbols), prefs, &HashSet::new(), None),
            AutoPlan::UpToDate
        );
    }

    /// 21:29 vs 21:31 on a Friday with Thursday priced: before the cutoff
    /// nothing is missing; after it, Friday's close is.
    #[test]
    fn the_cutoff_decides_whether_todays_close_is_missing() {
        let symbols = vec![held("AAPL", &["2026-11-12"], OPEN)];
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 0,
        };
        let memo = HashSet::new();
        assert_eq!(
            plan_auto(
                at("2026-11-13", 21, 29),
                &facts(symbols.clone()),
                prefs,
                &memo,
                None
            ),
            AutoPlan::UpToDate
        );
        let fetch = fetch_plan(plan_auto(
            at("2026-11-13", 21, 31),
            &facts(symbols),
            prefs,
            &memo,
            None,
        ));
        assert_eq!(fetch.close, "2026-11-13");
        assert_eq!(fetch.wants.len(), 1);
        assert!(fetch.wants[0].latest);
        assert!(fetch.wants[0].weeks.is_empty(), "N = 0 backfills nothing");
    }

    /// Only the uncovered weeks are wanted.
    #[test]
    fn only_uncovered_weeks_are_wanted() {
        let now = at("2026-11-13", 22, 0);
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 3,
        };
        // Priced on the close and mid-week 2026-11-04.
        let symbols = vec![held("AAPL", &["2026-11-13", "2026-11-04"], OPEN)];
        let fetch = fetch_plan(plan_auto(
            now,
            &facts(symbols),
            prefs,
            &HashSet::new(),
            None,
        ));
        let want = &fetch.wants[0];
        assert!(!want.latest);
        // Weeks of 11-02 (covered), 10-26 and 10-19.
        assert_eq!(mondays(want), ["2026-10-26", "2026-10-19"]);
        assert_eq!(fetch.from, "2026-10-12");
    }

    fn mondays(want: &Want) -> Vec<&str> {
        want.weeks.iter().map(|w| w.monday.as_str()).collect()
    }

    /// Bought on 1 September (well inside the window) and still held: every
    /// backfill week is wanted, the ones BEFORE the purchase included, and so
    /// is the latest close.
    #[test]
    fn a_position_still_held_wants_every_week_even_before_it_was_bought() {
        let now = at("2026-09-18", 22, 0);
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 4,
        };
        let symbols = vec![held("X", &[], "2026-09-18")];
        let fetch = fetch_plan(plan_auto(
            now,
            &facts(symbols),
            prefs,
            &HashSet::new(),
            None,
        ));
        let want = &fetch.wants[0];
        assert!(want.latest);
        assert_eq!(
            mondays(want),
            ["2026-09-07", "2026-08-31", "2026-08-24", "2026-08-17"]
        );
    }

    /// Sold out on Saturday 5 September: weeks up to and including that
    /// week are wanted, nothing after, and not the latest close.
    #[test]
    fn a_sold_position_wants_weeks_up_to_its_sale_and_nothing_after() {
        let now = at("2026-09-25", 22, 0);
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 4,
        };
        let symbols = vec![held("Y", &[], "2026-09-05")];
        let fetch = fetch_plan(plan_auto(
            now,
            &facts(symbols),
            prefs,
            &HashSet::new(),
            None,
        ));
        let want = &fetch.wants[0];
        assert!(!want.latest);
        // Backfill weeks: 09-14, 09-07, 08-31, 08-24. The sale's week is
        // 08-31; 09-07 and 09-14 begin after it.
        assert_eq!(mondays(want), ["2026-08-31", "2026-08-24"]);
    }

    /// Sold yesterday, with nothing else missing: today's close is not
    /// wanted, so no request is made at all.
    #[test]
    fn a_position_sold_yesterday_does_not_want_todays_close() {
        let now = at("2026-11-13", 22, 0);
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 0,
        };
        let symbols = vec![held("X", &["2026-11-11"], "2026-11-12")];
        assert_eq!(
            plan_auto(now, &facts(symbols.clone()), prefs, &HashSet::new(), None),
            AutoPlan::UpToDate
        );
        // Before the cutoff the target is yesterday, when it was still held.
        let before = at("2026-11-13", 12, 0);
        assert!(
            fetch_plan(plan_auto(
                before,
                &facts(symbols),
                prefs,
                &HashSet::new(),
                None
            ))
            .wants[0]
                .latest
        );
    }

    /// A symbol already asked about for this target (a holiday answered with
    /// an older candle) is not asked again — until the target moves.
    #[test]
    fn the_memo_stops_a_repeat_until_the_target_moves() {
        let symbols = vec![held("AAPL", &["2026-11-12"], OPEN)];
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 0,
        };
        let friday = at("2026-11-13", 22, 0);
        let memo = HashSet::from([MemoKey::new("AAPL", "2026-11-13", 0)]);
        assert_eq!(
            plan_auto(friday, &facts(symbols.clone()), prefs, &memo, None),
            AutoPlan::UpToDate
        );
        // The same memo on Monday evening, still held: the target is now 11-16.
        let monday = at("2026-11-16", 22, 0);
        let still_held = vec![held("AAPL", &["2026-11-12"], "2026-11-16")];
        assert!(matches!(
            plan_auto(monday, &facts(still_held), prefs, &memo, None),
            AutoPlan::Fetch(_)
        ));
        // And raising the week count is a different question.
        let more = AutoPricePrefs {
            enabled: true,
            weeks: 1,
        };
        assert!(matches!(
            plan_auto(friday, &facts(symbols), more, &memo, None),
            AutoPlan::Fetch(_)
        ));
    }

    /// A backoff suppresses fetching until it expires, but an up-to-date
    /// journal still says so.
    #[test]
    fn a_backoff_suppresses_fetches_until_it_expires() {
        let now = at("2026-11-13", 22, 0);
        let missing = vec![held("AAPL", &[], OPEN)];
        let memo = HashSet::new();
        assert_eq!(
            plan_auto(now, &facts(missing.clone()), ON, &memo, Some(now + 60)),
            AutoPlan::BackingOff { until: now + 60 }
        );
        assert!(matches!(
            plan_auto(now, &facts(missing), ON, &memo, Some(now)),
            AutoPlan::Fetch(_)
        ));
        let covered = vec![held("AAPL", &["2026-11-13"], OPEN)];
        let latest_only = AutoPricePrefs {
            enabled: true,
            weeks: 0,
        };
        assert_eq!(
            plan_auto(now, &facts(covered), latest_only, &memo, Some(now + 60)),
            AutoPlan::UpToDate
        );
    }

    // --- picking the closes ----------------------------------------------

    /// The newest close for the latest, and each wanted week's LAST trading
    /// day — Thursday when that Friday was a holiday.
    #[test]
    fn picks_the_newest_close_and_each_weeks_last_trading_day() {
        let want = Want {
            held: held("AAPL", &[], OPEN),
            latest: true,
            weeks: vec![
                Week {
                    monday: "2026-11-02".to_string(),
                    sunday: "2026-11-08".to_string(),
                },
                Week {
                    monday: "2026-10-26".to_string(),
                    sunday: "2026-11-01".to_string(),
                },
            ],
        };
        let closes = vec![
            price("2026-10-29", "9"),
            price("2026-10-30", "10"),
            price("2026-11-05", "11"),
            // 11-06, a Friday, missing: a holiday.
            price("2026-11-12", "12"),
            price("2026-11-13", "13"),
        ];
        assert_eq!(
            pick_prices(&want, &closes),
            vec![
                price("2026-10-30", "10"),
                price("2026-11-05", "11"),
                price("2026-11-13", "13"),
            ]
        );
    }

    /// A latest close the journal already has is not picked again, and a
    /// week with no candles at all (delisted) picks nothing.
    #[test]
    fn picks_nothing_already_priced_or_absent() {
        let want = Want {
            held: held("AAPL", &["2026-11-12"], OPEN),
            latest: true,
            weeks: vec![Week {
                monday: "2026-11-02".to_string(),
                sunday: "2026-11-08".to_string(),
            }],
        };
        assert!(pick_prices(&want, &[price("2026-11-12", "12")]).is_empty());
        assert!(pick_prices(&want, &[]).is_empty());
    }

    // --- the backoff -----------------------------------------------------

    #[test]
    fn the_backoff_doubles_to_a_day_and_clears_on_success() {
        let auto = AutoPrices::default();
        let steps: Vec<i64> = (0..7)
            .map(|_| {
                auto.back_off(0);
                auto.run().backoff.map(|b| b.until).unwrap()
            })
            .collect();
        assert_eq!(
            steps,
            [3_600, 7_200, 14_400, 28_800, 57_600, 86_400, 86_400]
        );
        auto.clear_backoff();
        auto.back_off(100);
        assert_eq!(auto.run().backoff.map(|b| b.until), Some(3_700));
    }

    /// Another journal (File → Open) starts the memo and the last run afresh,
    /// but keeps a backoff — that one is about Yahoo.
    #[test]
    fn a_new_ledger_resets_the_memo_but_not_the_backoff() {
        let auto = AutoPrices::default();
        let now = at("2026-11-13", 22, 0);
        let prefs = AutoPricePrefs {
            enabled: true,
            weeks: 0,
        };
        let symbols = facts(vec![held("AAPL", &["2026-11-12"], OPEN)]);
        assert!(matches!(
            auto.plan(now, "a.journal", &symbols, prefs),
            AutoPlan::Fetch(_)
        ));
        auto.remember([MemoKey::new("AAPL", "2026-11-13", 0)]);
        auto.record(now, RunOutcome::NothingNew, 0);
        assert_eq!(
            auto.plan(now, "a.journal", &symbols, prefs),
            AutoPlan::UpToDate
        );
        auto.back_off(now);
        assert!(matches!(
            auto.plan(now, "b.journal", &symbols, prefs),
            AutoPlan::BackingOff { .. }
        ));
        assert_eq!(auto.wire_status(true).last_outcome, None);
        assert!(auto.wire_status(true).backoff_until.is_some());
    }
}
