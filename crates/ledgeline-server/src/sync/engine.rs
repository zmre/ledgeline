//! One sync run (plan 43 §5): plan → fetch → normalize → split → stage →
//! dry run → commit, per linked account, plus the two held-row actions and
//! the diagnostics report.
//!
//! # The import is the import screen's
//!
//! A synced account's rows become a CSV under
//! `<syncSourceDir>/<connection>/<account>/<YYYY-MM-DDTHHMM>.csv` and go
//! through `import_api`'s own stage, `run_dry_run` and `run_commit` — the
//! headless path `ledgeline import` takes — so a synced row and the same CSV
//! imported by hand land identically, and `hledger import` from a terminal
//! agrees with both.
//!
//! # What is filtered before staging
//!
//! Rows the journal already holds by id are dropped silently (a hand edit
//! beats a re-download; sync never reports `conflicting`), and possible
//! duplicates are **held** in `sync-state.json`. Only the rest is staged, so
//! the commit can't write a held or known row.
//!
//! hledger's `.latest.<csv>` marker is deleted after each commit: every fetch
//! is a new file, ids and the start date do the dedup, and a marker would hide
//! a row that arrived late.
//!
//! # Never noisy, never stuck
//!
//! A run never fails as a whole. Every outcome is recorded per account; an
//! edit that lands mid-run makes the commit a conflict, which is recorded as
//! `skipped` and simply retried by the next run (the window overlaps).

use std::collections::{BTreeMap, HashMap, HashSet};

use ledgeline_core::Dec;
use ledgeline_core::edit::EditOp;
use ledgeline_core::feeds::config::{AccountLink, ConnectionLink, SyncConfig};
use ledgeline_core::feeds::dupes::{self, Held};
use ledgeline_core::feeds::normalize::{
    FeedColumns, NormRow, NormalizeOptions, epoch_to_date, fetched_ids, normalize, to_csv,
};
use ledgeline_core::feeds::simplefin::AccessUrl;
use ledgeline_core::feeds::{FeedAccount, FeedBatch, FeedCurrency, ProblemKind};
use ledgeline_core::model::{Amount, Journal, Tindex};
use ledgeline_core::reports::periods::{days_from_iso, iso_from_days};
use ledgeline_core::rules::RulesDoc;
use ledgeline_core::vault::VaultPath;
use serde::Deserialize;

use super::auto::{self, ConnFacts, LinkFacts, Mode, NeedsUser, SyncPlan, SyncPrefs};
use super::client::{AccountsQuery, FeedError};
use super::state::{AccountState, ConnState, RemoteSummary, SyncState};
use super::wire::{
    Outcome, ProblemCode, Severity, WireBankBalance, WireHistory, WireProblem, WireRunAccount,
    WireSyncRun, manage_url,
};
use super::{SyncReply, access_for, link, read_config, read_state, slug, update_state};
use crate::AppState;
use crate::error::AppError;
use crate::import_api::{self, HeadlessImport, HeadlessPreview};
use crate::ui_prefs::{self, SyncSettings};

const DAY: i64 = 86_400;

/// Days without new data before an account says so.
const STALE_DAYS: i64 = 14;

/// The fallback account a rules file without `account2` uses.
pub(crate) const DEFAULT_FALLBACK: &str = "expenses:unknown";

/// What to run.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) mode: Mode,
    /// One connection, or all.
    pub(crate) connection: Option<String>,
    /// These account keys only; empty = every linked account.
    pub(crate) accounts: Vec<String>,
}

/// Clears the session's `running` flag however a run ends.
struct Running<'a>(&'a super::SyncSession);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.set_running(false);
    }
}

/// A background tick: a run in [`Mode::Auto`] when the setting is on.
pub(crate) async fn run_auto(state: &AppState, now: i64) {
    let reader = state.clone();
    let Ok(settings) = super::blocking(move || Ok(ui_prefs::sync_settings(&reader))).await else {
        return;
    };
    if !settings.auto_sync {
        return;
    }
    let _ = run(
        state,
        now,
        &Request {
            mode: Mode::Auto,
            connection: None,
            accounts: Vec::new(),
        },
    )
    .await;
}

/// One run at `now`. Never fails: every outcome is in the answer and in
/// `sync-state.json`.
pub(crate) async fn run(state: &AppState, now: i64, request: &Request) -> WireSyncRun {
    let session = state.sync();
    let _one_at_a_time = session.run_lock().lock().await;
    session.set_running(true);
    let _running = Running(session);

    let reader = state.clone();
    let gathered = super::blocking(move || {
        let (config, _) = read_config(&reader)?;
        Ok((config, ui_prefs::sync_settings(&reader)))
    })
    .await;
    let (config, settings) = match gathered {
        Ok(gathered) => gathered,
        Err(error) => {
            return WireSyncRun {
                ok: false,
                requests_today: 0,
                quota_warning: false,
                accounts: Vec::new(),
                problems: vec![WireProblem::setup(error.to_string())],
            };
        }
    };

    let wanted =
        |link: &&AccountLink| request.accounts.is_empty() || request.accounts.contains(&link.key());
    let mut answer = WireSyncRun {
        ok: true,
        requests_today: 0,
        quota_warning: false,
        accounts: Vec::new(),
        problems: Vec::new(),
    };
    // Links whose connection this file doesn't list can't run anywhere.
    for link in config.accounts.iter().filter(wanted) {
        if config.connection(&link.connection).is_none()
            && request
                .connection
                .as_ref()
                .is_none_or(|id| *id == link.connection)
        {
            answer.accounts.push(failed_account(
                link,
                Outcome::Skipped,
                WireProblem::not_connected(&link.connection),
            ));
        }
    }
    for connection in &config.connections {
        if request
            .connection
            .as_ref()
            .is_some_and(|id| *id != connection.id)
        {
            continue;
        }
        let links: Vec<AccountLink> = config
            .accounts
            .iter()
            .filter(|link| link.connection == connection.id)
            .filter(wanted)
            .cloned()
            .collect();
        if links.is_empty() && !request.accounts.is_empty() {
            continue;
        }
        let targeted = !request.accounts.is_empty();
        let ran = run_connection(
            state,
            now,
            request.mode,
            connection,
            links,
            targeted,
            &settings,
        )
        .await;
        answer.requests_today += ran.requests_today;
        answer.quota_warning |= ran.quota_warning;
        answer.accounts.extend(ran.accounts);
        answer.problems.extend(ran.problems);
    }
    answer.ok = answer
        .accounts
        .iter()
        .all(|account| !matches!(account.outcome, Outcome::Failed))
        && answer
            .problems
            .iter()
            .all(|problem| problem.severity != Severity::Error);
    session.auto().record_run(now);
    answer
}

/// What one connection's part of a run did.
struct ConnRun {
    accounts: Vec<WireRunAccount>,
    problems: Vec<WireProblem>,
    requests_today: u32,
    quota_warning: bool,
}

fn failed_account(link: &AccountLink, outcome: Outcome, problem: WireProblem) -> WireRunAccount {
    WireRunAccount {
        key: link.key(),
        label: link.label.clone(),
        added: 0,
        held: 0,
        already: 0,
        uncategorized: 0,
        outcome,
        problem: Some(problem),
    }
}

/// A link that passed every check, with what its import needs.
#[derive(Debug, Clone)]
pub(crate) struct ReadyLink {
    pub(crate) link: AccountLink,
    pub(crate) key: String,
    /// The CSV header, from the rules file's own `fields` line.
    pub(crate) columns: FeedColumns,
    /// The rules file's `account2`: where an uncategorized row lands.
    pub(crate) fallback: String,
}

