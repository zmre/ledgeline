//! The bank-sync wire shapes (contract "Wire shapes") and the plan's §6 error
//! table: every way a sync can go wrong, as one [`WireProblem`] with who is
//! responsible, a plain sentence, since when, one action and the raw detail.
//!
//! The sentences are the plan's, verbatim where it gives one. They are built
//! here and nowhere else, so the web, the CLI and the diagnostics report all
//! say the same thing.
//!
//! Every server-supplied string that reaches a sentence or a detail has
//! already been through `feeds::sanitize` (control and bidi characters gone,
//! length capped); [`clip`] caps the rest.

use serde::{Deserialize, Serialize};

/// Who has to act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Who {
    Bank,
    Simplefin,
    Ledgeline,
    Network,
    Setup,
}

/// How loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// One row of §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProblemCode {
    ConAuth,
    ActFailed,
    Revoked,
    Payment,
    Quota,
    Network,
    EdgeBlocked,
    TokenUsed,
    Stale,
    Hledger,
    Rules,
    Held,
    Balance,
    Target,
    NotConnected,
    Elsewhere,
    Setup,
    Other,
}

impl ProblemCode {
    /// The docs anchor: the heading id in `docs/imports/bank-sync.md`, which
    /// the web links as `…/imports/bank-sync/#<anchor>`.
    #[must_use]
    pub const fn anchor(self) -> &'static str {
        match self {
            Self::ConAuth => "con-auth",
            Self::ActFailed => "act-failed",
            Self::Revoked => "revoked",
            Self::Payment => "payment",
            Self::Quota => "quota",
            Self::Network => "network",
            Self::EdgeBlocked => "edge-blocked",
            Self::TokenUsed => "token-used",
            Self::Stale => "stale",
            Self::Hledger => "hledger",
            Self::Rules => "rules",
            Self::Held => "held",
            Self::Balance => "balance",
            Self::Target => "target",
            Self::NotConnected => "not-connected",
            Self::Elsewhere => "elsewhere",
            Self::Setup => "setup",
            Self::Other => "other",
        }
    }
}

/// The one thing to do about a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    OpenUrl,
    Reconnect,
    Retry,
    EditRules,
    ReviewHeld,
    ChooseFile,
    Connect,
    SetHledger,
    ShowBalance,
    Recheck,
}

/// A problem's primary action. `ref` is the connection id for `reconnect`
/// and `connect`, the account key for `review-held`, `choose-file` and
/// `show-balance`, and the rules id for `edit-rules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireAction {
    pub kind: ActionKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub url: Option<String>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none", default)]
    pub reference: Option<String>,
}

/// "I've fixed it".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSecondary {
    pub kind: ActionKind,
    pub label: String,
}

/// One problem, ready to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireProblem {
    pub code: ProblemCode,
    pub severity: Severity,
    pub who: Who,
    pub message: String,
    pub since: Option<i64>,
    pub data_through: Option<String>,
    pub action: Option<WireAction>,
    pub secondary: Option<WireSecondary>,
    pub detail: Option<String>,
    pub doc_anchor: String,
}

/// The longest detail kept: hledger's stderr can be long, and the screen
/// shows it in a disclosure.
const MAX_DETAIL_CHARS: usize = 2_000;

/// `text` with control characters other than newline and tab removed, at
/// most [`MAX_DETAIL_CHARS`] characters.
#[must_use]
pub fn clip(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(MAX_DETAIL_CHARS)
        .collect()
}

/// SimpleFIN Bridge's own site, where every fix happens.
pub const BRIDGE_URL: &str = "https://beta-bridge.simplefin.org/";

/// The Bridge's host, to tell it from another SimpleFIN server.
pub const BRIDGE_HOST: &str = "beta-bridge.simplefin.org";

/// Where the user manages a connection: the Bridge's site for the Bridge,
/// else the server's own root.
#[must_use]
pub fn manage_url(host: Option<&str>) -> String {
    match host {
        Some(host) if host != BRIDGE_HOST && !host.is_empty() => format!("https://{host}/"),
        _ => BRIDGE_URL.to_string(),
    }
}

