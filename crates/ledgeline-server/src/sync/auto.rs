//! When to sync (plan 43 §3.7): the pure planner [`plan_sync`], and the
//! background loop [`run_forever`], copying the shape of `auto_prices.rs`.
//!
//! # "Most recent end of day"
//!
//! MX, SimpleFIN Bridge's data provider, refreshes about once a day,
//! overnight. So a connection is due when its last success is before today's
//! 07:00 local — before that, the newest complete day is yesterday's, which
//! the last sync already had — or when it is `autoSyncEveryHours` old.
//! Fetching more often gains nothing and spends the ~24-a-day quota.
//!
//! # The quota
//!
//! Counted per connection per UTC day, in `sync-state.json` so a relaunch
//! can't reset it: auto runs stop at [`AUTO_MAX`]; Sync now warns at
//! [`MANUAL_WARN`] and refuses at [`MANUAL_MAX`]. A quota warning from the
//! server pauses auto runs until the next UTC midnight.
//!
//! # The loop
//!
//! A first check [`FIRST_CHECK`] after launch plus up to [`JITTER_MAX`] of
//! jitter (so a fleet of laptops waking at 07:00 don't all ask at once), then
//! a [`WAKE_EVERY`] wall-clock wake, which survives a laptop's sleep where a
//! monotonic timer would not (plan 41's reasoning). [`SyncAuto::wake`] cuts a
//! wait short when the sync settings change.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use ledgeline_core::reports::periods::{days_from_iso, iso_from_days};
use tokio::sync::Notify;

/// Auto runs stop at this many requests in a UTC day.
pub(crate) const AUTO_MAX: u32 = 6;
/// Sync now warns from this many.
pub(crate) const MANUAL_WARN: u32 = 12;
/// Sync now refuses at this many.
pub(crate) const MANUAL_MAX: u32 = 16;

/// The longest window one request may ask for (SimpleFIN silently caps
/// longer ones and adds a `gen.api` error).
pub(crate) const CHUNK_DAYS: i64 = 90;

/// How far back each run re-reads past the last success: the Bridge filters
/// by day, on a date it doesn't name, so an overlap catches late rows.
pub(crate) const OVERLAP_DAYS: i64 = 5;

/// The local hour after which today's overnight refresh is assumed done.
const REFRESH_HOUR: i64 = 7;

/// First check after launch, before jitter.
pub(crate) const FIRST_CHECK: Duration = Duration::from_secs(45);
/// The most jitter added to the first check.
pub(crate) const JITTER_MAX: u64 = 600;
/// The wall-clock wake.
pub(crate) const WAKE_EVERY: Duration = Duration::from_secs(5 * 60);

/// Network backoff: first step, doubling, capped.
const BACKOFF_FIRST: i64 = 15 * 60;
const BACKOFF_MAX: i64 = 6 * 60 * 60;

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;

/// Who asked for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The background schedule.
    Auto,
    /// Sync now, I've fixed it, the first sync after linking.
    Manual,
}

/// The sync settings, effective for this ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncPrefs {
    pub(crate) auto_sync: bool,
    /// Already clamped to `3..=24`.
    pub(crate) every_hours: u32,
}

/// Why a connection can't sync without the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NeedsUser {
    /// No keychain item on this machine.
    NotConnected,
    /// The last request was refused with 403: a new token is needed.
    Revoked,
    /// 402. A manual run still tries: renewing fixes it with no new token.
    Payment,
}

/// One linked account, as the planner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkFacts {
    pub(crate) key: String,
    pub(crate) remote_id: String,
    /// ISO.
    pub(crate) start_date: String,
    pub(crate) last_success: Option<i64>,
}

/// One connection, as the planner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnFacts {
    pub(crate) id: String,
    /// Enabled links with valid paths.
    pub(crate) links: Vec<LinkFacts>,
    pub(crate) last_success: Option<i64>,
    pub(crate) requests_today: u32,
    pub(crate) quota_until: Option<i64>,
    pub(crate) needs_user: Option<NeedsUser>,
}