/// Check one link's paths inside the journal's folder (`ledgeline.toml` is
/// untrusted) and read what its import needs.
///
/// # Errors
/// A `setup` problem for a rules file that doesn't resolve, a `target`
/// problem for a journal file that isn't a writable part of this journal.
pub(crate) fn ready(ctx: &link::Context, link: &AccountLink) -> Result<ReadyLink, WireProblem> {
    let key = link.key();
    let discovery = ctx.discovery.as_ref().ok_or_else(|| {
        WireProblem::setup("No journal file is open, so there is nowhere to sync into.")
    })?;
    let rules = import_api::resolve_rules(discovery, &link.rules)
        .ok()
        .and(discovery.resolve(&link.rules))
        .ok_or_else(|| {
            WireProblem::setup(format!(
                "The rules file {:?} for {} isn't inside the journal's folder, or doesn't exist.",
                link.rules, link.label
            ))
        })?;
    let target_ok = ctx
        .targets
        .iter()
        .any(|target| target.id == link.target && target.writable);
    if !target_ok {
        return Err(WireProblem::target(&link.target, &key));
    }
    let text = rules.path().read_text().unwrap_or_default();
    let columns = columns_of(&text);
    Ok(ReadyLink {
        link: link.clone(),
        key,
        columns,
        fallback: rules
            .account2
            .clone()
            .unwrap_or_else(|| DEFAULT_FALLBACK.to_string()),
    })
}

/// The sync CSV's header names as the rules file's `fields` line spells them
/// (a shared file borrows the manual file's names), or the defaults.
pub(crate) fn columns_of(rules_text: &str) -> FeedColumns {
    let fields = RulesDoc::parse(rules_text)
        .settings()
        .fields
        .map(|setting| setting.value)
        .unwrap_or_default();
    match fields.as_slice() {
        [date, amount, payee, description, memo, id, transacted] => FeedColumns {
            date: date.clone(),
            amount: amount.clone(),
            payee: payee.clone(),
            description: description.clone(),
            memo: memo.clone(),
            id: id.clone(),
            transacted: transacted.clone(),
        },
        _ => FeedColumns::default(),
    }
}

/// Whether a rules file's text reads the sync CSV: seven fields, and the
/// sixth (the id) written as the transaction's `id:` tag.
pub(crate) fn fits_sync(rules_text: &str) -> bool {
    let fields = RulesDoc::parse(rules_text)
        .settings()
        .fields
        .map(|setting| setting.value)
        .unwrap_or_default();
    fields.len() == 7 && rules_text.contains(&format!("id:%{}", fields[5]))
}

/// The planner's view of `connection`, from the config and the state file.
pub(crate) fn conn_facts(
    connection: &str,
    links: &[ReadyLink],
    memory: &SyncState,
    now: i64,
    connected: bool,
) -> ConnFacts {
    let remembered = memory.connections.get(connection);
    let needs_user = if connected {
        remembered.and_then(|c| {
            c.problems.iter().find_map(|problem| match problem.code {
                ProblemCode::Revoked => Some(NeedsUser::Revoked),
                ProblemCode::Payment => Some(NeedsUser::Payment),
                _ => None,
            })
        })
    } else {
        Some(NeedsUser::NotConnected)
    };
    ConnFacts {
        id: connection.to_string(),
        links: links
            .iter()
            .filter(|ready| ready.link.enabled)
            .map(|ready| LinkFacts {
                key: ready.key.clone(),
                remote_id: ready.link.remote_id.clone(),
                start_date: ready.link.start_date.clone(),
                last_success: memory.accounts.get(&ready.key).and_then(|a| a.last_success),
            })
            .collect(),
        last_success: remembered.and_then(|c| c.last_success),
        requests_today: remembered.map_or(0, |c| c.requests_on(&auto::utc_day(now))),
        quota_until: remembered.and_then(|c| c.quota_until),
        needs_user,
    }
}

/// When the background schedule next has something to do, for the status
/// line: the soonest of every connection's next due time.
pub(crate) fn next_auto(
    state: &AppState,
    now: i64,
    config: &SyncConfig,
    memory: &SyncState,
    settings: &SyncSettings,
) -> Option<i64> {
    let snapshot = state.snapshot();
    let ctx = link::Context::of(state, &snapshot);
    let prefs = SyncPrefs {
        auto_sync: settings.auto_sync,
        every_hours: settings.every_hours,
    };
    config
        .connections
        .iter()
        .filter_map(|connection| {
            let links: Vec<ReadyLink> = config
                .accounts
                .iter()
                .filter(|link| link.connection == connection.id)
                .filter_map(|link| ready(&ctx, link).ok())
                .collect();
            let facts = conn_facts(&connection.id, &links, memory, now, true);
            let backoff = state.sync().auto().backoff_until(&connection.id, now);
            match auto::plan_sync(
                now,
                super::local_offset(now),
                Mode::Auto,
                state.editing_enabled(),
                prefs,
                &facts,
                backoff,
            ) {
                SyncPlan::Fetch { .. } => Some(now),
                SyncPlan::NotDue { next } => Some(next),
                SyncPlan::BackingOff { until } | SyncPlan::OverQuota { until, .. } => Some(until),
                _ => None,
            }
        })
        .min()
}

/// One account's result, before it is recorded.
#[derive(Debug, Default)]
struct AccountRun {
    outcome: Option<Outcome>,
    added: usize,
    held: Vec<Held>,
    already: usize,
    uncategorized: usize,
    problem: Option<WireProblem>,
    data_through: Option<String>,
}