/// `Nov 11` for `2026-11-11` (the plan's "Data stops at Nov 11").
#[must_use]
pub fn short_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = iso
        .get(5..7)
        .and_then(|m| m.parse::<usize>().ok())
        .and_then(|m| MONTHS.get(m.wrapping_sub(1)));
    let day = iso.get(8..10).and_then(|d| d.parse::<u32>().ok());
    match (month, day) {
        (Some(month), Some(day)) => format!("{month} {day}"),
        _ => iso.to_string(),
    }
}

impl WireProblem {
    fn new(code: ProblemCode, severity: Severity, who: Who, message: String) -> Self {
        Self {
            code,
            severity,
            who,
            message,
            since: None,
            data_through: None,
            action: None,
            secondary: None,
            detail: None,
            doc_anchor: code.anchor().to_string(),
        }
    }

    fn action(mut self, kind: ActionKind, label: &str) -> Self {
        self.action = Some(WireAction {
            kind,
            label: label.to_string(),
            url: None,
            reference: None,
        });
        self
    }

    fn url(mut self, url: String) -> Self {
        if let Some(action) = &mut self.action {
            action.url = Some(url);
        }
        self
    }

    fn reference(mut self, reference: &str) -> Self {
        if let Some(action) = &mut self.action {
            action.reference = Some(reference.to_string());
        }
        self
    }

    /// Since when it has been true (epoch seconds).
    #[must_use]
    pub fn since(mut self, since: Option<i64>) -> Self {
        self.since = since;
        self
    }

