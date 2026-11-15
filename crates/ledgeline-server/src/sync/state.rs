//! `sync-state.json` (plan 43 §3.6, "Machine state is not config").
//!
//! What a sync remembers between runs and relaunches: per connection, when it
//! last tried and last succeeded, how many requests it made per UTC day (the
//! quota guard must survive a relaunch), and a pause after a quota warning;
//! per linked account, its last ten outcomes, its held rows, and the balance
//! check.
//!
//! It lives in the app's **cache** directory, never beside the journal, and
//! it is **disposable**: losing it costs one extra fetch and an empty history.
//! So it is never a reason to fail. A missing, unreadable or corrupt file
//! reads as empty; a write is atomic (temp file, then rename) and its failure
//! is logged, not raised.
//!
//! Bank data in here is the minimum the UI needs: held rows (date, amount,
//! payee) and balances. No credential, no raw response.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ledgeline_core::feeds::dupes::Held;
use serde::{Deserialize, Serialize};

use super::wire::{WireBankBalance, WireHistory, WireProblem};

/// The file's name in the cache directory.
pub(crate) const FILE_NAME: &str = "sync-state.json";

/// The format version; anything else reads as empty.
const VERSION: u32 = 1;

/// How many outcomes an account keeps.
pub(crate) const HISTORY_LEN: usize = 10;

/// How many UTC days of request counts are kept (today and two before, for
/// a clock that moved).
const REQUEST_DAYS_KEPT: usize = 3;

/// The whole file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncState {
    #[serde(default)]
    pub(crate) version: u32,
    /// By connection id.
    #[serde(default)]
    pub(crate) connections: BTreeMap<String, ConnState>,
    /// By account key (`AccountLink::key`).
    #[serde(default)]
    pub(crate) accounts: BTreeMap<String, AccountState>,
}

/// One connection's memory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnState {
    /// Requests made, by UTC date (`YYYY-MM-DD`).
    #[serde(default)]
    pub(crate) requests: BTreeMap<String, u32>,
    #[serde(default)]
    pub(crate) last_attempt: Option<i64>,
    #[serde(default)]
    pub(crate) last_success: Option<i64>,
    /// After a quota warning, no auto run before this (the next UTC midnight).
    #[serde(default)]
    pub(crate) quota_until: Option<i64>,
    /// The first of an unbroken run of network failures, for "since…".
    #[serde(default)]
    pub(crate) failing_since: Option<i64>,
    /// The connection-level problems from the last attempt.
    #[serde(default)]
    pub(crate) problems: Vec<WireProblem>,
    /// The last response's error list, sanitized, for the diagnostics report.
    #[serde(default)]
    pub(crate) last_errlist: Vec<String>,
    /// The last outcomes, newest last, for the diagnostics report.
    #[serde(default)]
    pub(crate) log: Vec<LogEntry>,
    /// The provider's host, from the access URL (display only; the keychain
    /// stays the source of truth).
    #[serde(default)]
    pub(crate) host: Option<String>,
    /// Every account the provider listed last time, for the "not linked"
    /// rows and the wizard.
    #[serde(default)]
    pub(crate) remote: Vec<RemoteSummary>,
}

/// One account the provider listed: what the Accounts tab shows of an
/// account that isn't linked yet. Balances only, no transactions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) org: Option<String>,
    #[serde(default)]
    pub(crate) org_url: Option<String>,
    /// Decimal text, or `""` when unknown.
    #[serde(default)]
    pub(crate) balance: String,
    #[serde(default)]
    pub(crate) balance_date: Option<i64>,
    /// `USD`, or `?` for a custom currency.
    #[serde(default)]
    pub(crate) currency: String,
    /// A custom currency's URL, kept to fetch its symbol from the
    /// connection's own host.
    #[serde(default)]
    pub(crate) currency_url: Option<String>,
    /// The earliest posted date seen for it, ISO.
    #[serde(default)]
    pub(crate) oldest: Option<String>,
}

/// One line of a connection's diagnostic log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogEntry {
    pub(crate) at: i64,
    /// `ok`, or a problem code.
    pub(crate) code: String,
    pub(crate) detail: String,
}

/// One linked account's memory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountState {
    #[serde(default)]
    pub(crate) last_success: Option<i64>,
    /// The newest posted date seen in a fetch.
    #[serde(default)]
    pub(crate) data_through: Option<String>,
    #[serde(default)]
    pub(crate) last_added: usize,
    /// When a sync last brought a row this journal didn't have.
    #[serde(default)]
    pub(crate) last_new_data: Option<i64>,
    #[serde(default)]
    pub(crate) bank_balance: Option<WireBankBalance>,
    #[serde(default)]
    pub(crate) ledger_balance: Option<String>,
    #[serde(default)]
    pub(crate) balance_diff: Option<String>,
    /// Possible duplicates waiting for the user, oldest first. The row id is
    /// the stable `heldId`.
    #[serde(default)]
    pub(crate) held: Vec<Held>,
    #[serde(default)]
    pub(crate) history: Vec<WireHistory>,
    /// The account's problems from the last attempt.
    #[serde(default)]
    pub(crate) problems: Vec<WireProblem>,
    /// The first of an unbroken run of `act.failed`.
    #[serde(default)]
    pub(crate) failing_since: Option<i64>,
}

