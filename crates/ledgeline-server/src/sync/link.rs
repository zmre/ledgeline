//! The connect-a-bank wizard's server half (plan 43 §4.3): claiming a token,
//! listing the provider's accounts with suggested mappings, previewing a
//! first sync, writing the links, and sharing an existing manual rules file's
//! categorization (§3.2).
//!
//! # What is written, and how
//!
//! - **Rules files** through `Discovery::resolve_new` (a new name) and the
//!   vault's exclusive create; never over an existing file. Sharing rewrites
//!   the manual file pinned to the bytes it planned from, and rolls every
//!   write back if a later one fails.
//! - **`ledgeline.toml`** through `feeds::config`'s `toml_edit` writers and
//!   `ui_prefs`' pinned writer and lock. Linking may create the file.
//! - **A new account** gets an `account` declaration with its type and, when
//!   asked, an opening-balance transaction, appended to the link's target
//!   file (decision: the target, so the account's first lines sit where its
//!   synced ones will).
//!
//! The first sync of a newly linked account then runs through the engine.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use ledgeline_core::Dec;
use ledgeline_core::feeds::config::{
    self as feed_config, AccountLink, ConnectionLink, DEFAULT_KIND, SyncConfig,
};
use ledgeline_core::feeds::dupes;
use ledgeline_core::feeds::normalize::{
    FeedColumns, NormalizeOptions, fetched_ids, normalize, to_csv,
};
use ledgeline_core::feeds::simplefin::{AccessUrl, decode_setup_token_with};
use ledgeline_core::feeds::{FeedBatch, sanitize};
use ledgeline_core::journals::JournalTarget;
use ledgeline_core::model::{Amount, Journal};
use ledgeline_core::reports::account_types::{AccountType, AccountTypes, account_decls};
use ledgeline_core::reports::periods::{days_from_iso, iso_from_days};
use ledgeline_core::rules::generate::{DEFAULT_ACCOUNT2, FeedRulesParams, feed_rules};
use ledgeline_core::rules::share::{self, SharePlan, plan_share};
use ledgeline_core::rules::{DiscoveredRules, Discovery};
use serde::Deserialize;

use super::auto::{self, Mode};
use super::engine::{self, DEFAULT_FALLBACK};
use super::state::{RemoteSummary, SyncState};
use super::wire::{
    Severity, WireExisting, WireHeld, WireKept, WireLinkPreview, WireLinkedAccount,
    WireManualAccount, WireManualRules, WireProblem, WireRemoteAccount, WireRulesChoice,
    WireShareDiffs, WireSharePreview, WireSuggestion, WireSyncRun, manage_url,
};
use super::{SyncReply, access_for, blocking, read_config, read_state, update_state, write_config};
use crate::error::AppError;
use crate::{AppState, JournalLocation, Snapshot};

/// What a request needs to know about the open journal, read once.
pub(crate) struct Context {
    /// The journal as served (auto postings applied): balances.
    pub(crate) journal: Arc<Journal>,
    /// The journal as written: ids, the transactions files hold.
    pub(crate) raw: Arc<Journal>,
    pub(crate) location: Option<Arc<JournalLocation>>,
    pub(crate) discovery: Option<Discovery>,
    pub(crate) targets: Vec<JournalTarget>,
}

impl Context {
    pub(crate) fn of(_state: &AppState, snapshot: &Snapshot) -> Self {
        let location = snapshot.storage();
        let discovery = location.as_ref().map(|location| {
            ledgeline_core::rules::discover_in(Arc::clone(location.vault()), location.main())
        });
        let targets = location
            .as_ref()
            .map(|location| location.targets(&snapshot.journal))
            .unwrap_or_default();
        Self {
            journal: Arc::clone(&snapshot.journal),
            raw: Arc::clone(&snapshot.raw),
            location,
            discovery,
            targets,
        }
    }

    /// Rules files that read the sync CSV.
    fn sync_rules(&self) -> Vec<WireRulesChoice> {
        self.discovery
            .iter()
            .flat_map(|discovery| discovery.files.iter())
            .filter(|file| file.id.ends_with(".rules"))
            .filter_map(|file| {
                let text = file.path().read_text()?;
                let fits = engine::fits_sync(&text);
                (fits || file.id.contains(".sync.")).then(|| WireRulesChoice {
                    id: file.id.clone(),
                    fits,
                })
            })
            .collect()
    }

    /// A manual (non-sync) rules file whose `account1` is `account`.
    fn manual_rules_for(&self, account: &str) -> Option<&DiscoveredRules> {
        self.discovery.as_ref()?.files.iter().find(|file| {
            file.account1.as_deref() == Some(account)
                && file
                    .path()
                    .read_text()
                    .is_some_and(|text| !engine::fits_sync(&text))
        })
    }

    /// The target file holding `account`'s newest transaction.
    fn target_of(&self, account: &str) -> Option<String> {
        let location = self.location.as_ref()?;
        let prefix = format!("{account}:");
        let newest = self
            .raw
            .transactions
            .iter()
            .filter(|txn| {
                txn.postings
                    .iter()
                    .any(|p| p.account.0 == account || p.account.0.starts_with(&prefix))
            })
            .max_by(|a, b| a.date.cmp(&b.date))?;
        self.targets
            .iter()
            .filter(|target| target.writable)
            .find(|target| {
                location
                    .source_path(&self.raw, &target.id)
                    .is_some_and(|path| location.key(&path) == newest.source_file)
            })
            .map(|target| target.id.clone())
    }

    /// The default target: the best-ranked writable file.
    fn default_target(&self) -> Option<String> {
        self.targets
            .iter()
            .find(|target| target.writable)
            .map(|target| target.id.clone())
    }
}

// ===========================================================================
// Linked and unlinked accounts, for the status
// ===========================================================================

/// The categories file a rules file includes, if any.
fn categories_of(ctx: &Context, rules_id: &str) -> Option<String> {
    let text = ctx
        .discovery
        .as_ref()?
        .resolve(rules_id)?
        .path()
        .read_text()?;
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("include "))
        .map(str::trim)
        .find(|name| name.ends_with(".categories.rules"))
        .map(|name| match rules_id.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{name}"),
            None => name.to_string(),
        })
}

/// One linked account as the status shows it.
pub(crate) fn linked_account(
    _state: &AppState,
    ctx: &Context,
    config: &SyncConfig,
    memory: &SyncState,
    link: &AccountLink,
) -> WireLinkedAccount {
    let key = link.key();
    let remembered = memory.accounts.get(&key).cloned().unwrap_or_default();
    let org = memory
        .connections
        .get(&link.connection)
        .and_then(|c| c.remote.iter().find(|r| r.id == link.remote_id))
        .and_then(|r| r.org.clone());
    let mut problems = remembered.problems.clone();
    if let Err(problem) = engine::ready(ctx, link) {
        problems.retain(|p| p.code != problem.code);
        problems.insert(0, problem);
    }
    let _ = config;
    WireLinkedAccount {
        key: key.clone(),
        connection_id: link.connection.clone(),
        remote_id: link.remote_id.clone(),
        label: link.label.clone(),
        org,
        journal_account: link.journal_account.clone(),
        target: link.target.clone(),
        rules: link.rules.clone(),
        categories: categories_of(ctx, &link.rules),
        start_date: link.start_date.clone(),
        balance_sign: link.balance_sign,
        enabled: link.enabled,
        last_success: remembered.last_success,
        data_through: remembered.data_through.clone(),
        last_added: remembered.last_added,
        bank_balance: remembered.bank_balance.clone(),
        ledger_balance: remembered.ledger_balance.clone(),
        balance_diff: remembered.balance_diff.clone(),
        held: remembered
            .held
            .iter()
            .map(|held| WireHeld {
                held_id: held.row.id.clone(),
                date: held.row.date.clone(),
                amount: held.row.amount.clone(),
                payee: held.row.payee.clone(),
                existing: WireExisting {
                    index: held.existing.index,
                    date: held.existing.date.clone(),
                    description: held.existing.description.clone(),
                    amount: held.existing.amount.clone(),
                },
            })
            .collect(),
        history: remembered.history.clone(),
        problems,
        rules_choices: ctx.sync_rules(),
    }
}

/// The provider's accounts on `connection` that no link names.
pub(crate) fn unlinked_accounts(
    _state: &AppState,
    ctx: &Context,
    config: &SyncConfig,
    memory: &SyncState,
    connection: &str,
) -> Vec<WireRemoteAccount> {
    let linked: HashSet<&str> = config
        .accounts
        .iter()
        .filter(|a| a.connection == connection)
        .map(|a| a.remote_id.as_str())
        .collect();
    memory
        .connections
        .get(connection)
        .map(|c| c.remote.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|remote| !linked.contains(remote.id.as_str()))
        .map(|remote| remote_account(ctx, connection, remote, None))
        .collect()
}