/// The `[start, end)` dates of a run's fetch, the accounts asked for, and
/// what came back, for one connection.
async fn run_connection(
    state: &AppState,
    now: i64,
    mode: Mode,
    connection: &ConnectionLink,
    links: Vec<AccountLink>,
    targeted: bool,
    settings: &SyncSettings,
) -> ConnRun {
    let offset = super::local_offset(now);
    let mut out = ConnRun {
        accounts: Vec::new(),
        problems: Vec::new(),
        requests_today: 0,
        quota_warning: false,
    };

    // Local checks first: paths, the keychain, the plan. No network.
    let reader = state.clone();
    let conn_id = connection.id.clone();
    let checked = super::blocking(move || {
        let snapshot = reader.snapshot();
        let ctx = link::Context::of(&reader, &snapshot);
        let (ready_links, broken): (Vec<_>, Vec<_>) = links
            .into_iter()
            .filter(|link| link.enabled)
            .map(|link| ready(&ctx, &link).map_err(|problem| (link, problem)))
            .partition(Result::is_ok);
        let ready_links: Vec<ReadyLink> = ready_links.into_iter().flatten().collect();
        let broken: Vec<(AccountLink, WireProblem)> =
            broken.into_iter().filter_map(Result::err).collect();
        let access = access_for(&reader, &conn_id);
        let memory = read_state(&reader);
        Ok((ready_links, broken, access, memory))
    })
    .await;
    let Ok((ready_links, broken, access, memory)) = checked else {
        return out;
    };
    for (link, problem) in broken {
        let key = link.key();
        let recorded = problem.clone();
        update_state(state, |memory| {
            memory.accounts.entry(key).or_default().problems = vec![recorded];
        });
        out.accounts
            .push(failed_account(&link, Outcome::Failed, problem));
    }
    let access = match access {
        Ok(access) => access,
        Err(problem) => {
            out.problems.push(problem);
            return out;
        }
    };
    let facts = conn_facts(&connection.id, &ready_links, &memory, now, access.is_some());
    out.requests_today = facts.requests_today;
    let prefs = SyncPrefs {
        auto_sync: settings.auto_sync,
        every_hours: settings.every_hours,
    };
    let backoff = state.sync().auto().backoff_until(&connection.id, now);
    let plan = auto::plan_sync(
        now,
        offset,
        mode,
        state.editing_enabled(),
        prefs,
        &facts,
        backoff,
    );
    let (windows, accounts) = match plan {
        SyncPlan::Fetch { windows, accounts } => (windows, accounts),
        SyncPlan::NeedsUser(reason) => {
            if mode == Mode::Manual {
                let problem = match reason {
                    NeedsUser::NotConnected => WireProblem::not_connected(&connection.id),
                    NeedsUser::Revoked => WireProblem::revoked(&connection.id),
                    NeedsUser::Payment => {
                        WireProblem::payment(&manage_for(&memory, &connection.id))
                    }
                };
                for ready in &ready_links {
                    out.accounts.push(failed_account(
                        &ready.link,
                        Outcome::Skipped,
                        problem.clone(),
                    ));
                }
                out.problems.push(problem);
            }
            return out;
        }
        SyncPlan::OverQuota { count, .. } => {
            if mode == Mode::Manual {
                out.problems.push(WireProblem::quota_refused(count));
                out.quota_warning = true;
            }
            return out;
        }
        SyncPlan::Off
        | SyncPlan::ReadOnly
        | SyncPlan::NoLinks
        | SyncPlan::NotDue { .. }
        | SyncPlan::BackingOff { .. } => return out,
    };
    let Some(access) = access else {
        return out;
    };

    // The network, outside every lock.
    let host = access.host.clone();
    let fetched = fetch_windows(
        state,
        now,
        &connection.id,
        &access,
        &windows,
        if targeted { accounts } else { Vec::new() },
    )
    .await;
    out.requests_today = fetched.requests_today;
    out.quota_warning = fetched.requests_today >= auto::MANUAL_WARN;
    let batch = match fetched.result {
        Ok(batch) => {
            state.sync().auto().clear_backoff(&connection.id);
            batch
        }
        Err(error) => {
            if error.is_transient() {
                state.sync().auto().back_off(&connection.id, now);
            }
            let problem = update_state(state, |memory| {
                let conn = memory.connections.entry(connection.id.clone()).or_default();
                conn.last_attempt = Some(now);
                let since = if error.is_transient() {
                    *conn.failing_since.get_or_insert(now)
                } else {
                    now
                };
                let problem = feed_error_problem(&error, &connection.id, &host, since, now);
                conn.problems = vec![problem.clone()];
                conn.log(now, problem.code.anchor(), &error.to_string());
                for ready in &ready_links {
                    memory
                        .accounts
                        .entry(ready.key.clone())
                        .or_default()
                        .record(WireHistory {
                            at: now,
                            outcome: Outcome::Failed,
                            added: 0,
                            held: 0,
                        });
                }
                problem
            });
            for ready in &ready_links {
                out.accounts.push(failed_account(
                    &ready.link,
                    Outcome::Failed,
                    problem.clone(),
                ));
            }
            out.problems.push(problem);
            return out;
        }
    };

    // What the provider said about the connection and its accounts.
    let conn_problems = connection_problems(&batch, &connection.id, &host, now);
    out.quota_warning |= conn_problems
        .iter()
        .any(|problem| problem.code == ProblemCode::Quota);
    let manage = manage_url(Some(&host));
    let remote = remote_summaries(&batch);
    update_state(state, |memory| {
        let conn = memory.connections.entry(connection.id.clone()).or_default();
        conn.last_attempt = Some(now);
        conn.last_success = Some(now);
        conn.failing_since = None;
        conn.host = Some(host.clone());
        conn.problems = conn_problems.clone();
        if conn_problems.iter().any(|p| p.code == ProblemCode::Quota) {
            conn.quota_until = Some(auto::next_utc_midnight(now));
        }
        conn.last_errlist = errlist(&batch);
        conn.log(now, "ok", &format!("{} account(s)", batch.accounts.len()));
        if !targeted {
            // A custom currency's fetched symbol outlives the listing.
            let symbols: HashMap<String, String> = conn
                .remote
                .iter()
                .filter(|old| old.currency_url.is_some())
                .map(|old| (old.id.clone(), old.currency.clone()))
                .collect();
            conn.remote = remote
                .iter()
                .cloned()
                .map(|mut summary| {
                    if let Some(symbol) = symbols.get(&summary.id)
                        && summary.currency_url.is_some()
                    {
                        summary.currency = symbol.clone();
                    }
                    summary
                })
                .collect();
        }
    });
    out.problems.extend(conn_problems);

    // Each account: split and import under the import lock, then the
    // balance check.
    let account_problems = account_problems(&batch, &manage, now);
    for ready in &ready_links {
        let feed_account = batch.accounts.iter().find(|a| a.id == ready.link.remote_id);
        let mut problems: Vec<WireProblem> = account_problems
            .get(&ready.link.remote_id)
            .cloned()
            .unwrap_or_default();
        let run = match feed_account {
            Some(account) => {
                import_account(
                    state,
                    now,
                    offset,
                    ready,
                    account,
                    &connection.name,
                    settings,
                )
                .await
            }
            None => AccountRun {
                outcome: Some(Outcome::Failed),
                problem: Some(WireProblem::other(
                    "SimpleFIN didn't return this account. It may have been removed at the bank.",
                    Severity::Warning,
                )),
                ..AccountRun::default()
            },
        };
        let held_count = run.held.len();
        let outcome = run.outcome.unwrap_or(if run.added > 0 {
            Outcome::Added
        } else {
            Outcome::NothingNew
        });
        problems.extend(run.problem.clone());
        let balance = feed_account.and_then(|account| balance_check(state, ready, account, offset));
        let key = ready.key.clone();
        let fetched_ids_known: HashSet<String> = {
            let snapshot = state.snapshot();
            dupes::known_ids(&snapshot.raw)
        };
        let recorded = update_state(state, |memory| {
            let account = memory.accounts.entry(key.clone()).or_default();
            account.hold(run.held.clone(), &fetched_ids_known);
            account.record(WireHistory {
                at: now,
                outcome,
                added: run.added,
                held: held_count,
            });
            if matches!(outcome, Outcome::Added | Outcome::NothingNew) {
                account.last_success = Some(now);
                account.last_added = run.added;
                if run.added > 0 {
                    account.last_new_data = Some(now);
                }
            }
            if let Some(through) = &run.data_through
                && account
                    .data_through
                    .as_ref()
                    .is_none_or(|old| old < through)
            {
                account.data_through = Some(through.clone());
            }
            let failing = problems.iter().any(|p| p.code == ProblemCode::ActFailed);
            account.failing_since = if failing {
                Some(account.failing_since.unwrap_or(now))
            } else {
                None
            };
            // `act.failed` turns red once it has lasted three days.
            if let Some(since) = account.failing_since {
                for problem in problems
                    .iter_mut()
                    .filter(|p| p.code == ProblemCode::ActFailed)
                {
                    let detail = problem.detail.clone().unwrap_or_default();
                    *problem = WireProblem::act_failed((now - since) / DAY, &manage)
                        .since(Some(since))
                        .detail(detail);
                }
            }
            if let Some((bank, ledger, diff)) = &balance {
                account.bank_balance = Some(bank.clone());
                account.ledger_balance = ledger.clone();
                account.balance_diff = diff.clone();
            }
            let stored: Vec<WireProblem> = problems
                .clone()
                .into_iter()
                .map(|problem| {
                    if matches!(problem.code, ProblemCode::ConAuth | ProblemCode::ActFailed) {
                        problem.data_through(account.data_through.clone())
                    } else {
                        problem
                    }
                })
                .collect();
            let mut stored = stored;
            if !account.held.is_empty() {
                stored.push(WireProblem::held(account.held.len(), &key));
            }
            if let (Some(diff), Some(bank)) = (&account.balance_diff, &account.bank_balance) {
                stored.push(WireProblem::balance(diff, &bank.date, &key));
            }
            if stored.is_empty()
                && let Some(through) = &account.data_through
                && days_from_iso(&super::auto::local_day(now, offset)) - days_from_iso(through)
                    > STALE_DAYS
            {
                stored.push(WireProblem::stale(through, &manage));
            }
            account.problems = stored;
            account.problems.clone()
        });
        out.accounts.push(WireRunAccount {
            key: ready.key.clone(),
            label: ready.link.label.clone(),
            added: run.added,
            held: held_count,
            already: run.already,
            uncategorized: run.uncategorized,
            outcome,
            problem: recorded
                .into_iter()
                .filter(|p| p.severity != Severity::Info)
                .max_by_key(|p| p.severity)
                .or(run.problem),
        });
    }
    out
}