impl ConnState {
    /// Requests made on `utc_day`.
    pub(crate) fn requests_on(&self, utc_day: &str) -> u32 {
        self.requests.get(utc_day).copied().unwrap_or(0)
    }

    /// Count one more request on `utc_day`, forgetting old days.
    pub(crate) fn count_request(&mut self, utc_day: &str) {
        *self.requests.entry(utc_day.to_string()).or_insert(0) += 1;
        while self.requests.len() > REQUEST_DAYS_KEPT {
            let Some(oldest) = self.requests.keys().next().cloned() else {
                break;
            };
            self.requests.remove(&oldest);
        }
    }

    /// Add a diagnostic log line, keeping the last [`HISTORY_LEN`].
    pub(crate) fn log(&mut self, at: i64, code: &str, detail: &str) {
        self.log.push(LogEntry {
            at,
            code: code.to_string(),
            detail: super::wire::clip(detail),
        });
        let excess = self.log.len().saturating_sub(HISTORY_LEN);
        self.log.drain(..excess);
    }
}

impl AccountState {
    /// Add an outcome, keeping the last [`HISTORY_LEN`].
    pub(crate) fn record(&mut self, entry: WireHistory) {
        self.history.push(entry);
        let excess = self.history.len().saturating_sub(HISTORY_LEN);
        self.history.drain(..excess);
    }

    /// Merge newly held rows in, by row id: one already waiting keeps its
    /// place, and a row the journal now has by id is no longer held.
    pub(crate) fn hold(&mut self, new: Vec<Held>, known: &std::collections::HashSet<String>) {
        self.held.retain(|held| !known.contains(&held.row.id));
        for held in new {
            if !self.held.iter().any(|kept| kept.row.id == held.row.id) {
                self.held.push(held);
            }
        }
    }
}

/// Where the file is, when the app has a cache directory.
pub(crate) fn path_in(cache_dir: Option<&Path>) -> Option<PathBuf> {
    cache_dir.map(|dir| dir.join(FILE_NAME))
}

/// Read the state, or an empty one for any trouble at all.
pub(crate) fn load(path: Option<&Path>) -> SyncState {
    path.and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<SyncState>(&bytes).ok())
        .filter(|state| state.version == VERSION)
        .unwrap_or_else(|| SyncState {
            version: VERSION,
            ..SyncState::default()
        })
}

/// Write the state atomically: a temp file beside it, then a rename. A
/// failure is logged and otherwise ignored (the file is disposable).
pub(crate) fn save(path: Option<&Path>, state: &SyncState) {
    let Some(path) = path else {
        return;
    };
    let written = (|| -> std::io::Result<()> {
        let dir = path
            .parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?;
        std::fs::create_dir_all(dir)?;
        let bytes = serde_json::to_vec_pretty(&SyncState {
            version: VERSION,
            ..state.clone()
        })
        .map_err(std::io::Error::other)?;
        let temp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, path)
    })();
    if let Err(error) = written {
        eprintln!(
            "ledgeline: could not save the bank sync state: {}",
            error.kind()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::wire::Outcome;

    #[test]
    fn a_missing_or_corrupt_file_reads_as_empty() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join(FILE_NAME);
        assert_eq!(load(Some(&path)).connections.len(), 0);
        std::fs::write(&path, b"{not json").expect("write");
        assert_eq!(
            load(Some(&path)),
            SyncState {
                version: 1,
                ..SyncState::default()
            }
        );
        std::fs::write(&path, br#"{"version": 99, "connections": {"c": {}}}"#).expect("write");
        assert!(
            load(Some(&path)).connections.is_empty(),
            "another version is ignored"
        );
        assert_eq!(load(None).version, 1);
    }

    #[test]
    fn it_round_trips_atomically_and_keeps_quota_counts() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("nested").join(FILE_NAME);
        let mut state = load(Some(&path));
        let conn = state.connections.entry("c1".into()).or_default();
        conn.count_request("2026-11-14");
        conn.count_request("2026-11-14");
        save(Some(&path), &state);
        let reloaded = load(Some(&path));
        assert_eq!(reloaded.connections["c1"].requests_on("2026-11-14"), 2);
        assert_eq!(reloaded, state);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("list")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn request_days_and_history_are_bounded() {
        let mut conn = ConnState::default();
        for day in 1..=5 {
            conn.count_request(&format!("2026-11-0{day}"));
        }
        assert_eq!(conn.requests.len(), 3);
        assert!(conn.requests.contains_key("2026-11-05"));
        let mut account = AccountState::default();
        for at in 0..15 {
            account.record(WireHistory {
                at,
                outcome: Outcome::NothingNew,
                added: 0,
                held: 0,
            });
        }
        assert_eq!(account.history.len(), HISTORY_LEN);
        assert_eq!(account.history[0].at, 5);
    }
}