    /// The raw code and server message, sanitized.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let detail = clip(&detail.into());
        self.detail = (!detail.trim().is_empty()).then_some(detail);
        self
    }

    /// The last date data is complete through.
    #[must_use]
    pub fn data_through(mut self, date: Option<String>) -> Self {
        self.data_through = date;
        self
    }

    fn recheck(mut self) -> Self {
        self.secondary = Some(WireSecondary {
            kind: ActionKind::Recheck,
            label: "I've fixed it".to_string(),
        });
        self
    }

    // -----------------------------------------------------------------------
    // §6, one constructor per row
    // -----------------------------------------------------------------------

    /// `con.auth`, or v1's "may need attention": the bank login needs fixing
    /// at SimpleFIN.
    #[must_use]
    pub fn con_auth(institution: &str, manage: &str) -> Self {
        let who = if institution.trim().is_empty() {
            "Your bank"
        } else {
            institution
        };
        Self::new(
            ProblemCode::ConAuth,
            Severity::Error,
            Who::Bank,
            format!("{who} needs you to sign in again at SimpleFIN."),
        )
        .action(ActionKind::OpenUrl, "Fix at SimpleFIN")
        .url(manage.to_string())
        .recheck()
    }

    /// `act.failed` / `act.missingdata`: one account failed today. Red, with
    /// a fix link, once it has failed for three days.
    #[must_use]
    pub fn act_failed(failing_days: i64, manage: &str) -> Self {
        let problem = Self::new(
            ProblemCode::ActFailed,
            Severity::Warning,
            Who::Simplefin,
            "SimpleFIN couldn't fetch this account today. Usually temporary.".to_string(),
        );
        if failing_days >= 3 {
            Self {
                severity: Severity::Error,
                ..problem
            }
            .action(ActionKind::OpenUrl, "Fix at SimpleFIN")
            .url(manage.to_string())
            .recheck()
        } else {
            problem
        }
    }

    /// HTTP 403 on `/accounts`: the access was turned off or revoked.
    #[must_use]
    pub fn revoked(connection: &str) -> Self {
        Self::new(
            ProblemCode::Revoked,
            Severity::Error,
            Who::Simplefin,
            "Ledgeline's access to SimpleFIN was turned off or revoked.".to_string(),
        )
        .action(ActionKind::Reconnect, "Reconnect…")
        .reference(connection)
    }

    /// HTTP 402: the subscription lapsed.
    #[must_use]
    pub fn payment(manage: &str) -> Self {
        Self::new(
            ProblemCode::Payment,
            Severity::Error,
            Who::Simplefin,
            "Your SimpleFIN subscription needs renewing.".to_string(),
        )
        .action(ActionKind::OpenUrl, "Open SimpleFIN")
        .url(manage.to_string())
    }

    /// A quota warning in the response.
    #[must_use]
    pub fn quota() -> Self {
        Self::new(
            ProblemCode::Quota,
            Severity::Warning,
            Who::Simplefin,
            "Close to SimpleFIN's daily limit. Auto-sync paused until tomorrow.".to_string(),
        )
    }

    /// Sync now refused at the hard daily cap.
    #[must_use]
    pub fn quota_refused(count: u32) -> Self {
        Self::new(
            ProblemCode::Quota,
            Severity::Warning,
            Who::Ledgeline,
            format!(
                "{count} syncs today. SimpleFIN allows about 24 before it disables access, so \
                 Ledgeline stops here until midnight UTC."
            ),
        )
    }

    /// Network trouble, 5xx or a timeout. Quiet (`info`) until it has lasted
    /// a day; then amber.
    #[must_use]
    pub fn network(host: &str, since: i64, now: i64) -> Self {
        const DAY: i64 = 86_400;
        let lasting = now - since >= DAY;
        let message = if lasting {
            format!(
                "Can't reach SimpleFIN since {}.",
                short_date(&crate::sync::iso_utc(since))
            )
        } else {
            format!("Couldn't reach {host}. Check your connection.")
        };
        Self::new(
            ProblemCode::Network,
            if lasting {
                Severity::Warning
            } else {
                Severity::Info
            },
            Who::Network,
            message,
        )
        .action(ActionKind::Retry, "Try again")
        .since(Some(since))
    }

    /// The claim endpoint's 403 at the Cloudflare edge.
    #[must_use]
    pub fn edge_blocked() -> Self {
        Self::new(
            ProblemCode::EdgeBlocked,
            Severity::Warning,
            Who::Simplefin,
            "SimpleFIN's firewall refused the connection. Try again in a few minutes.".to_string(),
        )
        .action(ActionKind::Retry, "Try again")
    }

    /// The claim endpoint's own 403: the token was already used.
    #[must_use]
    pub fn token_used(manage: &str) -> Self {
        Self::new(
            ProblemCode::TokenUsed,
            Severity::Error,
            Who::Simplefin,
            "This token was already used. If you didn't use it in another app, someone else may \
             have, so disable it on SimpleFIN. Then create a new one and paste it here."
                .to_string(),
        )
        .action(ActionKind::OpenUrl, "Open SimpleFIN")
        .url(manage.to_string())
    }

    /// No new transactions for a while, and no error.
    #[must_use]
    pub fn stale(last_new: &str, manage: &str) -> Self {
        Self::new(
            ProblemCode::Stale,
            Severity::Info,
            Who::Bank,
            format!(
                "No new transactions from SimpleFIN since {}. Normal for quiet accounts.",
                short_date(last_new)
            ),
        )
        .action(ActionKind::OpenUrl, "Fix at SimpleFIN")
        .url(manage.to_string())
    }

    /// hledger is missing or too old: the import screen's own banner text.
    #[must_use]
    pub fn hledger(message: &str) -> Self {
        Self::new(
            ProblemCode::Hledger,
            Severity::Error,
            Who::Setup,
            clip(message),
        )
        .action(ActionKind::SetHledger, "Set hledger path")
    }

    /// The account's rules file made hledger fail.
    #[must_use]
    pub fn rules(rules_id: &str, stderr: &str) -> Self {
        Self::new(
            ProblemCode::Rules,
            Severity::Error,
            Who::Ledgeline,
            "The rules for this account stopped working.".to_string(),
        )
        .action(ActionKind::EditRules, "Edit rules")
        .reference(rules_id)
        .detail(stderr)
    }

    /// Possible duplicates are waiting.
    #[must_use]
    pub fn held(count: usize, account_key: &str) -> Self {
        let message = if count == 1 {
            "1 transaction needs a look before it's added.".to_string()
        } else {
            format!("{count} transactions need a look before they're added.")
        };
        Self::new(
            ProblemCode::Held,
            Severity::Warning,
            Who::Ledgeline,
            message,
        )
        .action(ActionKind::ReviewHeld, "Review")
        .reference(account_key)
    }

    /// The ledger and the bank disagree about the balance.
    #[must_use]
    pub fn balance(difference: &str, date: &str, account_key: &str) -> Self {
        Self::new(
            ProblemCode::Balance,
            Severity::Info,
            Who::Ledgeline,
            format!(
                "Ledger differs from bank by {difference} as of {}.",
                short_date(date)
            ),
        )
        .action(ActionKind::ShowBalance, "Show me")
        .reference(account_key)
    }

    /// The target file is missing or can't be written.
    #[must_use]
    pub fn target(target: &str, account_key: &str) -> Self {
        Self::new(
            ProblemCode::Target,
            Severity::Error,
            Who::Setup,
            format!("Can't write to {target}."),
        )
        .action(ActionKind::ChooseFile, "Choose another file")
        .reference(account_key)
    }

    /// A link whose connection has no keychain item on this machine.
    #[must_use]
    pub fn not_connected(connection: &str) -> Self {
        Self::new(
            ProblemCode::NotConnected,
            Severity::Warning,
            Who::Setup,
            "Not connected on this computer.".to_string(),
        )
        .action(ActionKind::Connect, "Connect…")
        .reference(connection)
    }

    /// A **Sync now** on a computer that was never connected to
    /// `connection`: another computer syncs this ledger. Info, not a fault;
    /// [`super::status`] doesn't report it at all.
    #[must_use]
    pub fn elsewhere(connection: &str) -> Self {
        Self::new(
            ProblemCode::Elsewhere,
            Severity::Info,
            Who::Setup,
            "Bank sync for this ledger runs on another computer.".to_string(),
        )
        .action(ActionKind::Connect, "Sync from this computer too…")
        .reference(connection)
    }

    /// The keychain holds something for `connection` that isn't an access
    /// URL: store a fresh token under the same id.
    #[must_use]
    pub fn damaged_key(connection: &str) -> Self {
        Self::setup(
            "The saved SimpleFIN access key on this computer is damaged. Reconnect with a new \
             setup token.",
        )
        .action(ActionKind::Reconnect, "Reconnect…")
        .reference(connection)
    }

    /// Something in the user's setup stops this: a path outside the journal
    /// folder, no keychain, an unreadable `ledgeline.toml`.
    #[must_use]
    pub fn setup(message: impl Into<String>) -> Self {
        Self::new(
            ProblemCode::Setup,
            Severity::Error,
            Who::Setup,
            clip(&message.into()),
        )
    }

    /// Anything else.
    #[must_use]
    pub fn other(message: impl Into<String>, severity: Severity) -> Self {
        Self::new(
            ProblemCode::Other,
            severity,
            Who::Ledgeline,
            clip(&message.into()),
        )
    }
}