// ===========================================================================
// Suggestions
// ===========================================================================

/// The last four digits in an account name (`Checking ••1234` → `1234`).
fn last_four(name: &str) -> Option<String> {
    let digits: Vec<char> = name
        .chars()
        .rev()
        .skip_while(|c| !c.is_ascii_digit())
        .collect();
    let run: String = digits
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    (run.len() >= 4).then(|| run[run.len() - 4..].to_string())
}

/// Lowercase words of three or more letters.
fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// Every asset or liability account the journal knows.
fn balance_sheet_accounts(journal: &Journal) -> Vec<String> {
    let types = AccountTypes::new(&account_decls(journal));
    let mut names: Vec<String> = journal
        .transactions
        .iter()
        .flat_map(|txn| txn.postings.iter().map(|p| p.account.0.clone()))
        .chain(journal.accounts.iter().map(|decl| decl.name.0.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|name| {
            types.is_type(name, AccountType::Asset) || types.is_type(name, AccountType::Liability)
        })
        .collect();
    names.sort();
    names
}

/// A liability by its name or balance.
fn looks_like_liability(remote: &RemoteSummary) -> bool {
    let name = format!(
        "{} {}",
        remote.name.to_lowercase(),
        remote.org.as_deref().unwrap_or_default().to_lowercase()
    );
    [
        "card",
        "credit",
        "visa",
        "mastercard",
        "amex",
        "loan",
        "mortgage",
        "sapphire",
    ]
    .iter()
    .any(|word| name.contains(word))
        || remote.balance.starts_with('-')
}

/// A name for a new account: `assets:bank:<org>:<name>` or
/// `liabilities:<org>:<name>`.
fn new_account_name(remote: &RemoteSummary) -> (String, &'static str) {
    let part = |text: &str| {
        let words: Vec<String> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()))
            .map(str::to_lowercase)
            .collect();
        words.join("-")
    };
    let org = remote.org.as_deref().map(part).filter(|o| !o.is_empty());
    let name = Some(part(&remote.name))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "account".to_string());
    let tail = match org {
        Some(org) => format!("{org}:{name}"),
        None => name,
    };
    if looks_like_liability(remote) {
        (format!("liabilities:{tail}"), "L")
    } else {
        let lower = remote.name.to_lowercase();
        let kind = if ["checking", "savings", "cash"]
            .iter()
            .any(|w| lower.contains(w))
        {
            "C"
        } else {
            "A"
        };
        (format!("assets:bank:{tail}"), kind)
    }
}

/// One candidate account: its score, its name, its balance on the bank's
/// date, and whether the bank's sign is the reverse of the ledger's.
type Scored<'a> = (f64, &'a String, Option<(Dec, Amount)>, bool);

/// `remote` as a wire account with its suggested mapping. `linked` is the
/// journal account it is already linked to, if any.
fn remote_account(
    ctx: &Context,
    connection: &str,
    remote: &RemoteSummary,
    linked: Option<&str>,
) -> WireRemoteAccount {
    let balance_date = remote.balance_date.map(super::iso_utc);
    let today = super::iso_utc(super::now_seconds());
    let as_of = balance_date.clone().unwrap_or(today.clone());
    let bank = Dec::parse(&remote.balance, '.').ok();
    let candidates = balance_sheet_accounts(&ctx.journal);
    let four = last_four(&remote.name);
    let remote_words: HashSet<String> = words(&format!(
        "{} {}",
        remote.name,
        remote.org.as_deref().unwrap_or_default()
    ));
    let scored: Vec<Scored> = candidates
        .iter()
        .map(|account| {
            let mut score = 0.0;
            if let Some(four) = &four
                && account.contains(four.as_str())
            {
                score += 0.5;
            }
            let shared = words(account).intersection(&remote_words).count();
            score += (0.2 * shared as f64).min(0.4);
            let ledger = engine::ledger_balance(&ctx.journal, account, &as_of);
            let mut flipped = false;
            if let (Some(bank), Some((ledger, _))) = (bank, &ledger)
                && !ledger.is_zero()
            {
                if bank == *ledger {
                    score += 0.3;
                } else if bank.neg().ok() == Some(*ledger) {
                    score += 0.3;
                    flipped = true;
                }
            }
            (score, account, ledger, flipped)
        })
        .collect();
    let best = match linked {
        Some(account) => scored.iter().find(|(_, a, _, _)| a.as_str() == account),
        None => scored
            .iter()
            .filter(|(score, ..)| *score >= 0.2)
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)),
    };
    let journal_account = best.map(|(_, account, ..)| (*account).clone());
    let ledger_balance = best
        .and_then(|(_, _, ledger, _)| ledger.as_ref())
        .map(|(quantity, sample)| engine::render_like(&ctx.journal, *quantity, sample));
    let start_date = journal_account
        .as_deref()
        .and_then(|account| last_date(&ctx.raw, account))
        .map(|date| iso_from_days(days_from_iso(&date) + 1))
        .or_else(|| remote.oldest.clone())
        .unwrap_or_else(|| iso_from_days(days_from_iso(&today) - 30));
    let (new_account_name, new_account_type) = new_account_name(remote);
    let target = journal_account
        .as_deref()
        .and_then(|account| ctx.target_of(account))
        .or_else(|| ctx.default_target());
    let manual_rules = journal_account
        .as_deref()
        .and_then(|account| ctx.manual_rules_for(account))
        .map(|file| WireManualRules {
            id: file.id.clone(),
            rule_count: file.if_block_count,
        });
    WireRemoteAccount {
        connection_id: connection.to_string(),
        remote_id: remote.id.clone(),
        label: label_of(remote),
        org: remote.org.clone(),
        balance: remote.balance.clone(),
        balance_date,
        currency: remote.currency.clone(),
        oldest_available: remote.oldest.clone(),
        suggestion: WireSuggestion {
            score: best.map_or(0.0, |(score, ..)| (*score).min(1.0)),
            sign_flipped: best.is_some_and(|(.., flipped)| *flipped),
            journal_account,
            ledger_balance,
            start_date,
            target,
            new_account_name,
            new_account_type: new_account_type.to_string(),
            sync_rules: ctx.sync_rules(),
            manual_rules,
        },
    }
}

/// `First Tech · Checking ••1234`.
fn label_of(remote: &RemoteSummary) -> String {
    match &remote.org {
        Some(org) if !org.is_empty() && !remote.name.contains(org.as_str()) => {
            format!("{org} · {}", remote.name)
        }
        _ => remote.name.clone(),
    }
}

/// The newest date of a transaction posting to `account` (or below).
fn last_date(journal: &Journal, account: &str) -> Option<String> {
    let prefix = format!("{account}:");
    journal
        .transactions
        .iter()
        .filter(|txn| {
            txn.postings
                .iter()
                .any(|p| p.account.0 == account || p.account.0.starts_with(&prefix))
        })
        .map(|txn| txn.date.clone())
        .max()
}

// ===========================================================================
// Connect and list
// ===========================================================================

/// A fresh connection id: a v4 UUID from the OS random source.
fn new_connection_id() -> Result<String, AppError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| {
        AppError::Internal("the operating system's random source is unavailable".into())
    })?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// SimpleFIN Bridge's host — or, in a test build, the fake bridge's, so e2e
/// runs and docs screenshots read like the real thing.
fn is_bridge(host: &str) -> bool {
    #[cfg(feature = "test-fakes")]
    if host == super::client::FAKE_HOST {
        return true;
    }
    host == super::wire::BRIDGE_HOST
}