/// The connection's manage URL, from its remembered host.
fn manage_for(memory: &SyncState, connection: &str) -> String {
    manage_url(
        memory
            .connections
            .get(connection)
            .and_then(|c| c.host.as_deref()),
    )
}

/// What a fetch of several windows brought back.
struct Fetched {
    result: Result<FeedBatch, FeedError>,
    requests_today: u32,
}

/// Fetch `windows` in order, counting each request against the day's quota
/// before it is sent, and merge them. Stops at the first failure.
async fn fetch_windows(
    state: &AppState,
    now: i64,
    connection: &str,
    access: &AccessUrl,
    windows: &[auto::Window],
    accounts: Vec<String>,
) -> Fetched {
    let mut merged: Option<FeedBatch> = None;
    let mut requests_today = 0;
    for window in windows {
        let query = AccountsQuery {
            start_date: Some(window.start),
            end_date: Some(window.end),
            balances_only: false,
            accounts: accounts.clone(),
        };
        requests_today = count_request(state, connection, now);
        match state.sync().provider().accounts(access, &query).await {
            Ok(batch) => {
                merged = Some(match merged {
                    None => batch,
                    Some(earlier) => merge(earlier, batch),
                });
            }
            Err(error) => {
                return Fetched {
                    result: Err(error),
                    requests_today,
                };
            }
        }
    }
    Fetched {
        result: Ok(merged.unwrap_or_default()),
        requests_today,
    }
}

/// Count one request for `connection` on `now`'s UTC day, persisted before
/// it is sent; answer the day's count.
pub(crate) fn count_request(state: &AppState, connection: &str, now: i64) -> u32 {
    update_state(state, |memory| {
        let conn = memory
            .connections
            .entry(connection.to_string())
            .or_default();
        conn.count_request(&auto::utc_day(now));
        conn.requests_on(&auto::utc_day(now))
    })
}

/// A balances-only fetch for the wizard and the "re-check accounts" menu:
/// one request, counted, remembered for the link preview.
///
/// # Errors
/// The problem to show.
pub(crate) async fn fetch_balances(
    state: &AppState,
    connection: &str,
    access: &AccessUrl,
    now: i64,
) -> Result<FeedBatch, WireProblem> {
    let count = count_request(state, connection, now);
    if count > auto::MANUAL_MAX {
        return Err(WireProblem::quota_refused(count - 1));
    }
    let query = AccountsQuery {
        balances_only: true,
        ..AccountsQuery::default()
    };
    match state.sync().provider().accounts(access, &query).await {
        Ok(batch) => {
            let mut remote = remote_summaries(&batch);
            for summary in &mut remote {
                if let Some(url) = summary.currency_url.clone()
                    && let Ok(Some(symbol)) =
                        state.sync().provider().currency_symbol(access, &url).await
                {
                    summary.currency = symbol;
                }
            }
            let host = access.host.clone();
            update_state(state, |memory| {
                let conn = memory
                    .connections
                    .entry(connection.to_string())
                    .or_default();
                conn.remote = remote;
                conn.host = Some(host);
                conn.last_errlist = errlist(&batch);
            });
            Ok(batch)
        }
        Err(error) => Err(feed_error_problem(
            &error,
            connection,
            &access.host,
            now,
            now,
        )),
    }
}

/// `later` folded into `earlier`: accounts by id, transactions by id, the
/// later window's balance, every problem and notice once.
fn merge(mut earlier: FeedBatch, later: FeedBatch) -> FeedBatch {
    for account in later.accounts {
        match earlier.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => {
                let seen: HashSet<String> =
                    existing.transactions.iter().map(|t| t.id.clone()).collect();
                existing.transactions.extend(
                    account
                        .transactions
                        .into_iter()
                        .filter(|txn| !seen.contains(&txn.id)),
                );
                existing.balance = account.balance;
                existing.available_balance = account.available_balance;
                existing.balance_date = account.balance_date;
            }
            None => earlier.accounts.push(account),
        }
    }
    for connection in later.connections {
        if !earlier.connections.iter().any(|c| c.id == connection.id) {
            earlier.connections.push(connection);
        }
    }
    for problem in later.problems {
        if !earlier.problems.contains(&problem) {
            earlier.problems.push(problem);
        }
    }
    for notice in later.notices {
        if !earlier.notices.contains(&notice) {
            earlier.notices.push(notice);
        }
    }
    earlier
}

/// The accounts a batch lists, for the Accounts tab and the wizard.
pub(crate) fn remote_summaries(batch: &FeedBatch) -> Vec<RemoteSummary> {
    batch
        .accounts
        .iter()
        .map(|account| {
            let org = account.org_name.clone().or_else(|| {
                account.conn_id.as_ref().and_then(|id| {
                    batch
                        .connections
                        .iter()
                        .find(|c| &c.id == id)
                        .map(|c| c.name.clone())
                })
            });
            let (currency, currency_url) = match &account.currency {
                FeedCurrency::Code(code) => (code.clone(), None),
                FeedCurrency::Url(url) => ("?".to_string(), Some(url.clone())),
            };
            RemoteSummary {
                id: account.id.clone(),
                name: account.name.clone(),
                org,
                org_url: account.org_url.clone(),
                balance: account.balance.clone(),
                balance_date: account.balance_date,
                currency,
                currency_url,
                oldest: account
                    .transactions
                    .iter()
                    .filter(|txn| txn.posted > 0)
                    .map(|txn| txn.posted)
                    .min()
                    .map(super::iso_utc),
            }
        })
        .collect()
}

/// The response's error list as text lines, for the diagnostics report.
fn errlist(batch: &FeedBatch) -> Vec<String> {
    batch
        .problems
        .iter()
        .map(|p| {
            if p.code.is_empty() {
                p.message.clone()
            } else {
                format!("{}: {}", p.code, p.message)
            }
        })
        .collect()
}