// ---------------------------------------------------------------------------
// Status and runs
// ---------------------------------------------------------------------------

/// `GET /api/sync/status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSyncStatus {
    pub auto_sync: bool,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub running: bool,
    pub last_run: Option<i64>,
    pub next_auto: Option<i64>,
    pub connections: Vec<WireConnection>,
    pub accounts: Vec<WireLinkedAccount>,
    pub unlinked: Vec<WireRemoteAccount>,
    pub problem_count: WireProblemCount,
}

/// Warnings and errors across the whole status (info is quiet).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct WireProblemCount {
    pub warning: usize,
    pub error: usize,
}

impl WireProblemCount {
    /// Count `problems`.
    pub fn add<'a>(&mut self, problems: impl IntoIterator<Item = &'a WireProblem>) {
        for problem in problems {
            match problem.severity {
                Severity::Warning => self.warning += 1,
                Severity::Error => self.error += 1,
                Severity::Info => {}
            }
        }
    }
}

/// The quota numbers the UI explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireQuota {
    pub auto_max: u32,
    pub manual_warn: u32,
    pub manual_max: u32,
}

/// One connection's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionState {
    Ok,
    Syncing,
    Attention,
    Broken,
    NotConnected,
    /// Linked in the shared `ledgeline.toml`, never connected on this
    /// computer: another computer syncs this ledger. Not a problem.
    Elsewhere,
}

/// One `[[sync.connection]]`, as the Accounts tab shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireConnection {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub host: Option<String>,
    pub connected: bool,
    pub state: ConnectionState,
    pub last_attempt: Option<i64>,
    pub last_success: Option<i64>,
    pub requests_today: u32,
    pub quota: WireQuota,
    pub manage_url: String,
    pub problems: Vec<WireProblem>,
}

/// One sync outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Added,
    NothingNew,
    Failed,
    Skipped,
}