/// Claim `setup_token`, keep the access URL in the keychain, record the
/// connection in `ledgeline.toml`, and list its accounts (one balances-only
/// request). With `existing`, replace that connection's token and keep its id
/// and links.
///
/// Answers `{connectionId, accounts}`, plus `problem` when the claim worked
/// but the listing didn't (the connection is kept, and "Re-check" lists it
/// later).
///
/// # Errors
/// A `422` problem when the token can't be used or claimed, or the keychain
/// isn't there; `404` for an unknown `existing`.
pub(crate) async fn connect(
    state: &AppState,
    setup_token: &str,
    name: Option<&str>,
    existing: Option<&str>,
) -> Result<serde_json::Value, SyncReply> {
    let session = state.sync();
    let secrets = Arc::clone(session.secrets());
    blocking(move || {
        secrets
            .check()
            .map_err(|error| AppError::BadRequest(error.to_string()))
    })
    .await
    .map_err(|error| WireProblem::setup(error.to_string()))?;
    let token = decode_setup_token_with(setup_token, session.loopback_http())
        .map_err(|error| WireProblem::setup(error.to_string()))?;
    if let Some(id) = existing {
        let (config, _) = read_config(state)?;
        if config.connection(id).is_none() {
            return Err(AppError::NotFound(format!("no connection {id:?}")).into());
        }
    }
    let now = super::now_seconds();
    let access_text = session.provider().claim(&token).await.map_err(|error| {
        engine::feed_error_problem(&error, existing.unwrap_or(""), &token.host, now, now)
    })?;
    let access = AccessUrl::parse(&access_text, session.loopback_http()).map_err(|_| {
        WireProblem::other(
            "SimpleFIN's answer to the token wasn't an access address Ledgeline can use.",
            Severity::Error,
        )
    })?;
    let id = match existing {
        Some(id) => id.to_string(),
        None => new_connection_id()?,
    };
    let worker = state.clone();
    let secret = access.to_secret_string();
    let display = name
        .map(sanitize)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            if is_bridge(&token.host) {
                "SimpleFIN Bridge".to_string()
            } else {
                token.host.clone()
            }
        });
    let conn_id = id.clone();
    let host = access.host.clone();
    let is_new = existing.is_none();
    blocking(move || {
        worker
            .sync()
            .secrets()
            .set(&conn_id, &secret)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        if is_new {
            let link = ConnectionLink {
                id: conn_id.clone(),
                name: display.clone(),
                kind: DEFAULT_KIND.to_string(),
            };
            if let Err(error) =
                write_config(&worker, |text| feed_config::upsert_connection(text, &link))
            {
                // Nothing points at the key; don't leave it behind.
                let _ = worker.sync().secrets().delete(&conn_id);
                return Err(error);
            }
        }
        update_state(&worker, |memory| {
            let conn = memory.connections.entry(conn_id.clone()).or_default();
            conn.problems.clear();
            conn.failing_since = None;
            conn.host = Some(host.clone());
        });
        worker.sync().forget_batch(&conn_id);
        Ok(())
    })
    .await
    .map_err(|error| WireProblem::setup(error.to_string()))?;

    let listed = engine::fetch_balances(state, &id, &access, now).await;
    let reader = state.clone();
    let conn_id = id.clone();
    let accounts = blocking(move || Ok(accounts_for(&reader, &conn_id))).await?;
    let mut answer = serde_json::json!({ "connectionId": id, "accounts": accounts });
    match listed {
        Err(problem) => answer["problem"] = serde_json::to_value(problem).unwrap_or_default(),
        Ok(batch) if batch.accounts.is_empty() => {
            answer["problem"] = serde_json::to_value(WireProblem::other(
                "SimpleFIN has no banks connected yet. Connect one on SimpleFIN, then press Re-check.",
                Severity::Info,
            ))
            .unwrap_or_default();
        }
        Ok(_) => {}
    }
    Ok(answer)
}

/// Every remote account remembered for `connection`, with suggestions.
fn accounts_for(state: &AppState, connection: &str) -> Vec<WireRemoteAccount> {
    let snapshot = state.snapshot();
    let ctx = Context::of(state, &snapshot);
    let memory = read_state(state);
    let linked: HashMap<String, String> = read_config(state)
        .map(|(config, _)| {
            config
                .accounts
                .into_iter()
                .filter(|a| a.connection == connection)
                .map(|a| (a.remote_id, a.journal_account))
                .collect()
        })
        .unwrap_or_default();
    memory
        .connections
        .get(connection)
        .map(|c| c.remote.clone())
        .unwrap_or_default()
        .iter()
        .map(|remote| {
            remote_account(
                &ctx,
                connection,
                remote,
                linked.get(&remote.id).map(String::as_str),
            )
        })
        .collect()
}

/// `GET /api/sync/connections/{id}/accounts`: a balances-only fetch (one
/// request), then every account with suggestions.
///
/// # Errors
/// `404` for an unknown connection; the problem as `422`.
pub(crate) async fn remote_accounts(
    state: &AppState,
    id: &str,
) -> Result<Vec<WireRemoteAccount>, SyncReply> {
    let reader = state.clone();
    let wanted = id.to_string();
    let access = blocking(move || {
        let (config, _) = read_config(&reader)?;
        if config.connection(&wanted).is_none() {
            return Err(AppError::NotFound(format!("no connection {wanted:?}")));
        }
        Ok(access_for(&reader, &wanted))
    })
    .await?;
    let access = access?.ok_or_else(|| WireProblem::not_connected(id))?;
    engine::fetch_balances(state, id, &access, super::now_seconds()).await?;
    let reader = state.clone();
    let wanted = id.to_string();
    Ok(blocking(move || Ok(accounts_for(&reader, &wanted))).await?)
}

// ===========================================================================
// Link requests
// ===========================================================================

/// `{type, openingBalance, openingDate}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NewAccount {
    #[serde(rename = "type")]
    kind: String,
    opening_balance: String,
    opening_date: String,
}

/// Which rules file a link reads with.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum RulesChoice {
    New { name: String },
    Existing { id: String },
}

/// `{manualRulesId}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ShareChoice {
    manual_rules_id: String,
}

/// One `WireLinkRequest`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LinkRequest {
    connection_id: String,
    remote_id: String,
    label: String,
    journal_account: String,
    #[serde(default)]
    new_account: Option<NewAccount>,
    start_date: String,
    target: String,
    balance_sign: i8,
    rules: RulesChoice,
    #[serde(default)]
    share: Option<ShareChoice>,
}

/// An ISO date, checked.
fn iso_date(text: &str) -> Option<String> {
    let ok = text.len() == 10
        && text.as_bytes()[4] == b'-'
        && text.as_bytes()[7] == b'-'
        && iso_from_days(days_from_iso(text)) == text;
    ok.then(|| text.to_string())
}

/// An account name a link may write: hledger's account syntax, no comment or
/// amount separators, no control characters.
fn account_name_ok(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 512
        && name == name.trim()
        && !name.contains("  ")
        && !name.contains('\t')
        && !name.contains(';')
        && !name.chars().any(char::is_control)
}

/// The categories file name for a manual rules file:
/// `chase.csv.rules` → `chase.categories.rules`.
fn categories_name(manual_id: &str) -> String {
    let file = manual_id.rsplit('/').next().unwrap_or(manual_id);
    let stem = file
        .strip_suffix(".csv.rules")
        .or_else(|| file.strip_suffix(".rules"))
        .unwrap_or(file);
    format!("{stem}.categories.rules")
}

/// `id`'s directory part, with a trailing `/`, or `""`.
fn dir_of(id: &str) -> &str {
    id.rfind('/').map_or("", |at| &id[..=at])
}

/// A link request, checked against the open journal.
struct CheckedLink {
    request: LinkRequest,
    /// The rules file's id after any share placement.
    rules_id: String,
    /// The rules text the sync will read (new or existing; with the include
    /// when shared).
    rules_text: String,
    /// The categories file's text, when a share creates one.
    share: Option<PlannedShare>,
    columns: FeedColumns,
    fallback: String,
    /// For a currency URL with no fetched symbol.
    currency_warning: Option<WireProblem>,
}

/// A share to carry out.
struct PlannedShare {
    manual_id: String,
    categories_id: String,
    plan: SharePlan,
}

/// The commodity synced amounts should carry (lead decision 3): the one
/// most used on the account already; else the feed's code, as `$` when the
/// journal is mostly `$` and the code is USD; else the code. A custom
/// currency is `?` (its symbol, fetched from the connection's host, would be
/// better; the fake and an unreachable host both land here) with a warning.
fn pick_currency(
    journal: &Journal,
    account: &str,
    remote: Option<&RemoteSummary>,
) -> (Option<String>, Option<WireProblem>) {
    let prefix = format!("{account}:");
    let mut on_account: BTreeMap<String, usize> = BTreeMap::new();
    let mut overall: BTreeMap<String, usize> = BTreeMap::new();
    for posting in journal.transactions.iter().flat_map(|txn| &txn.postings) {
        for amount in &posting.amounts {
            *overall.entry(amount.commodity.0.clone()).or_default() += 1;
            if posting.account.0 == account || posting.account.0.starts_with(&prefix) {
                *on_account.entry(amount.commodity.0.clone()).or_default() += 1;
            }
        }
    }
    let most = |counts: &BTreeMap<String, usize>| {
        counts
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(commodity, _)| commodity.clone())
    };
    if let Some(commodity) = most(&on_account) {
        return (Some(commodity).filter(|c| !c.is_empty()), None);
    }
    let Some(remote) = remote else {
        return (most(&overall).filter(|c| !c.is_empty()), None);
    };
    if remote.currency_url.is_some() && remote.currency != "?" && !remote.currency.is_empty() {
        // The symbol fetched from the connection's own host.
        return (Some(remote.currency.clone()), None);
    }
    if remote.currency_url.is_some() {
        return (
            Some("?".to_string()),
            Some(WireProblem::other(
                "This account uses a custom currency (points or miles), so its amounts are written \
                 with the commodity \"?\". Rename it in the rules file's currency line.",
                Severity::Warning,
            )),
        );
    }
    let code = remote.currency.trim();
    if code.is_empty() {
        return (most(&overall).filter(|c| !c.is_empty()), None);
    }
    if code == "USD" && most(&overall).as_deref() == Some("$") {
        return (Some("$".to_string()), None);
    }
    let safe = code.chars().all(|c| c.is_ascii_alphabetic()) && code.len() <= 8;
    (safe.then(|| code.to_string()), None)
}