/// §6 for a failed request.
pub(crate) fn feed_error_problem(
    error: &FeedError,
    connection: &str,
    host: &str,
    since: i64,
    now: i64,
) -> WireProblem {
    let manage = manage_url(Some(host));
    match error {
        FeedError::Network(_) | FeedError::Timeout | FeedError::Server(_) => {
            WireProblem::network(host, since, now).detail(error.to_string())
        }
        FeedError::Forbidden => WireProblem::revoked(connection).detail(error.to_string()),
        FeedError::Payment => WireProblem::payment(&manage).detail(error.to_string()),
        FeedError::TokenUsed => WireProblem::token_used(&manage),
        FeedError::EdgeBlocked => WireProblem::edge_blocked().detail(error.to_string()),
        FeedError::Redirect(_) | FeedError::Status(_) | FeedError::BadResponse(_) => {
            WireProblem::other(
                format!("SimpleFIN's answer couldn't be used: {error}."),
                Severity::Error,
            )
            .detail(error.to_string())
        }
    }
}

/// §6 for the connection-wide problems in a response: quota warnings,
/// `gen.auth`, and a v1 "may need attention" with nothing to attach it to.
fn connection_problems(
    batch: &FeedBatch,
    connection: &str,
    host: &str,
    now: i64,
) -> Vec<WireProblem> {
    let manage = manage_url(Some(host));
    let mut out: Vec<WireProblem> = Vec::new();
    for problem in &batch.problems {
        let made = match problem.kind {
            ProblemKind::Quota => Some(WireProblem::quota().since(Some(now))),
            ProblemKind::GenAuth => Some(WireProblem::revoked(connection)),
            ProblemKind::NeedsAttention | ProblemKind::ConAuth
                if problem.conn_id.is_none() && problem.account_id.is_none() =>
            {
                Some(WireProblem::con_auth("", &manage))
            }
            _ => None,
        };
        if let Some(made) = made {
            let detail = if problem.code.is_empty() {
                problem.message.clone()
            } else {
                format!("{}: {}", problem.code, problem.message)
            };
            let made = made.detail(detail);
            if !out.iter().any(|p| p.code == made.code) {
                out.push(made);
            }
        }
    }
    out
}

/// §6 for the per-account problems in a response, by remote account id:
/// `con.auth` reaches every account of its institution, `act.*` its own.
fn account_problems(
    batch: &FeedBatch,
    manage: &str,
    now: i64,
) -> HashMap<String, Vec<WireProblem>> {
    let mut out: HashMap<String, Vec<WireProblem>> = HashMap::new();
    for problem in &batch.problems {
        let detail = if problem.code.is_empty() {
            problem.message.clone()
        } else {
            format!("{}: {}", problem.code, problem.message)
        };
        match problem.kind {
            ProblemKind::ConAuth | ProblemKind::NeedsAttention => {
                let Some(conn) = &problem.conn_id else {
                    continue;
                };
                let institution = batch
                    .connections
                    .iter()
                    .find(|c| &c.id == conn)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                for account in batch
                    .accounts
                    .iter()
                    .filter(|a| a.conn_id.as_ref() == Some(conn))
                {
                    out.entry(account.id.clone()).or_default().push(
                        WireProblem::con_auth(&institution, manage)
                            .since(Some(now))
                            .detail(detail.clone()),
                    );
                }
            }
            ProblemKind::ActFailed | ProblemKind::ActMissingData | ProblemKind::Act => {
                let Some(account) = &problem.account_id else {
                    continue;
                };
                // Its age is settled when it is recorded, against the
                // account's own `failingSince`.
                out.entry(account.clone()).or_default().push(
                    WireProblem::act_failed(0, manage)
                        .since(Some(now))
                        .detail(detail),
                );
            }
            _ => {}
        }
    }
    out
}

/// Normalize, split and import one account, under the import lock.
async fn import_account(
    state: &AppState,
    now: i64,
    offset: i32,
    ready: &ReadyLink,
    account: &FeedAccount,
    connection_name: &str,
    settings: &SyncSettings,
) -> AccountRun {
    let _imports = state.import_writes().lock().await;
    let worker = state.clone();
    let ready = ready.clone();
    let account = account.clone();
    let connection_name = connection_name.to_string();
    let settings = settings.clone();
    super::blocking(move || {
        let snapshot = worker.snapshot();
        let journal = &snapshot.raw;
        let known = dupes::known_ids(journal);
        let normalized = normalize(
            &account,
            &NormalizeOptions {
                start_date: &ready.link.start_date,
                account_short: &ready.key,
                columns: &ready.columns,
                utc_offset_seconds: offset,
            },
        );
        let data_through = normalized.rows.iter().map(|row| row.date.clone()).max();
        let fetched = fetched_ids(&account, &ready.key);
        let split = dupes::split(
            normalized.rows,
            journal,
            &ready.link.journal_account,
            &known,
            &fetched,
        );
        let mut run = AccountRun {
            held: split.held,
            already: split.already.len(),
            data_through,
            ..AccountRun::default()
        };
        if split.fresh.is_empty() {
            return Ok(run);
        }
        match import_rows(
            &worker,
            now,
            offset,
            &ready,
            &split.fresh,
            &connection_name,
            &settings,
        ) {
            Ok(imported) => {
                run.added = imported.added;
                run.uncategorized = imported.uncategorized;
            }
            Err(failure) => {
                run.outcome = Some(failure.outcome);
                run.problem = failure.problem;
            }
        }
        Ok(run)
    })
    .await
    .unwrap_or_else(|error| AccountRun {
        outcome: Some(Outcome::Failed),
        problem: Some(WireProblem::other(error.to_string(), Severity::Error)),
        ..AccountRun::default()
    })
}

/// What an import appended.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Imported {
    pub(crate) added: usize,
    pub(crate) uncategorized: usize,
}

/// Why an import wrote nothing.
#[derive(Debug, Clone)]
pub(crate) struct Failure {
    pub(crate) outcome: Outcome,
    pub(crate) problem: Option<WireProblem>,
}

impl Failure {
    fn failed(problem: WireProblem) -> Self {
        Self {
            outcome: Outcome::Failed,
            problem: Some(problem),
        }
    }
}

/// The folder an account's CSVs go in, journal-relative:
/// `<syncSourceDir>/<connection>/<account>-<key prefix>`.
pub(crate) fn source_folder(
    source_dir: &str,
    connection_name: &str,
    label: &str,
    key: &str,
) -> String {
    let prefix: String = key.chars().take(6).collect();
    format!(
        "{source_dir}/{}/{}-{prefix}",
        slug(connection_name),
        slug(label)
    )
}