/// A date window, epoch seconds, `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) start: i64,
    pub(crate) end: i64,
}

/// What one connection should do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SyncPlan {
    /// Auto-sync is off (auto runs only).
    Off,
    /// The journal can't be written.
    ReadOnly,
    /// Nothing linked and enabled.
    NoLinks,
    /// Not due before `next`.
    NotDue { next: i64 },
    /// Network trouble or a quota pause; not before `until`.
    BackingOff { until: i64 },
    /// The day's request budget is spent (`count` so far).
    OverQuota { count: u32, until: i64 },
    /// Can't continue without the user.
    NeedsUser(NeedsUser),
    /// Fetch these windows, oldest first, for these accounts.
    Fetch {
        windows: Vec<Window>,
        accounts: Vec<String>,
    },
}

/// The next UTC midnight after `now`.
pub(crate) fn next_utc_midnight(now: i64) -> i64 {
    (now.div_euclid(DAY) + 1) * DAY
}

/// The UTC date of `now`, ISO — the quota's day.
pub(crate) fn utc_day(now: i64) -> String {
    iso_from_days(now.div_euclid(DAY))
}

/// The local date of `now` at `offset` seconds east of UTC, ISO.
pub(crate) fn local_day(now: i64, offset: i32) -> String {
    iso_from_days((now + i64::from(offset)).div_euclid(DAY))
}

/// The epoch of local midnight starting `iso`.
fn local_midnight(iso: &str, offset: i32) -> i64 {
    days_from_iso(iso) * DAY - i64::from(offset)
}

/// Today's 07:00 local, as an epoch.
fn refresh_today(now: i64, offset: i32) -> i64 {
    local_midnight(&local_day(now, offset), offset) + REFRESH_HOUR * HOUR
}

/// When a connection whose last success was `last` is next due.
fn due_at(last: i64, now: i64, offset: i32, every_hours: u32) -> i64 {
    let by_interval = last + i64::from(every_hours) * HOUR;
    let refresh = refresh_today(now, offset);
    // The first 07:00 after the last success.
    let next_refresh = if last < refresh {
        refresh
    } else {
        refresh + DAY
    };
    by_interval.min(next_refresh)
}

/// One account's window start: its start date, or the last success less the
/// overlap, whichever is later; and a day earlier still, because the Bridge
/// is vague about which date it filters on (rows before the start date are
/// dropped locally anyway).
fn link_start(link: &LinkFacts, offset: i32) -> i64 {
    let from_date = local_midnight(&link.start_date, offset) - DAY;
    link.last_success
        .map_or(from_date, |last| from_date.max(last - OVERLAP_DAYS * DAY))
}

/// `[start, end)` cut into windows of at most [`CHUNK_DAYS`], oldest first.
pub(crate) fn chunks(start: i64, end: i64) -> Vec<Window> {
    let step = CHUNK_DAYS * DAY;
    let mut windows = Vec::new();
    let mut at = start;
    while at < end {
        let next = (at + step).min(end);
        windows.push(Window {
            start: at,
            end: next,
        });
        at = next;
    }
    windows
}