/// Whether a new rules file may be created at `id`, and the folder to create
/// first if its folder doesn't exist yet: `Discovery::resolve_new` agrees, or
/// refuses only because the folder is missing and that folder resolves inside
/// the journal's (checked through any symlinks before anything is created).
fn new_rules_folder(
    ctx: &Context,
    discovery: &Discovery,
    id: &str,
) -> Result<Option<ledgeline_core::vault::VaultPath>, WireProblem> {
    let refused = || {
        WireProblem::setup(format!(
            "{id} already exists, isn't a .rules file, or isn't inside the journal's folder."
        ))
    };
    match discovery.resolve_new(id) {
        Ok(_) => Ok(None),
        Err(ledgeline_core::rules::CreateRefusal::DirectoryMissing) => {
            let location = ctx.location.as_ref().ok_or_else(refused)?;
            let root = location.root_dir();
            let folder = id
                .rsplit_once('/')
                .map(|(dir, _)| dir)
                .ok_or_else(refused)?;
            let ok_parts = folder.split('/').all(|part| {
                !part.is_empty() && part != "." && part != ".." && !part.starts_with('.')
            });
            let dir = root
                .join(folder)
                .ok()
                .filter(|_| ok_parts)
                .and_then(|dir| {
                    ledgeline_core::vault::canonicalize_existing_prefix(&**location.vault(), &dir)
                        .ok()
                })
                .filter(|dir| dir.starts_with(&root))
                .ok_or_else(refused)?;
            Ok(Some(dir))
        }
        Err(_) => Err(refused()),
    }
}

/// `share::append_include`, but kept ahead of a feed template's closing id
/// guard (`if %id .` / `comment id:%id`): the guard must stay the last rule,
/// or a comment rule in the included categories would erase synced ids again.
fn include_before_guard(text: &str, categories: &str) -> Result<String, WireProblem> {
    let fields = ledgeline_core::rules::RulesDoc::parse(text)
        .settings()
        .fields
        .map(|setting| setting.value)
        .unwrap_or_default();
    let guard = fields
        .get(5)
        .map(|id| format!("if %{id} .\n  comment id:%{id}\n"))
        .filter(|guard| text.ends_with(guard.as_str()));
    let appended = |body: &str| {
        share::append_include(body, categories)
            .map_err(|error| WireProblem::setup(error.to_string()))
    };
    match guard {
        Some(guard) => Ok(appended(&text[..text.len() - guard.len()])? + &guard),
        None => appended(text),
    }
}

/// Check one link request, plan its rules (and share), but write nothing.
fn check(
    state: &AppState,
    ctx: &Context,
    request: LinkRequest,
) -> Result<CheckedLink, WireProblem> {
    let discovery = ctx
        .discovery
        .as_ref()
        .ok_or_else(|| WireProblem::setup("No journal file is open."))?;
    if !account_name_ok(&request.journal_account) {
        return Err(WireProblem::setup(format!(
            "{:?} isn't an account name Ledgeline can write.",
            request.journal_account
        )));
    }
    if iso_date(&request.start_date).is_none() {
        return Err(WireProblem::setup(
            "The start date isn't a date (YYYY-MM-DD).",
        ));
    }
    if !matches!(request.balance_sign, 1 | -1) {
        return Err(WireProblem::setup("The balance sign must be 1 or -1."));
    }
    if !ctx
        .targets
        .iter()
        .any(|target| target.id == request.target && target.writable)
    {
        return Err(WireProblem::target(&request.target, ""));
    }
    if let Some(new) = &request.new_account
        && (!matches!(new.kind.as_str(), "A" | "L" | "C")
            || iso_date(&new.opening_date).is_none()
            || (!new.opening_balance.is_empty()
                && !ledgeline_core::feeds::is_decimal_text(&new.opening_balance)))
    {
        return Err(WireProblem::setup(
            "The new account's type, opening balance or date isn't usable.",
        ));
    }
    let memory = read_state(state);
    let remote = memory
        .connections
        .get(&request.connection_id)
        .and_then(|c| c.remote.iter().find(|r| r.id == request.remote_id))
        .cloned();
    let (currency, currency_warning) =
        pick_currency(&ctx.journal, &request.journal_account, remote.as_ref());
    let share = match &request.share {
        Some(share) => {
            let manual = discovery.resolve(&share.manual_rules_id).ok_or_else(|| {
                WireProblem::setup("The rules file to share from isn't there anymore.")
            })?;
            let text = manual.path().read_text().ok_or_else(|| {
                WireProblem::setup("The rules file to share from couldn't be read.")
            })?;
            let name = categories_name(&share.manual_rules_id);
            let plan = plan_share(&text, &name).map_err(|error| {
                WireProblem::setup(format!("Its rules can't be shared: {error}."))
            })?;
            let categories_id = format!("{}{name}", dir_of(&share.manual_rules_id));
            discovery.resolve_new(&categories_id).map_err(|_| {
                WireProblem::setup(format!(
                    "{categories_id} already exists or can't be created."
                ))
            })?;
            Some(PlannedShare {
                manual_id: share.manual_rules_id.clone(),
                categories_id,
                plan,
            })
        }
        None => None,
    };
    let (rules_id, rules_text) = match &request.rules {
        RulesChoice::New { name } => {
            // A shared file's include names a sibling, so the new sync file
            // goes beside the manual file it shares with.
            let id = match &share {
                Some(share) => format!(
                    "{}{}",
                    dir_of(&share.manual_id),
                    name.rsplit('/').next().unwrap_or(name)
                ),
                None => name.clone(),
            };
            new_rules_folder(ctx, discovery, &id)?;
            let columns = share
                .as_ref()
                .map_or_else(FeedColumns::default, |share| share.plan.columns.clone());
            let categories = share
                .as_ref()
                .map(|share| categories_name(&share.manual_id));
            let text = feed_rules(&FeedRulesParams {
                label: &sanitize(&request.label),
                provider: "SimpleFIN",
                columns: &columns,
                journal_account: &request.journal_account,
                fallback_account: DEFAULT_ACCOUNT2,
                categories_include: categories.as_deref(),
                currency: currency.as_deref(),
            });
            (id, text)
        }
        RulesChoice::Existing { id } => {
            let found = discovery.resolve(id).ok_or_else(|| {
                WireProblem::setup(format!("The rules file {id} isn't there anymore."))
            })?;
            let text = found.path().read_text().ok_or_else(|| {
                WireProblem::setup(format!("The rules file {id} couldn't be read."))
            })?;
            let text = match &share {
                Some(share) if dir_of(id) == dir_of(&share.manual_id) => {
                    include_before_guard(&text, &categories_name(&share.manual_id))?
                }
                Some(_) => {
                    return Err(WireProblem::setup(
                        "To share categories, the sync rules file has to be in the same folder as \
                         the rules file it shares with.",
                    ));
                }
                None => text,
            };
            (id.clone(), text)
        }
    };
    let columns = engine::columns_of(&rules_text);
    let fallback = ledgeline_core::rules::RulesDoc::parse(&rules_text)
        .settings()
        .account2
        .map(|setting| setting.value)
        .unwrap_or_else(|| DEFAULT_FALLBACK.to_string());
    Ok(CheckedLink {
        request,
        rules_id,
        rules_text,
        share,
        columns,
        fallback,
        currency_warning,
    })
}