/// Write `rows` as this fetch's CSV and import it through the import
/// screen's own dry run and commit. The caller holds `import_writes`.
///
/// # Errors
/// A [`Failure`]: a conflict is `skipped` with no problem (the next run
/// retries); everything else is `failed` with the §6 problem.
pub(crate) fn import_rows(
    state: &AppState,
    now: i64,
    offset: i32,
    ready: &ReadyLink,
    rows: &[NormRow],
    connection_name: &str,
    settings: &SyncSettings,
) -> Result<Imported, Failure> {
    if !state.editing_enabled() {
        return Err(Failure {
            outcome: Outcome::Skipped,
            problem: None,
        });
    }
    if let Err(error) = import_api::resolve_hledger(state.app_dirs()) {
        return Err(Failure::failed(WireProblem::hledger(&error.to_string())));
    }
    let location = state.snapshot().storage().ok_or_else(|| {
        Failure::failed(WireProblem::setup(
            "No journal file is open, so there is nowhere to sync into.",
        ))
    })?;
    let vault = location.vault();
    let root = location.root_dir();
    let folder = source_folder(
        &settings.source_dir,
        connection_name,
        &ready.link.label,
        &ready.key,
    );
    let outside = || {
        Failure::failed(WireProblem::setup(format!(
            "The sync source folder {:?} isn't inside the journal's folder.",
            settings.source_dir
        )))
    };
    // Resolved BEFORE anything is created, so a symlink out of the journal's
    // folder is refused rather than followed.
    let dir = root
        .join(&folder)
        .ok()
        .and_then(|dir| ledgeline_core::vault::canonicalize_existing_prefix(&**vault, &dir).ok())
        .filter(|dir| dir.starts_with(&root))
        .ok_or_else(outside)?;
    vault.create_dir_all(&dir).map_err(|error| {
        Failure::failed(WireProblem::setup(format!(
            "The sync source folder couldn't be created ({}).",
            error.kind()
        )))
    })?;
    let stamp = file_stamp(now, offset);
    let name = (1..100)
        .map(|n| {
            if n == 1 {
                format!("{stamp}.csv")
            } else {
                format!("{stamp}-{n}.csv")
            }
        })
        .find(|name| {
            dir.child(name)
                .ok()
                .is_some_and(|path| !vault.is_file(&path))
        })
        .ok_or_else(|| {
            Failure::failed(WireProblem::other(
                "Too many syncs this minute.",
                Severity::Warning,
            ))
        })?;
    let csv_path = format!("{folder}/{name}");
    let csv = to_csv(rows, &ready.columns);
    let stage_id = import_api::stage_headless(state, &name, &csv)
        .map_err(|error| Failure::failed(WireProblem::other(error.to_string(), Severity::Error)))?;
    let import = HeadlessImport {
        stage_id,
        rules_id: ready.link.rules.clone(),
        csv_path,
        journal_id: ready.link.target.clone(),
    };
    let entries = match import_api::headless_dry_run(state, &import) {
        Ok(HeadlessPreview::Proposed { count, entries }) => {
            if count == 0 {
                return Ok(Imported::default());
            }
            entries
        }
        Ok(HeadlessPreview::Failed { stderr }) => {
            return Err(Failure::failed(WireProblem::rules(
                &ready.link.rules,
                &stderr,
            )));
        }
        Err(error) => return Err(Failure::failed(app_error_problem(&error, ready))),
    };
    #[cfg(feature = "test-fakes")]
    {
        use std::sync::atomic::Ordering;
        let injected = &state.sync().inject_conflicts;
        if injected
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(Failure {
                outcome: Outcome::Skipped,
                problem: None,
            });
        }
    }
    let added = match import_api::headless_commit(state, &import) {
        Ok(added) => added,
        Err(AppError::Conflict(_)) => {
            return Err(Failure {
                outcome: Outcome::Skipped,
                problem: None,
            });
        }
        Err(error) => return Err(Failure::failed(app_error_problem(&error, ready))),
    };
    // Every fetch is a new file; a `.latest` marker would only ever hide a
    // late-arriving row from a later, hand-run import of this CSV.
    if let Ok(marker) = dir.child(&crate::stage::latest_name(&name)) {
        let _ = vault.remove_file(&marker);
    }
    prune(&**vault, &dir, now, offset, settings.keep_days);
    Ok(Imported {
        added,
        uncategorized: count_fallback(&entries, &ready.fallback),
    })
}

/// An import error as §6: the target, hledger, or anything else.
fn app_error_problem(error: &AppError, ready: &ReadyLink) -> WireProblem {
    let text = error.to_string();
    if text.contains("cannot be imported into") || text.contains("is not a journal") {
        WireProblem::target(&ready.link.target, &ready.key).detail(text)
    } else if text.contains("hledger") && matches!(error, AppError::Unavailable(_)) {
        WireProblem::hledger(&text)
    } else {
        WireProblem::other(
            format!("The import didn't go through: {text}"),
            Severity::Error,
        )
    }
}

/// `2026-11-14T0712` for `now` in local time.
fn file_stamp(now: i64, offset: i32) -> String {
    let local = now + i64::from(offset);
    let minutes = local.rem_euclid(DAY) / 60;
    format!(
        "{}T{:02}{:02}",
        iso_from_days(local.div_euclid(DAY)),
        minutes / 60,
        minutes % 60
    )
}

/// Delete this account folder's fetched CSVs older than `keep_days` (0 =
/// keep forever), judged by the date in their names. Only files this module
/// names are touched.
fn prune(
    vault: &dyn ledgeline_core::vault::Vault,
    dir: &VaultPath,
    now: i64,
    offset: i32,
    keep_days: u32,
) {
    if keep_days == 0 {
        return;
    }
    let today = days_from_iso(&auto::local_day(now, offset));
    let Ok(entries) = vault.list_dir(dir, Some(10_000)) else {
        return;
    };
    for entry in entries {
        let Some(name) = entry.path.file_name() else {
            continue;
        };
        let dated = name.len() >= 16
            && name.ends_with(".csv")
            && name.as_bytes().get(10) == Some(&b'T')
            && name[..10].bytes().filter(u8::is_ascii_digit).count() == 8;
        if dated && today - days_from_iso(&name[..10]) > i64::from(keep_days) {
            let _ = vault.remove_file(&entry.path);
        }
    }
}

/// How many transactions in `entries` post to `fallback`.
fn count_fallback(entries: &str, fallback: &str) -> usize {
    ledgeline_core::parse_journal(entries, "sync-entries.journal")
        .map(|journal| {
            journal
                .transactions
                .iter()
                .filter(|txn| txn.postings.iter().any(|p| p.account.0 == fallback))
                .count()
        })
        .unwrap_or(0)
}

/// `account`'s balance in `journal` at the end of `date`, summed across its
/// subaccounts: `(quantity, a sample amount for its commodity and style)`,
/// or `None` when it holds more than one commodity (or none).
pub(crate) fn ledger_balance(
    journal: &Journal,
    account: &str,
    date: &str,
) -> Option<(Dec, Amount)> {
    let prefix = format!("{account}:");
    let mut sums: BTreeMap<String, (Dec, Amount)> = BTreeMap::new();
    for txn in journal.transactions.iter() {
        for posting in &txn.postings {
            let name = &posting.account.0;
            if name != account && !name.starts_with(&prefix) {
                continue;
            }
            let day = posting.date.as_deref().unwrap_or(&txn.date);
            if day > date {
                continue;
            }
            for amount in &posting.amounts {
                let entry = sums
                    .entry(amount.commodity.0.clone())
                    .or_insert_with(|| (Dec::zero(), amount.clone()));
                entry.0 = entry.0.add(amount.quantity).ok()?;
            }
        }
    }
    sums.retain(|_, (sum, _)| !sum.is_zero());
    match sums.len() {
        0 => journal
            .transactions
            .iter()
            .flat_map(|txn| &txn.postings)
            .find(|p| p.account.0 == account || p.account.0.starts_with(&prefix))
            .and_then(|p| p.amounts.first())
            .map(|sample| (Dec::zero(), sample.clone())),
        1 => sums.into_values().next(),
        _ => None,
    }
}