/// One history line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireHistory {
    pub at: i64,
    pub outcome: Outcome,
    pub added: usize,
    pub held: usize,
}

/// The bank's balance, as sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireBankBalance {
    pub amount: String,
    pub date: String,
}

/// A held row's existing match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireExisting {
    pub index: usize,
    pub date: String,
    pub description: String,
    pub amount: String,
}

/// A possible duplicate waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireHeld {
    pub held_id: String,
    pub date: String,
    pub amount: String,
    pub payee: String,
    pub existing: WireExisting,
}

/// A rules file the drawer's "Change…" may offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireRulesChoice {
    pub id: String,
    pub fits: bool,
    /// The file's top-level `account1`: the journal account it posts to, so
    /// the link form preselects only a file made for that account.
    pub account1: Option<String>,
}

/// One `[[sync.account]]`, with what the last syncs did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireLinkedAccount {
    pub key: String,
    pub connection_id: String,
    pub remote_id: String,
    pub label: String,
    pub org: Option<String>,
    pub journal_account: String,
    pub target: String,
    pub rules: String,
    pub categories: Option<String>,
    pub start_date: String,
    pub balance_sign: i8,
    pub enabled: bool,
    pub last_success: Option<i64>,
    pub data_through: Option<String>,
    pub last_added: usize,
    pub bank_balance: Option<WireBankBalance>,
    pub ledger_balance: Option<String>,
    pub balance_diff: Option<String>,
    pub held: Vec<WireHeld>,
    pub history: Vec<WireHistory>,
    pub problems: Vec<WireProblem>,
    pub rules_choices: Vec<WireRulesChoice>,
}

/// A remote account's suggested mapping.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSuggestion {
    pub journal_account: Option<String>,
    pub score: f64,
    pub ledger_balance: Option<String>,
    pub sign_flipped: bool,
    pub start_date: String,
    pub target: Option<String>,
    pub new_account_name: String,
    pub new_account_type: String,
    pub sync_rules: Vec<WireRulesChoice>,
    pub manual_rules: Option<WireManualRules>,
}

/// A manual rules file that already categorizes the account.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireManualRules {
    pub id: String,
    pub rule_count: usize,
}

/// One account the provider has, linked or not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireRemoteAccount {
    pub connection_id: String,
    pub remote_id: String,
    pub label: String,
    pub org: Option<String>,
    pub balance: String,
    pub balance_date: Option<String>,
    pub currency: String,
    pub oldest_available: Option<String>,
    pub suggestion: WireSuggestion,
}

/// One account's part of a run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireRunAccount {
    pub key: String,
    pub label: String,
    pub added: usize,
    pub held: usize,
    pub already: usize,
    /// Written rows still posted to the fallback account (WP-W: "Review N").
    pub uncategorized: usize,
    pub outcome: Outcome,
    pub problem: Option<WireProblem>,
}

/// What a sync run did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSyncRun {
    pub ok: bool,
    pub requests_today: u32,
    pub quota_warning: bool,
    pub accounts: Vec<WireRunAccount>,
    pub problems: Vec<WireProblem>,
}

/// One link's dry run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireLinkPreview {
    pub remote_id: String,
    pub ok: bool,
    pub error: Option<WireProblem>,
    pub new: usize,
    pub already: usize,
    pub held: usize,
    pub uncategorized: usize,
    pub dropped_before_start: usize,
    pub entries: String,
}

/// One kept block in a share preview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireKept {
    pub header: String,
    pub reason: String,
}

/// The two diffs of a share preview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireShareDiffs {
    pub manual: String,
    pub categories: String,
}

/// `POST /api/sync/share/preview`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSharePreview {
    pub manual_rules_id: String,
    pub categories_id: String,
    pub moved: usize,
    pub kept: Vec<WireKept>,
    pub diffs: WireShareDiffs,
    pub identical: bool,
    pub differences: Option<String>,
}