/// Transactions for each connection a set of links names, enough for every
/// link's start date: a fresh cached batch when it covers them... always
/// fetched with transactions here (a cached batch is balances only).
async fn batches_for(
    state: &AppState,
    links: &[CheckedLink],
    now: i64,
) -> HashMap<String, Result<FeedBatch, WireProblem>> {
    let mut out = HashMap::new();
    let mut by_connection: BTreeMap<String, Vec<&CheckedLink>> = BTreeMap::new();
    for link in links {
        by_connection
            .entry(link.request.connection_id.clone())
            .or_default()
            .push(link);
    }
    for (connection, links) in by_connection {
        let reader = state.clone();
        let wanted = connection.clone();
        let access = blocking(move || Ok(access_for(&reader, &wanted)))
            .await
            .unwrap_or_else(|error| Err(WireProblem::setup(error.to_string())));
        let access = match access {
            Ok(Some(access)) => access,
            Ok(None) => {
                out.insert(
                    connection.clone(),
                    Err(WireProblem::not_connected(&connection)),
                );
                continue;
            }
            Err(problem) => {
                out.insert(connection, Err(problem));
                continue;
            }
        };
        let offset = super::local_offset(now);
        let start = links
            .iter()
            .map(|link| {
                days_from_iso(&link.request.start_date) * 86_400 - i64::from(offset) - 86_400
            })
            .min()
            .unwrap_or(now);
        let end =
            days_from_iso(&auto::local_day(now, offset)) * 86_400 - i64::from(offset) + 86_400;
        let accounts: Vec<String> = links.iter().map(|l| l.request.remote_id.clone()).collect();
        if let Some(cached) = state.sync().fresh_batch(&connection, now, start, &accounts) {
            out.insert(connection, Ok(cached));
            continue;
        }
        let mut merged: Option<FeedBatch> = None;
        let mut failed = None;
        for window in auto::chunks(start, end) {
            let count = engine::count_request(state, &connection, now);
            if count > auto::MANUAL_MAX {
                failed = Some(WireProblem::quota_refused(count - 1));
                break;
            }
            let query = super::AccountsQuery {
                start_date: Some(window.start),
                end_date: Some(window.end),
                balances_only: false,
                accounts: accounts.clone(),
            };
            match state.sync().provider().accounts(&access, &query).await {
                Ok(batch) => {
                    merged = Some(match merged {
                        None => batch,
                        Some(mut earlier) => {
                            for account in batch.accounts {
                                match earlier.accounts.iter_mut().find(|a| a.id == account.id) {
                                    Some(existing) => {
                                        let seen: HashSet<String> = existing
                                            .transactions
                                            .iter()
                                            .map(|t| t.id.clone())
                                            .collect();
                                        existing.transactions.extend(
                                            account
                                                .transactions
                                                .into_iter()
                                                .filter(|t| !seen.contains(&t.id)),
                                        );
                                    }
                                    None => earlier.accounts.push(account),
                                }
                            }
                            earlier
                        }
                    });
                }
                Err(error) => {
                    failed = Some(engine::feed_error_problem(
                        &error,
                        &connection,
                        &access.host,
                        now,
                        now,
                    ));
                    break;
                }
            }
        }
        let answer = match failed {
            Some(problem) => Err(problem),
            None => {
                let batch = merged.unwrap_or_default();
                state
                    .sync()
                    .remember_batch(&connection, now, start, batch.clone());
                Ok(batch)
            }
        };
        out.insert(connection, answer);
    }
    out
}

/// `POST /api/sync/link/preview`: each link's first sync as a dry run. Writes
/// nothing to the journal's folder: the rules (and a shared categories file)
/// are run by hledger from a private scratch folder.
///
/// # Errors
/// A transport failure of the blocking pool.
pub(crate) async fn preview(
    state: &AppState,
    requests: Vec<LinkRequest>,
) -> Result<Vec<WireLinkPreview>, SyncReply> {
    let reader = state.clone();
    let checked: Vec<(String, Result<CheckedLink, WireProblem>)> = blocking(move || {
        let snapshot = reader.snapshot();
        let ctx = Context::of(&reader, &snapshot);
        Ok(requests
            .into_iter()
            .map(|request| (request.remote_id.clone(), check(&reader, &ctx, request)))
            .collect())
    })
    .await?;
    let now = super::now_seconds();
    let usable: Vec<CheckedLink> = checked
        .iter()
        .filter_map(|(_, checked)| checked.as_ref().ok())
        .map(|link| CheckedLink {
            request: link.request.clone(),
            rules_id: link.rules_id.clone(),
            rules_text: link.rules_text.clone(),
            share: None,
            columns: link.columns.clone(),
            fallback: link.fallback.clone(),
            currency_warning: None,
        })
        .collect();
    let batches = batches_for(state, &usable, now).await;
    let worker = state.clone();
    Ok(blocking(move || {
        let snapshot = worker.snapshot();
        let ctx = Context::of(&worker, &snapshot);
        Ok(checked
            .into_iter()
            .map(|(remote_id, checked)| match checked {
                Err(problem) => failed_preview(remote_id, problem),
                Ok(link) => match batches.get(&link.request.connection_id) {
                    Some(Ok(batch)) => preview_one(&worker, &ctx, &link, batch, now),
                    Some(Err(problem)) => failed_preview(remote_id, problem.clone()),
                    None => failed_preview(
                        remote_id,
                        WireProblem::other("Nothing was fetched.", Severity::Error),
                    ),
                },
            })
            .collect())
    })
    .await?)
}

fn failed_preview(remote_id: String, problem: WireProblem) -> WireLinkPreview {
    WireLinkPreview {
        remote_id,
        ok: false,
        error: Some(problem),
        new: 0,
        already: 0,
        held: 0,
        uncategorized: 0,
        dropped_before_start: 0,
        entries: String::new(),
    }
}

/// One link's dry run over `batch`.
fn preview_one(
    state: &AppState,
    ctx: &Context,
    link: &CheckedLink,
    batch: &FeedBatch,
    now: i64,
) -> WireLinkPreview {
    let request = &link.request;
    let Some(account) = batch.accounts.iter().find(|a| a.id == request.remote_id) else {
        return failed_preview(
            request.remote_id.clone(),
            WireProblem::other("SimpleFIN didn't return this account.", Severity::Warning),
        );
    };
    let key =
        ledgeline_core::feeds::normalize::account_short(&request.connection_id, &request.remote_id);
    let normalized = normalize(
        account,
        &NormalizeOptions {
            start_date: &request.start_date,
            account_short: &key,
            columns: &link.columns,
            utc_offset_seconds: super::local_offset(now),
        },
    );
    let dropped = normalized.dropped_before_start;
    let known = dupes::known_ids(&ctx.raw);
    let split = dupes::split(
        normalized.rows,
        &ctx.raw,
        &request.journal_account,
        &known,
        &fetched_ids(account, &key),
    );
    let mut preview = WireLinkPreview {
        remote_id: request.remote_id.clone(),
        ok: true,
        error: link.currency_warning.clone(),
        new: split.fresh.len(),
        already: split.already.len(),
        held: split.held.len(),
        uncategorized: 0,
        dropped_before_start: dropped,
        entries: String::new(),
    };
    if split.fresh.is_empty() {
        return preview;
    }
    match print_with(state, ctx, link, &to_csv(&split.fresh, &link.columns)) {
        Ok(entries) => {
            preview.uncategorized = ledgeline_core::parse_journal(&entries, "preview.journal")
                .map(|journal| {
                    journal
                        .transactions
                        .iter()
                        .filter(|txn| txn.postings.iter().any(|p| p.account.0 == link.fallback))
                        .count()
                })
                .unwrap_or(0);
            preview.new = ledgeline_core::parse_journal(&entries, "preview.journal")
                .map_or(preview.new, |journal| journal.transactions.len());
            preview.entries = entries;
        }
        Err(problem) => {
            preview.ok = false;
            preview.error = Some(problem);
        }
    }
    preview
}

/// `hledger print` of `csv` with the link's rules (and any categories file it
/// includes), from a private scratch folder.
fn print_with(
    state: &AppState,
    ctx: &Context,
    link: &CheckedLink,
    csv: &str,
) -> Result<String, WireProblem> {
    let hledger = crate::import_api::resolve_hledger(state.app_dirs())
        .map_err(|error| WireProblem::hledger(&error.to_string()))?;
    let scratch = state.stages().scratch().map_err(|error| {
        WireProblem::other(
            format!("no scratch folder ({})", error.kind()),
            Severity::Error,
        )
    })?;
    let dir = scratch.path();
    let write = |name: &str, text: &str| {
        std::fs::write(dir.join(name), text).map_err(|error| {
            WireProblem::other(
                format!("scratch write failed ({})", error.kind()),
                Severity::Error,
            )
        })
    };
    write("sync.csv", csv)?;
    write("sync.csv.rules", &link.rules_text)?;
    // Every sibling include the rules name, from the plan or the folder.
    for name in link
        .rules_text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("include "))
        .map(str::trim)
        .filter(|name| share::check_include_name(name).is_ok())
    {
        let text = match &link.share {
            Some(share) if categories_name(&share.manual_id) == name => {
                Some(share.plan.categories_text.clone())
            }
            _ => ctx
                .discovery
                .as_ref()
                .and_then(|d| d.resolve(&format!("{}{name}", dir_of(&link.rules_id))))
                .and_then(|file| file.path().read_text()),
        };
        write(name, &text.unwrap_or_default())?;
    }
    let output = hledger
        .invoke(["-f".as_ref(), dir.join("sync.csv").as_os_str()])
        .arg("--rules")
        .arg(dir.join("sync.csv.rules"))
        .arg("print")
        .run()
        .map_err(|error| WireProblem::hledger(&error.to_string()))?;
    if output.success() {
        Ok(output.stdout_lossy())
    } else {
        let redactor = crate::redact::Redactor::default().hide_prefix(dir);
        Err(WireProblem::rules(
            &link.rules_id,
            &redactor.apply(&output.stderr_lossy()),
        ))
    }
}