/// Decide what `conn` does at `now`. Pure: the clock, the offset, the
/// settings and the backoff all come in, so every rule is tested with fixed
/// clocks.
///
/// Auto runs honour, in order: the setting, a read-only journal, links,
/// the user's attention, the quota and its pause, the network backoff, and
/// the due rule. A manual run skips the setting, the backoff and the due rule,
/// and has its own, higher quota ceiling.
pub(crate) fn plan_sync(
    now: i64,
    offset: i32,
    mode: Mode,
    editable: bool,
    prefs: SyncPrefs,
    conn: &ConnFacts,
    backoff_until: Option<i64>,
) -> SyncPlan {
    let auto = mode == Mode::Auto;
    if auto && !prefs.auto_sync {
        return SyncPlan::Off;
    }
    if !editable {
        return SyncPlan::ReadOnly;
    }
    if conn.links.is_empty() {
        return SyncPlan::NoLinks;
    }
    match &conn.needs_user {
        Some(NeedsUser::Payment) if !auto => {}
        Some(reason) => return SyncPlan::NeedsUser(reason.clone()),
        None => {}
    }
    let midnight = next_utc_midnight(now);
    let ceiling = if auto { AUTO_MAX } else { MANUAL_MAX };
    if conn.requests_today >= ceiling {
        return SyncPlan::OverQuota {
            count: conn.requests_today,
            until: midnight,
        };
    }
    if auto {
        if let Some(until) = conn.quota_until.filter(|until| now < *until) {
            return SyncPlan::BackingOff { until };
        }
        if let Some(until) = backoff_until.filter(|until| now < *until) {
            return SyncPlan::BackingOff { until };
        }
        if let Some(last) = conn.last_success {
            let next = due_at(last, now, offset, prefs.every_hours);
            if now < next {
                return SyncPlan::NotDue { next };
            }
        }
    }
    let start = conn
        .links
        .iter()
        .map(|link| link_start(link, offset))
        .min()
        .unwrap_or(now);
    let end = local_midnight(&local_day(now, offset), offset) + DAY;
    SyncPlan::Fetch {
        windows: chunks(start, end.max(start + 1)),
        accounts: conn
            .links
            .iter()
            .map(|link| link.remote_id.clone())
            .collect(),
    }
}

/// The first check's delay for a random `seed`: [`FIRST_CHECK`] plus
/// `seed % (JITTER_MAX + 1)` seconds.
pub(crate) fn first_delay(seed: u64) -> Duration {
    FIRST_CHECK + Duration::from_secs(seed % (JITTER_MAX + 1))
}

// ---------------------------------------------------------------------------
// Per-process state: backoff, last run, the wake
// ---------------------------------------------------------------------------

/// A backoff in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Backoff {
    pub(crate) until: i64,
    step: i64,
}

#[derive(Debug, Default)]
struct RunState {
    /// By connection id. In memory only, like plan 41's.
    backoff: HashMap<String, Backoff>,
    last_run: Option<i64>,
}

/// The scheduler's per-process state, shared by every `AppState` clone.
#[derive(Debug, Default)]
pub(crate) struct SyncAuto {
    run: Mutex<RunState>,
    pub(crate) wake: Notify,
}

impl SyncAuto {
    fn run(&self) -> std::sync::MutexGuard<'_, RunState> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ask the loop to look now (sync settings changed).
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    /// The backoff for `connection`, if one is in force at `now`.
    pub(crate) fn backoff_until(&self, connection: &str, now: i64) -> Option<i64> {
        self.run()
            .backoff
            .get(connection)
            .map(|backoff| backoff.until)
            .filter(|until| now < *until)
    }

    /// A transient failure at `now`: back off, twice as long as last time.
    pub(crate) fn back_off(&self, connection: &str, now: i64) {
        let mut run = self.run();
        let step = run
            .backoff
            .get(connection)
            .map_or(BACKOFF_FIRST, |backoff| (backoff.step * 2).min(BACKOFF_MAX));
        run.backoff.insert(
            connection.to_string(),
            Backoff {
                until: now + step,
                step,
            },
        );
    }

    /// A request got through: the next failure starts from the first step.
    pub(crate) fn clear_backoff(&self, connection: &str) {
        self.run().backoff.remove(connection);
    }

    pub(crate) fn record_run(&self, now: i64) {
        self.run().last_run = Some(now);
    }

    pub(crate) fn last_run(&self) -> Option<i64> {
        self.run().last_run
    }
}