/// One manual-download account.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireManualAccount {
    pub account: String,
    #[serde(default)]
    pub last_transaction: Option<String>,
    #[serde(default)]
    pub last_import: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    pub nudge: bool,
    #[serde(default)]
    pub stale: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_code_serializes_as_its_anchor() {
        for code in [
            ProblemCode::ConAuth,
            ProblemCode::ActFailed,
            ProblemCode::Revoked,
            ProblemCode::Payment,
            ProblemCode::Quota,
            ProblemCode::Network,
            ProblemCode::EdgeBlocked,
            ProblemCode::TokenUsed,
            ProblemCode::Stale,
            ProblemCode::Hledger,
            ProblemCode::Rules,
            ProblemCode::Held,
            ProblemCode::Balance,
            ProblemCode::Target,
            ProblemCode::NotConnected,
            ProblemCode::Elsewhere,
            ProblemCode::Setup,
            ProblemCode::Other,
        ] {
            assert_eq!(
                serde_json::to_value(code).expect("code"),
                json!(code.anchor())
            );
        }
    }

    #[test]
    fn a_problem_has_the_contract_shape() {
        let problem = WireProblem::con_auth("Chase", BRIDGE_URL)
            .since(Some(5))
            .data_through(Some("2026-11-11".into()))
            .detail("con.auth: Please reauthenticate");
        assert_eq!(
            serde_json::to_value(&problem).expect("problem"),
            json!({
                "code": "con-auth", "severity": "error", "who": "bank",
                "message": "Chase needs you to sign in again at SimpleFIN.",
                "since": 5, "dataThrough": "2026-11-11",
                "action": {"kind": "open-url", "label": "Fix at SimpleFIN",
                           "url": "https://beta-bridge.simplefin.org/"},
                "secondary": {"kind": "recheck", "label": "I've fixed it"},
                "detail": "con.auth: Please reauthenticate",
                "docAnchor": "con-auth"
            })
        );
    }

    #[test]
    fn refs_name_what_the_action_needs() {
        let reference = |p: WireProblem| p.action.and_then(|a| a.reference);
        assert_eq!(reference(WireProblem::revoked("c1")).as_deref(), Some("c1"));
        assert_eq!(reference(WireProblem::held(2, "k1")).as_deref(), Some("k1"));
        assert_eq!(
            reference(WireProblem::target("2025.journal", "k1")).as_deref(),
            Some("k1")
        );
        assert_eq!(
            reference(WireProblem::balance("$1.00", "2026-11-13", "k1")).as_deref(),
            Some("k1")
        );
        assert_eq!(
            reference(WireProblem::rules("r.rules", "boom")).as_deref(),
            Some("r.rules")
        );
        assert_eq!(
            reference(WireProblem::not_connected("c1")).as_deref(),
            Some("c1")
        );
    }

    #[test]
    fn act_failed_turns_red_after_three_days() {
        assert_eq!(
            WireProblem::act_failed(1, BRIDGE_URL).severity,
            Severity::Warning
        );
        assert!(WireProblem::act_failed(1, BRIDGE_URL).action.is_none());
        let red = WireProblem::act_failed(3, BRIDGE_URL);
        assert_eq!(red.severity, Severity::Error);
        assert_eq!(red.action.expect("action").kind, ActionKind::OpenUrl);
    }

    #[test]
    fn network_is_quiet_for_a_day() {
        let since = 1_794_268_800;
        let early = WireProblem::network("beta-bridge.simplefin.org", since, since + 3_600);
        assert_eq!(early.severity, Severity::Info);
        assert_eq!(
            early.message,
            "Couldn't reach beta-bridge.simplefin.org. Check your connection."
        );
        let late = WireProblem::network("beta-bridge.simplefin.org", since, since + 86_400);
        assert_eq!(late.severity, Severity::Warning);
        assert_eq!(late.message, "Can't reach SimpleFIN since Nov 10.");
    }

    #[test]
    fn manage_url_is_the_bridge_unless_another_server() {
        assert_eq!(manage_url(Some(BRIDGE_HOST)), BRIDGE_URL);
        assert_eq!(manage_url(None), BRIDGE_URL);
        assert_eq!(
            manage_url(Some("sf.example.com")),
            "https://sf.example.com/"
        );
    }

    #[test]
    fn held_counts_read_naturally() {
        assert_eq!(
            WireProblem::held(1, "k").message,
            "1 transaction needs a look before it's added."
        );
        assert_eq!(
            WireProblem::held(2, "k").message,
            "2 transactions need a look before they're added."
        );
    }
}