/// `POST /api/sync/link`: write each link's rules (and share), `[sync]`
/// entries, new account declarations and opening balances, then run the
/// first sync of the accounts just linked.
///
/// # Errors
/// The first link that can't be checked or written, as a `422` problem; the
/// links before it stay written.
pub(crate) async fn link(
    state: &AppState,
    requests: Vec<LinkRequest>,
) -> Result<WireSyncRun, SyncReply> {
    let mut keys = Vec::new();
    let mut connections: HashSet<String> = HashSet::new();
    let mut warnings = Vec::new();
    for request in requests {
        let _rules = state.rules_writes().lock().await;
        let _imports = state.import_writes().lock().await;
        let worker = state.clone();
        let written =
            blocking(move || {
                let snapshot = worker.snapshot();
                let ctx = Context::of(&worker, &snapshot);
                Ok(check(&worker, &ctx, request)
                    .and_then(|checked| write_link(&worker, &ctx, checked)))
            })
            .await?;
        let (key, connection, warning) = written?;
        keys.push(key);
        connections.insert(connection);
        warnings.extend(warning);
    }
    let mut run = WireSyncRun {
        ok: true,
        requests_today: 0,
        quota_warning: false,
        accounts: Vec::new(),
        problems: warnings,
    };
    for connection in connections {
        let ran = engine::run(
            state,
            super::now_seconds(),
            &engine::Request {
                mode: Mode::Manual,
                connection: Some(connection),
                accounts: keys.clone(),
            },
        )
        .await;
        run.ok &= ran.ok;
        run.requests_today = run.requests_today.max(ran.requests_today);
        run.quota_warning |= ran.quota_warning;
        run.accounts.extend(ran.accounts);
        run.problems.extend(ran.problems);
    }
    Ok(run)
}

/// Carry out one checked link. Every file write before a failure is rolled
/// back.
fn write_link(
    state: &AppState,
    ctx: &Context,
    checked: CheckedLink,
) -> Result<(String, String, Option<WireProblem>), WireProblem> {
    let discovery = ctx
        .discovery
        .as_ref()
        .ok_or_else(|| WireProblem::setup("No journal file is open."))?;
    let request = &checked.request;
    let vault = ctx
        .location
        .as_ref()
        .map(|location| Arc::clone(location.vault()))
        .ok_or_else(|| WireProblem::setup("No journal file is open."))?;
    let failed = |what: &str, error: &dyn std::fmt::Display| {
        WireProblem::setup(format!("Couldn't write {what}: {error}."))
    };
    let mut undo: Vec<Box<dyn FnOnce()>> = Vec::new();
    let rollback = |undo: Vec<Box<dyn FnOnce()>>| undo.into_iter().rev().for_each(|step| step());

    // 1. The share: categories first (new), then the manual file (pinned).
    if let Some(share) = &checked.share {
        if let Some(differences) = identity_check(state, ctx, &share.manual_id, &share.plan)? {
            return Err(WireProblem::setup(format!(
                "Sharing would change what the manual rules import, so nothing was changed: {differences}"
            )));
        }
        let categories = discovery
            .resolve_new(&share.categories_id)
            .map_err(|_| WireProblem::setup(format!("{} already exists.", share.categories_id)))?;
        vault
            .create_new_shared(
                categories.vault_path(),
                share.plan.categories_text.as_bytes(),
            )
            .map_err(|error| failed(&share.categories_id, &error.kind()))?;
        let created = categories.vault_path().clone();
        let v = Arc::clone(&vault);
        undo.push(Box::new(move || {
            let _ = v.remove_file(&created);
        }));
        let manual = discovery
            .resolve(&share.manual_id)
            .ok_or_else(|| WireProblem::setup("The manual rules file went away."))?;
        let path = manual.path().vault_path().clone();
        let (before, pin) = match vault.read_pinned(&path, None) {
            Ok(read) => read,
            Err(error) => {
                rollback(undo);
                return Err(failed(&share.manual_id, &error.kind()));
            }
        };
        let planned = manual.path().read_text().unwrap_or_default();
        if before != planned.as_bytes() {
            rollback(undo);
            return Err(WireProblem::setup(format!(
                "{} changed while sharing; try again.",
                share.manual_id
            )));
        }
        if let Err(error) = vault.replace(&path, share.plan.manual_new_text.as_bytes(), &pin) {
            rollback(undo);
            return Err(failed(&share.manual_id, &error.kind()));
        }
        let v = Arc::clone(&vault);
        undo.push(Box::new(move || {
            let _ = v.write_atomic(&path, &before);
        }));
    }

    // 2. The sync rules file.
    match &request.rules {
        RulesChoice::New { .. } => {
            match new_rules_folder(ctx, discovery, &checked.rules_id) {
                Ok(Some(folder)) => {
                    if let Err(error) = vault.create_dir_all(&folder) {
                        rollback(undo);
                        return Err(failed(&checked.rules_id, &error.kind()));
                    }
                }
                Ok(None) => {}
                Err(problem) => {
                    rollback(undo);
                    return Err(problem);
                }
            }
            // A new folder: scan again so `resolve_new` sees it.
            let rescanned;
            let discovery = match ctx.location.as_ref() {
                Some(location) => {
                    rescanned = ledgeline_core::rules::discover_in(
                        Arc::clone(location.vault()),
                        location.main(),
                    );
                    &rescanned
                }
                None => discovery,
            };
            let rules = match discovery.resolve_new(&checked.rules_id) {
                Ok(rules) => rules,
                Err(_) => {
                    rollback(undo);
                    return Err(WireProblem::setup(format!(
                        "{} already exists.",
                        checked.rules_id
                    )));
                }
            };
            if let Err(error) =
                vault.create_new_shared(rules.vault_path(), checked.rules_text.as_bytes())
            {
                rollback(undo);
                return Err(failed(&checked.rules_id, &error.kind()));
            }
            let created = rules.vault_path().clone();
            let v = Arc::clone(&vault);
            undo.push(Box::new(move || {
                let _ = v.remove_file(&created);
            }));
        }
        RulesChoice::Existing { id } if checked.share.is_some() => {
            let Some(found) = discovery.resolve(id) else {
                rollback(undo);
                return Err(WireProblem::setup(format!("{id} went away.")));
            };
            let path = found.path().vault_path().clone();
            match vault.read_pinned(&path, None) {
                Ok((before, pin)) => {
                    if let Err(error) = vault.replace(&path, checked.rules_text.as_bytes(), &pin) {
                        rollback(undo);
                        return Err(failed(id, &error.kind()));
                    }
                    let v = Arc::clone(&vault);
                    undo.push(Box::new(move || {
                        let _ = v.write_atomic(&path, &before);
                    }));
                }
                Err(error) => {
                    rollback(undo);
                    return Err(failed(id, &error.kind()));
                }
            }
        }
        RulesChoice::Existing { .. } => {}
    }

    // 3. A new account's declaration and opening balance, in the target.
    if let Some(new) = &request.new_account
        && let Err(problem) = declare_account(state, ctx, request, new)
    {
        rollback(undo);
        return Err(problem);
    }

    // 4. `[sync]`.
    let link = AccountLink {
        connection: request.connection_id.clone(),
        remote_id: request.remote_id.clone(),
        label: sanitize(&request.label),
        journal_account: request.journal_account.clone(),
        rules: checked.rules_id.clone(),
        target: request.target.clone(),
        start_date: request.start_date.clone(),
        balance_sign: request.balance_sign,
        enabled: true,
    };
    if let Err(error) = write_config(state, |text| feed_config::upsert_account(text, &link)) {
        rollback(undo);
        return Err(WireProblem::setup(error.to_string()));
    }
    Ok((link.key(), link.connection, checked.currency_warning))
}