/// `quantity` for display, in `sample`'s commodity and the journal's style
/// for it (digit groups included): `$32,292.23`.
pub(crate) fn render_like(journal: &Journal, quantity: Dec, sample: &Amount) -> String {
    // The `commodity` directive's style when there is one: a sample posting
    // may be an elided one, whose inferred amount has no style of its own.
    let style = journal
        .commodity_styles
        .iter()
        .find(|(commodity, _)| *commodity == sample.commodity)
        .map_or_else(|| sample.style.clone(), |(_, style)| style.clone());
    ledgeline_core::display::format_amount(
        &sample.commodity.0,
        quantity,
        &style,
        ledgeline_core::display::MAX_DISPLAY_DECIMALS,
    )
}

/// The bank's balance against the ledger's on the bank's balance date:
/// `(bank, ledger, difference)` with the difference `None` when they agree or
/// can't be compared.
fn balance_check(
    state: &AppState,
    ready: &ReadyLink,
    account: &FeedAccount,
    offset: i32,
) -> Option<(WireBankBalance, Option<String>, Option<String>)> {
    if account.balance.is_empty() {
        return None;
    }
    let date = account
        .balance_date
        .map(|epoch| epoch_to_date(epoch, offset))?;
    let bank = WireBankBalance {
        amount: account.balance.clone(),
        date: date.clone(),
    };
    let snapshot = state.snapshot();
    let Some((ledger, sample)) =
        ledger_balance(&snapshot.journal, &ready.link.journal_account, &date)
    else {
        return Some((bank, None, None));
    };
    let reported = Dec::parse(&account.balance, '.').ok()?;
    let signed = if ready.link.balance_sign < 0 {
        reported.neg().ok()?
    } else {
        reported
    };
    let diff = signed.sub(ledger).ok()?;
    Some((
        bank,
        Some(render_like(&snapshot.journal, ledger, &sample)),
        (!diff.is_zero())
            .then(|| render_like(&snapshot.journal, diff.abs().unwrap_or(diff), &sample)),
    ))
}

// ---------------------------------------------------------------------------
// Held rows
// ---------------------------------------------------------------------------

/// What the user decided about a held row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HeldAction {
    /// Import anyway: a one-row CSV through the same path.
    Import,
    /// It's a duplicate: record the row's id on the existing transaction.
    Duplicate,
}

/// Resolve one held row.
///
/// # Errors
/// `404` for an unknown account or row; `409` when the transaction it matched
/// has changed; the import's problem as `422`.
pub(crate) async fn resolve_held(
    state: &AppState,
    now: i64,
    key: &str,
    held_id: &str,
    action: HeldAction,
) -> Result<(), SyncReply> {
    let reader = state.clone();
    let wanted_key = key.to_string();
    let (link, memory) = super::blocking(move || {
        let (config, _) = read_config(&reader)?;
        let link = config
            .account(&wanted_key)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("no linked account {wanted_key:?}")))?;
        Ok((link, read_state(&reader)))
    })
    .await?;
    let held = memory
        .accounts
        .get(key)
        .and_then(|account| account.held.iter().find(|h| h.row.id == held_id))
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("no held transaction {held_id:?}")))?;
    let connection_name = read_config(state)?
        .0
        .connection(&link.connection)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "connection".to_string());

    match action {
        HeldAction::Import => {
            let _imports = state.import_writes().lock().await;
            let worker = state.clone();
            let row = held.row.clone();
            let outcome = super::blocking(move || {
                let snapshot = worker.snapshot();
                let ctx = link::Context::of(&worker, &snapshot);
                let ready =
                    ready(&ctx, &link).map_err(|problem| AppError::BadRequest(problem.message))?;
                if dupes::known_ids(&snapshot.raw).contains(&row.id) {
                    return Ok(Ok(Imported::default()));
                }
                let settings = ui_prefs::sync_settings(&worker);
                Ok(import_rows(
                    &worker,
                    now,
                    super::local_offset(now),
                    &ready,
                    std::slice::from_ref(&row),
                    &connection_name,
                    &settings,
                ))
            })
            .await?;
            match outcome {
                Ok(imported) => {
                    forget_held(state, key, held_id, now, imported.added);
                    Ok(())
                }
                Err(failure) => Err(failure.problem.map_or_else(
                    || {
                        SyncReply::App(AppError::Conflict(
                            "the journal changed while importing; try again".to_string(),
                        ))
                    },
                    SyncReply::from,
                )),
            }
        }
        HeldAction::Duplicate => {
            let worker = state.clone();
            let existing = held.existing.clone();
            let id = held.row.id.clone();
            super::blocking(move || {
                mark_duplicate(
                    &worker,
                    existing.index,
                    &existing.date,
                    &existing.description,
                    &id,
                )
            })
            .await?;
            forget_held(state, key, held_id, now, 0);
            Ok(())
        }
    }
}

/// Record `id` as one more `id:` tag on the transaction at `index`, through
/// the transaction editor, after checking it is still the one the row was
/// held against.
fn mark_duplicate(
    state: &AppState,
    index: usize,
    date: &str,
    description: &str,
    id: &str,
) -> Result<(), AppError> {
    let index =
        Tindex(u32::try_from(index).map_err(|_| AppError::NotFound("no such transaction".into()))?);
    let mut guard = crate::edit_api::lock_editor(state)?;
    let editor = crate::edit_api::bound(&mut guard)?;
    let same = editor
        .journal()
        .transactions
        .iter()
        .find(|txn| txn.index == index)
        .is_some_and(|txn| txn.date == date && txn.description == description);
    if !same {
        return Err(AppError::Conflict(
            "The transaction this row matched has changed since. Sync again to re-check it."
                .to_string(),
        ));
    }
    let fingerprint = editor
        .transaction_fingerprint(index)
        .ok_or_else(|| AppError::NotFound("no such transaction".to_string()))?;
    editor
        .apply_batch(&[(
            index,
            fingerprint,
            vec![EditOp::AddTxnTagValue {
                name: ledgeline_core::reimport::ID_TAG.to_string(),
                value: id.to_string(),
            }],
        )])
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    crate::edit_api::save_and_publish(state, &mut guard)
}