/// The background task: a jittered first check, then a run every
/// [`WAKE_EVERY`] of wall-clock time (each a cheap local check unless
/// something is due), or straight away on [`SyncAuto::wake`]. Each run
/// re-reads the settings, so turning auto-sync off needs nothing cancelled.
pub(crate) async fn run_forever(state: crate::AppState) {
    let auto = state.sync().auto_arc();
    let mut seed = [0u8; 8];
    let seed = getrandom::fill(&mut seed).map_or(0, |()| u64::from_le_bytes(seed));
    tokio::select! {
        () = tokio::time::sleep(first_delay(seed)) => {}
        () = auto.wake.notified() => {}
    }
    loop {
        super::engine::run_auto(&state, super::now_seconds()).await;
        tokio::select! {
            () = tokio::time::sleep(WAKE_EVERY) => {}
            () = auto.wake.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTC-7 (Denver in summer), like the web suite's pinned zone.
    const DENVER: i32 = -7 * 3_600;

    /// `date` at `hh:mm` local Denver time, as an epoch.
    fn at(date: &str, hh: i64, mm: i64) -> i64 {
        local_midnight(date, DENVER) + hh * HOUR + mm * 60
    }

    fn prefs(auto_sync: bool, every_hours: u32) -> SyncPrefs {
        SyncPrefs {
            auto_sync,
            every_hours,
        }
    }

    fn conn(last_success: Option<i64>) -> ConnFacts {
        ConnFacts {
            id: "c1".into(),
            links: vec![LinkFacts {
                key: "k1".into(),
                remote_id: "ACT-1".into(),
                start_date: "2026-11-01".into(),
                last_success,
            }],
            last_success,
            requests_today: 0,
            quota_until: None,
            needs_user: None,
        }
    }

    fn plan(now: i64, conn: &ConnFacts) -> SyncPlan {
        plan_sync(now, DENVER, Mode::Auto, true, prefs(true, 6), conn, None)
    }

    #[test]
    fn seven_am_local_decides_whether_last_night_is_new() {
        // Synced yesterday at 22:00; today 06:59 is not due (and 6 h haven't
        // passed... they have: 8 h 59 m). Use a 24-hour interval to isolate
        // the 07:00 rule.
        let last = at("2026-11-14", 22, 0);
        let c = conn(Some(last));
        let daily = |now| plan_sync(now, DENVER, Mode::Auto, true, prefs(true, 24), &c, None);
        assert_eq!(
            daily(at("2026-11-15", 6, 59)),
            SyncPlan::NotDue {
                next: at("2026-11-15", 7, 0)
            }
        );
        assert!(matches!(
            daily(at("2026-11-15", 7, 1)),
            SyncPlan::Fetch { .. }
        ));
    }

    #[test]
    fn the_interval_makes_a_connection_due_too() {
        let last = at("2026-11-15", 8, 0);
        let c = conn(Some(last));
        assert_eq!(
            plan(at("2026-11-15", 13, 59), &c),
            SyncPlan::NotDue {
                next: at("2026-11-15", 14, 0)
            }
        );
        assert!(matches!(
            plan(at("2026-11-15", 14, 0), &c),
            SyncPlan::Fetch { .. }
        ));
    }

    #[test]
    fn a_long_sleep_is_due_on_waking() {
        // Closed the laptop for three days: the first wake is due, whatever
        // the monotonic clock thinks.
        let c = conn(Some(at("2026-11-12", 9, 0)));
        assert!(matches!(
            plan(at("2026-11-15", 9, 0), &c),
            SyncPlan::Fetch { .. }
        ));
    }

    #[test]
    fn never_synced_is_due_at_once() {
        assert!(matches!(
            plan(at("2026-11-15", 3, 0), &conn(None)),
            SyncPlan::Fetch { .. }
        ));
    }

    #[test]
    fn off_read_only_and_no_links_never_fetch() {
        let now = at("2026-11-15", 9, 0);
        let c = conn(None);
        assert_eq!(
            plan_sync(now, DENVER, Mode::Auto, true, prefs(false, 6), &c, None),
            SyncPlan::Off
        );
        // Sync now ignores the setting.
        assert!(matches!(
            plan_sync(now, DENVER, Mode::Manual, true, prefs(false, 6), &c, None),
            SyncPlan::Fetch { .. }
        ));
        assert_eq!(
            plan_sync(now, DENVER, Mode::Manual, false, prefs(true, 6), &c, None),
            SyncPlan::ReadOnly
        );
        let empty = ConnFacts {
            links: Vec::new(),
            ..c
        };
        assert_eq!(plan(now, &empty), SyncPlan::NoLinks);
    }

    #[test]
    fn needs_user_short_circuits_before_any_request() {
        let now = at("2026-11-15", 9, 0);
        for reason in [NeedsUser::NotConnected, NeedsUser::Revoked] {
            let c = ConnFacts {
                needs_user: Some(reason.clone()),
                ..conn(None)
            };
            assert_eq!(plan(now, &c), SyncPlan::NeedsUser(reason.clone()));
            assert_eq!(
                plan_sync(now, DENVER, Mode::Manual, true, prefs(true, 6), &c, None),
                SyncPlan::NeedsUser(reason)
            );
        }
        // A lapsed payment stops auto runs, but Sync now may try once.
        let c = ConnFacts {
            needs_user: Some(NeedsUser::Payment),
            ..conn(None)
        };
        assert_eq!(plan(now, &c), SyncPlan::NeedsUser(NeedsUser::Payment));
        assert!(matches!(
            plan_sync(now, DENVER, Mode::Manual, true, prefs(true, 6), &c, None),
            SyncPlan::Fetch { .. }
        ));
    }

    #[test]
    fn the_quota_stops_auto_at_six_and_manual_at_sixteen() {
        let now = at("2026-11-15", 9, 0);
        let midnight = next_utc_midnight(now);
        let c = ConnFacts {
            requests_today: 6,
            ..conn(None)
        };
        assert_eq!(
            plan(now, &c),
            SyncPlan::OverQuota {
                count: 6,
                until: midnight
            }
        );
        assert!(matches!(
            plan_sync(now, DENVER, Mode::Manual, true, prefs(true, 6), &c, None),
            SyncPlan::Fetch { .. }
        ));
        let spent = ConnFacts {
            requests_today: 16,
            ..conn(None)
        };
        assert_eq!(
            plan_sync(
                now,
                DENVER,
                Mode::Manual,
                true,
                prefs(true, 6),
                &spent,
                None
            ),
            SyncPlan::OverQuota {
                count: 16,
                until: midnight
            }
        );
    }

    #[test]
    fn the_quota_survives_a_relaunch_through_the_state_file() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join(super::super::state::FILE_NAME);
        let now = at("2026-11-15", 9, 0);
        let mut state = super::super::state::load(Some(&path));
        let entry = state.connections.entry("c1".into()).or_default();
        for _ in 0..6 {
            entry.count_request(&utc_day(now));
        }
        super::super::state::save(Some(&path), &state);
        // "Relaunch": read it back fresh.
        let reloaded = super::super::state::load(Some(&path));
        let c = ConnFacts {
            requests_today: reloaded.connections["c1"].requests_on(&utc_day(now)),
            ..conn(None)
        };
        assert!(matches!(
            plan(now, &c),
            SyncPlan::OverQuota { count: 6, .. }
        ));
        // A new UTC day starts the count again.
        let tomorrow = next_utc_midnight(now) + 60;
        let c = ConnFacts {
            requests_today: reloaded.connections["c1"].requests_on(&utc_day(tomorrow)),
            ..conn(None)
        };
        assert!(matches!(plan(tomorrow, &c), SyncPlan::Fetch { .. }));
    }

    #[test]
    fn a_quota_warning_pauses_auto_until_utc_midnight() {
        let now = at("2026-11-15", 9, 0);
        let until = next_utc_midnight(now);
        let c = ConnFacts {
            quota_until: Some(until),
            ..conn(None)
        };
        assert_eq!(plan(now, &c), SyncPlan::BackingOff { until });
        assert!(matches!(plan(until + 1, &c), SyncPlan::Fetch { .. }));
    }

    #[test]
    fn the_backoff_doubles_to_six_hours_and_clears() {
        let auto = SyncAuto::default();
        let now = 1_000;
        let mut steps = Vec::new();
        for _ in 0..7 {
            auto.back_off("c1", now);
            steps.push(auto.backoff_until("c1", now).expect("backing off") - now);
        }
        assert_eq!(steps, [900, 1_800, 3_600, 7_200, 14_400, 21_600, 21_600]);
        let c = conn(None);
        assert_eq!(
            plan_sync(
                now,
                DENVER,
                Mode::Auto,
                true,
                prefs(true, 6),
                &c,
                Some(now + 60)
            ),
            SyncPlan::BackingOff { until: now + 60 }
        );
        // Sync now isn't held back by it.
        assert!(matches!(
            plan_sync(
                now,
                DENVER,
                Mode::Manual,
                true,
                prefs(true, 6),
                &c,
                Some(now + 60)
            ),
            SyncPlan::Fetch { .. }
        ));
        auto.clear_backoff("c1");
        assert_eq!(auto.backoff_until("c1", now), None);
        auto.back_off("c1", now);
        assert_eq!(auto.backoff_until("c1", now), Some(now + 900));
    }

    #[test]
    fn a_long_first_sync_is_cut_into_90_day_chunks_oldest_first() {
        let now = at("2026-11-15", 9, 0);
        let c = ConnFacts {
            links: vec![LinkFacts {
                key: "k1".into(),
                remote_id: "ACT-1".into(),
                start_date: "2026-03-01".into(),
                last_success: None,
            }],
            ..conn(None)
        };
        let SyncPlan::Fetch { windows, accounts } = plan(now, &c) else {
            panic!("fetch");
        };
        assert_eq!(accounts, ["ACT-1"]);
        assert_eq!(windows.len(), 3, "{windows:?}");
        assert_eq!(windows[0].start, local_midnight("2026-03-01", DENVER) - DAY);
        assert!(windows.windows(2).all(|pair| pair[0].end == pair[1].start));
        assert!(windows.iter().all(|w| w.end - w.start <= CHUNK_DAYS * DAY));
        // Ends at tomorrow's local midnight: all of today is asked for.
        assert_eq!(
            windows.last().expect("last").end,
            local_midnight("2026-11-16", DENVER)
        );
    }

    #[test]
    fn later_runs_overlap_the_last_success_by_five_days() {
        let last = at("2026-11-14", 8, 0);
        let c = conn(Some(last));
        let SyncPlan::Fetch { windows, .. } = plan(at("2026-11-15", 9, 0), &c) else {
            panic!("fetch");
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].start, last - OVERLAP_DAYS * DAY);
        // The start date wins when it is later.
        let fresh = ConnFacts {
            links: vec![LinkFacts {
                start_date: "2026-11-13".into(),
                ..c.links[0].clone()
            }],
            ..c
        };
        let SyncPlan::Fetch { windows, .. } = plan(at("2026-11-15", 9, 0), &fresh) else {
            panic!("fetch");
        };
        assert_eq!(windows[0].start, local_midnight("2026-11-13", DENVER) - DAY);
    }

    #[test]
    fn the_first_check_is_45s_plus_at_most_ten_minutes() {
        assert_eq!(first_delay(0), Duration::from_secs(45));
        assert_eq!(first_delay(600), Duration::from_secs(645));
        assert_eq!(first_delay(601), Duration::from_secs(45));
        for seed in [7_u64, 1 << 40, u64::MAX] {
            let delay = first_delay(seed);
            assert!(delay >= FIRST_CHECK && delay <= FIRST_CHECK + Duration::from_secs(JITTER_MAX));
        }
    }

    #[test]
    fn utc_and_local_days() {
        let now = at("2026-11-15", 20, 0); // 03:00 UTC on the 16th
        assert_eq!(local_day(now, DENVER), "2026-11-15");
        assert_eq!(utc_day(now), "2026-11-16");
    }
}