/// Append `account NAME  ; type:X` and, for a non-zero opening balance, an
/// opening-balance transaction to the link's target file, then re-read the
/// journal. Skips the declaration when the journal already declares it.
fn declare_account(
    state: &AppState,
    ctx: &Context,
    request: &LinkRequest,
    new: &NewAccount,
) -> Result<(), WireProblem> {
    let location = ctx
        .location
        .as_ref()
        .ok_or_else(|| WireProblem::setup("No journal file is open."))?;
    let path = location
        .source_path(&ctx.raw, &request.target)
        .ok_or_else(|| WireProblem::target(&request.target, ""))?;
    let account = &request.journal_account;
    let mut text = String::new();
    if !ctx.raw.accounts.iter().any(|decl| &decl.name.0 == account) {
        text.push_str(&format!("\naccount {account}  ; type:{}\n", new.kind));
    }
    let opening = Dec::parse(&new.opening_balance, '.')
        .ok()
        .filter(|d| !d.is_zero());
    if let Some(quantity) = opening {
        let (currency, _) = pick_currency(&ctx.journal, account, None);
        let rendered = render_in(&ctx.journal, currency.as_deref().unwrap_or(""), quantity);
        text.push_str(&format!(
            "\n{} Opening balance\n    {account}  {rendered}\n    equity:opening balances\n",
            new.opening_date
        ));
    }
    if text.is_empty() {
        return Ok(());
    }
    ledgeline_core::parse_journal(&text, "opening.journal")
        .map_err(|_| WireProblem::setup("The new account's lines wouldn't parse."))?;
    let vault = location.vault();
    let (before, pin) = vault.read_pinned(&path, None).map_err(|error| {
        WireProblem::target(&request.target, "").detail(error.kind().to_string())
    })?;
    let mut after = before.clone();
    if !after.is_empty() && !after.ends_with(b"\n") {
        after.push(b'\n');
    }
    after.extend_from_slice(text.as_bytes());
    vault.replace(&path, &after, &pin).map_err(|error| {
        WireProblem::target(&request.target, "").detail(error.kind().to_string())
    })?;
    if let Some(Err(error)) = state.reopen_editor() {
        let _ = vault.write_atomic(&path, &before);
        let _ = state.reopen_editor();
        return Err(WireProblem::setup(format!(
            "The journal didn't read back after adding the account: {error}"
        )));
    }
    Ok(())
}

/// `quantity` in `commodity`, styled like the journal writes it.
fn render_in(journal: &Journal, commodity: &str, quantity: Dec) -> String {
    let sample: Option<&Amount> = journal
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .flat_map(|p| &p.amounts)
        .find(|a| a.commodity.0 == commodity);
    match sample {
        // Journal text, so the editor's renderer (no digit groups).
        Some(sample) => ledgeline_core::edit::render_amount(&Amount {
            quantity,
            cost: None,
            ..sample.clone()
        }),
        None if commodity.is_empty() => format!("{}", quantity.floating_point()),
        None if commodity.chars().count() == 1 => format!("{commodity}{}", plain(quantity)),
        None => format!("{} {commodity}", plain(quantity)),
    }
}

/// A decimal as plain text with at least two places.
fn plain(quantity: Dec) -> String {
    let places = quantity.places.max(2);
    let scaled = quantity.rounded(places).unwrap_or(quantity);
    let negative = scaled.mantissa < 0;
    let digits = scaled.mantissa.unsigned_abs().to_string();
    let places = scaled.places as usize;
    let padded = format!("{digits:0>width$}", width = places + 1);
    let (whole, fraction) = padded.split_at(padded.len() - places);
    format!("{}{whole}.{fraction}", if negative { "-" } else { "" })
}

// ===========================================================================
// Sharing
// ===========================================================================

/// hledger's `print` of the manual file's own last saved CSV, before and
/// after the split; `Some(differences)` when they differ. `Ok(None)` when
/// they match, or when there is no CSV to check against (the split is a byte
/// partition, property-tested in core).
fn identity_check(
    state: &AppState,
    ctx: &Context,
    manual_id: &str,
    plan: &SharePlan,
) -> Result<Option<String>, WireProblem> {
    let Some(discovery) = ctx.discovery.as_ref() else {
        return Ok(None);
    };
    let Some(location) = ctx.location.as_ref() else {
        return Ok(None);
    };
    let Some(data) = discovery.data_file(manual_id) else {
        return Ok(None);
    };
    let Some(csv) = location.vault().local_path(&data) else {
        return Ok(None);
    };
    let Some(manual) = discovery
        .resolve(manual_id)
        .and_then(|f| f.path().local_path())
    else {
        return Ok(None);
    };
    let hledger = crate::import_api::resolve_hledger(state.app_dirs())
        .map_err(|error| WireProblem::hledger(&error.to_string()))?;
    let print = |rules: &std::path::Path| {
        hledger
            .invoke(["-f".as_ref(), csv.as_os_str()])
            .arg("--rules")
            .arg(rules)
            .arg("print")
            .run()
            .map(|output| {
                (
                    output.success(),
                    output.stdout_lossy(),
                    output.stderr_lossy(),
                )
            })
            .map_err(|error| WireProblem::hledger(&error.to_string()))
    };
    let before = print(&manual)?;
    let scratch = state.stages().scratch().map_err(|error| {
        WireProblem::other(
            format!("no scratch folder ({})", error.kind()),
            Severity::Error,
        )
    })?;
    let file_name = manual_id.rsplit('/').next().unwrap_or(manual_id);
    let rules = scratch.path().join(file_name);
    let written = std::fs::write(&rules, &plan.manual_new_text).and_then(|()| {
        std::fs::write(
            scratch.path().join(categories_name(manual_id)),
            &plan.categories_text,
        )
    });
    written.map_err(|error| {
        WireProblem::other(
            format!("scratch write failed ({})", error.kind()),
            Severity::Error,
        )
    })?;
    let after = print(&rules)?;
    if before == after {
        Ok(None)
    } else {
        Ok(Some(first_difference(&before.1, &after.1)))
    }
}

/// The first line where two prints differ, for the dialog.
fn first_difference(before: &str, after: &str) -> String {
    before
        .lines()
        .zip(after.lines())
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .map_or_else(
            || "the two prints have different lengths".to_string(),
            |(at, (a, b))| format!("line {}: {a:?} became {b:?}", at + 1),
        )
}

/// A unified diff of the manual file's split: each moved block's lines as
/// removals, and the appended include as an addition.
fn manual_diff(original: &str, plan: &SharePlan, manual_id: &str) -> String {
    let mut out = format!("--- a/{manual_id}\n+++ b/{manual_id}\n");
    for span in &plan.moved_spans {
        let line = original[..span.start].matches('\n').count() + 1;
        let removed: Vec<&str> = original[span.clone()].lines().collect();
        out.push_str(&format!("@@ -{line},{} +{line},0 @@\n", removed.len()));
        for text in removed {
            out.push_str(&format!("-{text}\n"));
        }
    }
    let added: Vec<&str> = plan.manual_new_text[plan.include_offset..]
        .lines()
        .collect();
    let at = plan.manual_new_text[..plan.include_offset]
        .matches('\n')
        .count()
        + 1;
    out.push_str(&format!("@@ +{at},{} @@\n", added.len()));
    for text in added {
        out.push_str(&format!("+{text}\n"));
    }
    out
}

/// A unified diff creating the categories file.
fn categories_diff(text: &str, categories_id: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = format!(
        "--- /dev/null\n+++ b/{categories_id}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for line in lines {
        out.push_str(&format!("+{line}\n"));
    }
    out
}

/// `POST /api/sync/share/preview`: what sharing `manual_id`'s categorization
/// would move, keep and change, and whether hledger prints its own CSV the
/// same after.
///
/// # Errors
/// `404` for an unknown rules file; `400` when it can't be shared.
pub(crate) fn share_preview(
    state: &AppState,
    manual_id: &str,
) -> Result<WireSharePreview, AppError> {
    let snapshot = state.snapshot();
    let ctx = Context::of(state, &snapshot);
    let discovery = ctx
        .discovery
        .as_ref()
        .ok_or_else(|| AppError::Conflict("no journal file is open".to_string()))?;
    let manual = discovery
        .resolve(manual_id)
        .ok_or_else(|| AppError::NotFound(format!("no rules file {manual_id:?}")))?;
    let text = manual
        .path()
        .read_text()
        .ok_or_else(|| AppError::Conflict(format!("{manual_id} couldn't be read")))?;
    let name = categories_name(manual_id);
    let plan = plan_share(&text, &name)
        .map_err(|error| AppError::BadRequest(format!("{manual_id} can't be shared: {error}")))?;
    let categories_id = format!("{}{name}", dir_of(manual_id));
    let differences = identity_check(state, &ctx, manual_id, &plan)
        .map_err(|problem| AppError::BadRequest(problem.message))?;
    Ok(WireSharePreview {
        manual_rules_id: manual_id.to_string(),
        categories_id: categories_id.clone(),
        moved: plan.moved,
        kept: plan
            .kept
            .iter()
            .map(|kept| WireKept {
                header: kept.header.clone(),
                reason: kept.reason.to_string(),
            })
            .collect(),
        diffs: WireShareDiffs {
            manual: manual_diff(&text, &plan, manual_id),
            categories: categories_diff(&plan.categories_text, &categories_id),
        },
        identical: differences.is_none(),
        differences,
    })
}

// ===========================================================================
// Account settings and manual accounts
// ===========================================================================