/// Drop a resolved held row and note the outcome.
fn forget_held(state: &AppState, key: &str, held_id: &str, now: i64, added: usize) {
    update_state(state, |memory| {
        let account: &mut AccountState = memory.accounts.entry(key.to_string()).or_default();
        account.held.retain(|held| held.row.id != held_id);
        account.problems.retain(|p| p.code != ProblemCode::Held);
        if !account.held.is_empty() {
            account
                .problems
                .push(WireProblem::held(account.held.len(), key));
        }
        if added > 0 {
            account.record(WireHistory {
                at: now,
                outcome: Outcome::Added,
                added,
                held: account.held.len(),
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

/// "Copy diagnostic report": connection kind and host, the last outcomes
/// with codes, the last response's error list, the app version. Never the
/// access URL — and run through `redact` anyway, so a stray one would be
/// scrubbed.
pub(crate) fn diagnostics(state: &AppState, connection: &str) -> Result<String, AppError> {
    let (config, _) = read_config(state)?;
    let link = config
        .connection(connection)
        .ok_or_else(|| AppError::NotFound(format!("no connection {connection:?}")))?;
    let memory = read_state(state);
    let empty = ConnState::default();
    let remembered = memory.connections.get(connection).unwrap_or(&empty);
    let mut text = String::new();
    text.push_str(&format!(
        "Ledgeline {} bank sync diagnostics\n",
        env!("CARGO_PKG_VERSION")
    ));
    text.push_str(&format!("connection: {} ({})\n", link.name, link.kind));
    text.push_str(&format!(
        "host: {}\n",
        remembered.host.as_deref().unwrap_or("unknown")
    ));
    text.push_str(&format!(
        "connected on this computer: {}\n",
        matches!(access_for(state, connection), Ok(Some(_)))
    ));
    text.push_str(&format!(
        "linked accounts: {}\n",
        config
            .accounts
            .iter()
            .filter(|a| a.connection == connection)
            .count()
    ));
    text.push_str("\nlast outcomes:\n");
    for entry in &remembered.log {
        text.push_str(&format!(
            "  {} {} {}\n",
            super::iso_utc(entry.at),
            entry.code,
            entry.detail
        ));
    }
    text.push_str("\nlast response's errors:\n");
    for line in &remembered.last_errlist {
        text.push_str(&format!("  {line}\n"));
    }
    for problem in &remembered.problems {
        text.push_str(&format!(
            "problem: {} — {}\n",
            problem.code.anchor(),
            problem.message
        ));
    }
    let text = crate::redact::scrub_credentials(&text);
    Ok(crate::redact::Redactor::for_journal(state).apply(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::feeds::{FeedConnection, FeedProblem};

    fn problem(
        kind: ProblemKind,
        code: &str,
        conn: Option<&str>,
        account: Option<&str>,
    ) -> FeedProblem {
        FeedProblem {
            kind,
            code: code.into(),
            message: "msg".into(),
            conn_id: conn.map(str::to_string),
            account_id: account.map(str::to_string),
        }
    }

    fn account(id: &str, conn: &str) -> FeedAccount {
        FeedAccount {
            id: id.into(),
            name: "Checking".into(),
            conn_id: Some(conn.into()),
            org_name: None,
            org_url: None,
            currency: FeedCurrency::Code("USD".into()),
            balance: "1.00".into(),
            available_balance: None,
            balance_date: None,
            transactions: Vec::new(),
        }
    }

    #[test]
    fn every_feed_error_maps_to_its_row() {
        let host = "beta-bridge.simplefin.org";
        let code = |error: FeedError| feed_error_problem(&error, "c1", host, 0, 10).code;
        assert_eq!(code(FeedError::Network("x".into())), ProblemCode::Network);
        assert_eq!(code(FeedError::Timeout), ProblemCode::Network);
        assert_eq!(code(FeedError::Server(503)), ProblemCode::Network);
        assert_eq!(code(FeedError::Forbidden), ProblemCode::Revoked);
        assert_eq!(code(FeedError::Payment), ProblemCode::Payment);
        assert_eq!(code(FeedError::TokenUsed), ProblemCode::TokenUsed);
        assert_eq!(code(FeedError::EdgeBlocked), ProblemCode::EdgeBlocked);
        assert_eq!(code(FeedError::Redirect(302)), ProblemCode::Other);
        assert_eq!(code(FeedError::BadResponse("x".into())), ProblemCode::Other);
        let revoked = feed_error_problem(&FeedError::Forbidden, "c1", host, 0, 10);
        assert_eq!(
            revoked.action.and_then(|a| a.reference).as_deref(),
            Some("c1")
        );
    }

    #[test]
    fn response_problems_reach_the_right_scope() {
        let batch = FeedBatch {
            connections: vec![FeedConnection {
                id: "CON-1".into(),
                name: "Chase".into(),
                org_url: None,
            }],
            accounts: vec![account("A1", "CON-1"), account("A2", "CON-2")],
            problems: vec![
                problem(ProblemKind::ConAuth, "con.auth", Some("CON-1"), None),
                problem(ProblemKind::ActFailed, "act.failed", None, Some("A2")),
                problem(ProblemKind::Quota, "gen.api", None, None),
                problem(ProblemKind::GenApi, "gen.api", None, None),
            ],
            notices: Vec::new(),
        };
        let conn = connection_problems(&batch, "c1", "beta-bridge.simplefin.org", 5);
        assert_eq!(
            conn.iter().map(|p| p.code).collect::<Vec<_>>(),
            [ProblemCode::Quota]
        );
        let accounts = account_problems(&batch, "https://x/", 5);
        assert_eq!(accounts["A1"][0].code, ProblemCode::ConAuth);
        assert_eq!(
            accounts["A1"][0].message,
            "Chase needs you to sign in again at SimpleFIN."
        );
        assert_eq!(accounts["A2"][0].code, ProblemCode::ActFailed);
        // v1's unattached "may need attention" is the connection's.
        let v1 = FeedBatch {
            problems: vec![problem(ProblemKind::NeedsAttention, "", None, None)],
            ..FeedBatch::default()
        };
        assert_eq!(
            connection_problems(&v1, "c1", "h", 5)[0].code,
            ProblemCode::ConAuth
        );
        // gen.auth means the credentials are refused: reconnect.
        let auth = FeedBatch {
            problems: vec![problem(ProblemKind::GenAuth, "gen.auth", None, None)],
            ..FeedBatch::default()
        };
        assert_eq!(
            connection_problems(&auth, "c1", "h", 5)[0].code,
            ProblemCode::Revoked
        );
    }

    #[test]
    fn merging_windows_dedups_transactions_and_keeps_the_latest_balance() {
        let mut first = account("A1", "C");
        first.transactions = vec![txn("T1", 10), txn("T2", 20)];
        let mut second = account("A1", "C");
        second.balance = "2.00".into();
        second.transactions = vec![txn("T2", 20), txn("T3", 30)];
        let merged = merge(
            FeedBatch {
                accounts: vec![first],
                ..FeedBatch::default()
            },
            FeedBatch {
                accounts: vec![second],
                ..FeedBatch::default()
            },
        );
        let ids: Vec<&str> = merged.accounts[0]
            .transactions
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ids, ["T1", "T2", "T3"]);
        assert_eq!(merged.accounts[0].balance, "2.00");
    }

    fn txn(id: &str, posted: i64) -> ledgeline_core::feeds::FeedTxn {
        ledgeline_core::feeds::FeedTxn {
            id: id.into(),
            posted,
            amount: "-1.00".into(),
            description: "X".into(),
            payee: None,
            memo: None,
            transacted_at: None,
            pending: false,
        }
    }

    #[test]
    fn file_stamps_are_local_minutes() {
        // 2026-11-14 07:12 at UTC-7 is 14:12 UTC.
        let now = days_from_iso("2026-11-14") * DAY + 14 * 3_600 + 12 * 60;
        assert_eq!(file_stamp(now, -7 * 3_600), "2026-11-14T0712");
    }

    #[test]
    fn columns_come_from_the_fields_line() {
        let shared = "skip 1\nfields date, amount, name, sfdescription, memo, id, transacted\n";
        assert_eq!(columns_of(shared).payee, "name");
        assert_eq!(columns_of("skip 1\n"), FeedColumns::default());
        assert!(fits_sync(&format!("{shared}comment id:%id\n")));
        assert!(!fits_sync("fields date, amount, name\ncomment id:%fitid\n"));
    }

    #[test]
    fn the_source_folder_is_slugged() {
        assert_eq!(
            source_folder(
                "imports/sync",
                "SimpleFIN Bridge",
                "First Tech · Checking ••1234",
                "a1b2c3d4e5"
            ),
            "imports/sync/simplefin-bridge/first-tech-checking-1234-a1b2c3"
        );
    }
}