/// `PATCH /api/sync/accounts/{key}`: change a link's mapping. Paths are
/// checked as a link's are.
///
/// # Errors
/// `404` for an unknown key; `400` for a value that can't be used.
pub(crate) fn patch_account(
    state: &AppState,
    key: &str,
    patch: &super::AccountPatch,
) -> Result<(), AppError> {
    let (config, _) = read_config(state)?;
    let mut link = config
        .account(key)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("no linked account {key:?}")))?;
    if let Some(account) = &patch.journal_account {
        if !account_name_ok(account) {
            return Err(AppError::BadRequest(format!(
                "{account:?} isn't an account name"
            )));
        }
        link.journal_account = account.clone();
    }
    if let Some(target) = &patch.target {
        link.target = target.clone();
    }
    if let Some(rules) = &patch.rules {
        link.rules = rules.clone();
    }
    if let Some(date) = &patch.start_date {
        link.start_date =
            iso_date(date).ok_or_else(|| AppError::BadRequest("startDate isn't a date".into()))?;
    }
    if let Some(sign) = patch.balance_sign {
        if !matches!(sign, 1 | -1) {
            return Err(AppError::BadRequest(
                "balanceSign must be 1 or -1".to_string(),
            ));
        }
        link.balance_sign = sign;
    }
    if let Some(enabled) = patch.enabled {
        link.enabled = enabled;
    }
    let snapshot = state.snapshot();
    let ctx = Context::of(state, &snapshot);
    engine::ready(&ctx, &link).map_err(|problem| AppError::BadRequest(problem.message))?;
    write_config(state, |text| feed_config::upsert_account(text, &link))?;
    if patch.balance_sign.is_some() || patch.journal_account.is_some() {
        update_state(state, |memory| {
            if let Some(account) = memory.accounts.get_mut(key) {
                account.balance_diff = None;
                account.ledger_balance = None;
                account
                    .problems
                    .retain(|p| p.code != super::wire::ProblemCode::Balance);
            }
        });
    }
    Ok(())
}

/// Days without a new transaction before a nudged manual account is stale.
const MANUAL_STALE_DAYS: i64 = 35;

/// `GET /api/sync/manual`: the journal's asset and liability accounts that no
/// link names (by declared type, falling back to the name), with their last
/// transaction and the `[[sync.manual]]` settings.
///
/// # Errors
/// An unreadable `ledgeline.toml`.
pub(crate) fn manual_accounts(
    state: &AppState,
    now: i64,
) -> Result<Vec<WireManualAccount>, AppError> {
    let (config, _) = read_config(state)?;
    let snapshot = state.snapshot();
    let linked: HashSet<&str> = config
        .accounts
        .iter()
        .map(|a| a.journal_account.as_str())
        .collect();
    let today = days_from_iso(&super::iso_utc(now));
    let settings: HashMap<&str, &feed_config::ManualAccount> = config
        .manual
        .iter()
        .map(|m| (m.account.as_str(), m))
        .collect();
    let mut last: HashMap<&str, &str> = HashMap::new();
    for txn in &snapshot.raw.transactions {
        for posting in &txn.postings {
            let entry = last
                .entry(posting.account.0.as_str())
                .or_insert(txn.date.as_str());
            if txn.date.as_str() > *entry {
                *entry = txn.date.as_str();
            }
        }
    }
    Ok(balance_sheet_accounts(&snapshot.journal)
        .into_iter()
        .filter(|account| !linked.contains(account.as_str()))
        .filter(|account| {
            last.contains_key(account.as_str()) || settings.contains_key(account.as_str())
        })
        .map(|account| {
            let manual = settings.get(account.as_str());
            let last_transaction = last.get(account.as_str()).map(|d| (*d).to_string());
            let nudge = manual.is_none_or(|m| m.nudge);
            let stale = nudge
                && last_transaction
                    .as_deref()
                    .is_some_and(|date| today - days_from_iso(date) > MANUAL_STALE_DAYS);
            WireManualAccount {
                download_url: manual.and_then(|m| m.download_url.clone()),
                last_import: None,
                last_transaction,
                nudge,
                stale,
                account,
            }
        })
        .collect())
}

#[allow(dead_code)]
fn _manage(host: &str) -> String {
    manage_url(Some(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(name: &str, org: Option<&str>, balance: &str) -> RemoteSummary {
        RemoteSummary {
            id: "ACT-1".into(),
            name: name.into(),
            org: org.map(str::to_string),
            balance: balance.into(),
            currency: "USD".into(),
            ..RemoteSummary::default()
        }
    }

    #[test]
    fn last_four_finds_the_masked_digits() {
        assert_eq!(last_four("Checking ••1234").as_deref(), Some("1234"));
        assert_eq!(last_four("Card x987654").as_deref(), Some("7654"));
        assert_eq!(last_four("Savings"), None);
        assert_eq!(last_four("Acct 12"), None);
    }

    #[test]
    fn new_account_names_follow_the_kind() {
        assert_eq!(
            new_account_name(&remote("Savings ••0077", Some("Ally"), "20000.00")),
            ("assets:bank:ally:savings".to_string(), "C")
        );
        assert_eq!(
            new_account_name(&remote("Sapphire Visa ••9921", Some("Chase"), "-1032.18")),
            ("liabilities:chase:sapphire-visa".to_string(), "L")
        );
        assert_eq!(
            new_account_name(&remote("Brokerage", None, "5.00")),
            ("assets:bank:brokerage".to_string(), "A")
        );
    }

    #[test]
    fn categories_files_are_named_after_the_manual_file() {
        assert_eq!(
            categories_name("import/chase.csv.rules"),
            "chase.categories.rules"
        );
        assert_eq!(categories_name("bank.rules"), "bank.categories.rules");
        assert_eq!(dir_of("import/chase.csv.rules"), "import/");
        assert_eq!(dir_of("chase.csv.rules"), "");
    }

    #[test]
    fn account_names_and_dates_are_checked() {
        assert!(account_name_ok("assets:bank:ally:savings"));
        assert!(!account_name_ok("assets  x"));
        assert!(!account_name_ok("a;b"));
        assert!(!account_name_ok(" a"));
        assert_eq!(iso_date("2026-11-01").as_deref(), Some("2026-11-01"));
        assert_eq!(iso_date("2026-13-01"), None);
        assert_eq!(iso_date("11/01/2026"), None);
    }

    #[test]
    fn currency_prefers_the_account_then_maps_usd_to_dollars() {
        let journal = ledgeline_core::parse_journal(
            "2026-01-01 x\n    assets:bank:a  10 EUR\n    equity\n\n2026-01-02 y\n    assets:cash  $5\n    equity\n\n2026-01-03 z\n    assets:cash  $5\n    equity\n",
            "t.journal",
        )
        .expect("parses");
        let usd = remote("Checking", None, "1.00");
        assert_eq!(
            pick_currency(&journal, "assets:bank:a", Some(&usd))
                .0
                .as_deref(),
            Some("EUR")
        );
        assert_eq!(
            pick_currency(&journal, "assets:new", Some(&usd))
                .0
                .as_deref(),
            Some("$")
        );
        let gbp = RemoteSummary {
            currency: "GBP".into(),
            ..usd.clone()
        };
        assert_eq!(
            pick_currency(&journal, "assets:new", Some(&gbp))
                .0
                .as_deref(),
            Some("GBP")
        );
        let points = RemoteSummary {
            currency: "?".into(),
            currency_url: Some("https://bridge/currency/points".into()),
            ..usd
        };
        let (symbol, warning) = pick_currency(&journal, "assets:new", Some(&points));
        assert_eq!(symbol.as_deref(), Some("?"));
        assert!(warning.is_some());
    }

    #[test]
    fn a_shared_include_goes_ahead_of_the_id_guard() {
        let template = "skip 1\nfields date, amount, payee, description, memo, id, transacted\n\
                        comment id:%id\nif %id .\n  comment id:%id\n";
        assert_eq!(
            include_before_guard(template, "x.categories.rules").expect("ok"),
            "skip 1\nfields date, amount, payee, description, memo, id, transacted\n\
             comment id:%id\n\ninclude x.categories.rules\nif %id .\n  comment id:%id\n"
        );
        let plain = "skip 1\nfields a, b\n";
        assert!(
            include_before_guard(plain, "x.categories.rules")
                .expect("ok")
                .ends_with("include x.categories.rules\n")
        );
    }

    #[test]
    fn plain_decimals_have_two_places() {
        assert_eq!(plain(Dec::parse("20000", '.').expect("dec")), "20000.00");
        assert_eq!(plain(Dec::parse("-4.5", '.').expect("dec")), "-4.50");
        assert_eq!(plain(Dec::parse("0.125", '.').expect("dec")), "0.125");
    }
}
