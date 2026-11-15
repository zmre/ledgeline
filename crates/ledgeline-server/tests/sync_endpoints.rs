//! Bank sync's HTTP surface and engine (plan 43, contract WP-S), end to end
//! over a scratch copy of `fixtures/sample.journal`.
//!
//! Hermetic: `FakeFeed` stands in for SimpleFIN (injected through
//! `AppState::with_feed_provider`, serving `fixtures/feeds/fake-bridge/` the
//! way the Bridge would) and `MemoryStore` for the keychain. No network, no
//! keychain, ever. The real reqwest client is tested against an in-process
//! axum "SimpleFIN" in `src/sync/client.rs`.
//!
//! The import itself runs hledger, so everything that commits is gated behind
//! `LEDGELINE_HLEDGER_SYNC_CHECK=1` like the other hledger suites (`just
//! hledger-checks`). The rest — claims, the keychain, `ledgeline.toml`,
//! status, read-only refusals, the scheduler's quiet paths — always runs.

mod common;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{
    AccountsQuery, AppDirs, AppState, FeedError, FeedProvider, MemoryStore, filter_batch,
    router_with_state,
};
use ledgeline_core::feeds::FeedBatch;
use ledgeline_core::feeds::simplefin::{AccessUrl, SetupToken, parse_accounts};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tower::ServiceExt;

/// Opts in to the checks that run a real `hledger`.
const SYNC_CHECK: &str = "LEDGELINE_HLEDGER_SYNC_CHECK";

macro_rules! require_hledger {
    () => {
        if std::env::var_os(SYNC_CHECK).is_none() {
            eprintln!("skipping: set {SYNC_CHECK}=1 (or run `just hledger-checks`)");
            return;
        }
    };
}

/// 2026-07-16 12:00 UTC: the day after the fake bridge's balance date.
const NOW: i64 = 1_784_203_200;
const DAY: i64 = 86_400;

/// `https://fake.simplefin.test/claim/DEMO`, base64.
const TOKEN: &str = "aHR0cHM6Ly9mYWtlLnNpbXBsZWZpbi50ZXN0L2NsYWltL0RFTU8=";

// ---------------------------------------------------------------------------
// The fake feed
// ---------------------------------------------------------------------------

/// How the fake answers a claim.
#[derive(Clone, Copy)]
enum Claim {
    Ok,
    Used,
    Edge,
}

/// A [`FeedProvider`] over a batch the test controls, recording every query.
struct FakeFeed {
    batch: Mutex<FeedBatch>,
    queries: Mutex<Vec<AccountsQuery>>,
    claim: Mutex<Claim>,
    fail: Mutex<Option<FeedError>>,
}

impl FakeFeed {
    fn bridge() -> Arc<Self> {
        let body = std::fs::read(common::fixtures_dir().join("feeds/fake-bridge/accounts.json"))
            .expect("fake bridge fixture");
        Arc::new(Self {
            batch: Mutex::new(parse_accounts(&body).expect("fixture parses")),
            queries: Mutex::default(),
            claim: Mutex::new(Claim::Ok),
            fail: Mutex::new(None),
        })
    }

    fn queries(&self) -> Vec<AccountsQuery> {
        self.queries.lock().expect("lock").clone()
    }

    fn requests(&self) -> usize {
        self.queries.lock().expect("lock").len()
    }

    fn edit(&self, change: impl FnOnce(&mut FeedBatch)) {
        change(&mut self.batch.lock().expect("lock"));
    }
}

#[async_trait]
impl FeedProvider for FakeFeed {
    async fn claim(&self, token: &SetupToken) -> Result<String, FeedError> {
        match *self.claim.lock().expect("lock") {
            Claim::Ok => Ok(format!("https://user:secret@{}/simplefin", token.host)),
            Claim::Used => Err(FeedError::TokenUsed),
            Claim::Edge => Err(FeedError::EdgeBlocked),
        }
    }

    async fn accounts(
        &self,
        _access: &AccessUrl,
        query: &AccountsQuery,
    ) -> Result<FeedBatch, FeedError> {
        self.queries.lock().expect("lock").push(query.clone());
        if let Some(error) = self.fail.lock().expect("lock").clone() {
            return Err(error);
        }
        Ok(filter_batch(
            self.batch.lock().expect("lock").clone(),
            query,
        ))
    }
}

// ---------------------------------------------------------------------------
// The scratch ledger
// ---------------------------------------------------------------------------

/// The `ledgeline.toml` every tree starts with: a `[ui]` table and a comment
/// that linking must leave byte for byte.
const LEDGER_TOML: &str = "# my settings\n[ui]\ntheme = \"nord\" # keep\n";

struct Tree {
    dir: TempDir,
    state: AppState,
    feed: Arc<FakeFeed>,
    secrets: Arc<MemoryStore>,
}

impl Tree {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::copy(
            common::fixture_journal_path(),
            dir.path().join("sample.journal"),
        )
        .expect("copy the sample journal");
        std::fs::write(dir.path().join("ledgeline.toml"), LEDGER_TOML).expect("toml");
        let feed = FakeFeed::bridge();
        let secrets = Arc::new(MemoryStore::new());
        let dirs = AppDirs::under(dir.path().join("app"));
        dirs.create_all().expect("app dirs");
        let state = AppState::from_journal_path(dir.path().join("sample.journal"))
            .expect("opens")
            .with_app_dirs(dirs)
            .with_feed_provider(feed.clone())
            .with_secret_store(secrets.clone());
        Self {
            dir,
            state,
            feed,
            secrets,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).unwrap_or_default()
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |body| {
                Body::from(serde_json::to_vec(&body).expect("json"))
            }))
            .expect("request");
        let response = self.router().oneshot(request).await.expect("responds");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn connect(&self) -> String {
        let (status, body) = self
            .call(
                "POST",
                "/api/sync/connect",
                Some(json!({"setupToken": TOKEN})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["connectionId"].as_str().expect("an id").to_string()
    }

    /// Link checking and the Visa card, with new rules files beside the
    /// journal, and run their first sync.
    async fn link_both(&self, connection: &str) -> Value {
        let (status, body) = self
            .call(
                "POST",
                "/api/sync/link",
                Some(json!({"links": [
                    link_request(connection, "ACT-lakeside-checking", "assets:bank:checking", "checking.sync.csv.rules"),
                    link_request(connection, "ACT-harbor-visa", "liabilities:cc:visa", "visa.sync.csv.rules"),
                ]})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn status(&self) -> Value {
        let (status, body) = self.call("GET", "/api/sync/status", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }
}

impl Tree {
    /// Link `remote` to `account` by hand — a fresh feed rules file and a
    /// `[[sync.account]]` — with no first sync, so a test can run every sync
    /// on its own fixed clock.
    fn link_by_hand(&self, connection: &str, remote: &str, account: &str, rules: &str) -> String {
        let text = ledgeline_core::rules::generate::feed_rules(
            &ledgeline_core::rules::generate::FeedRulesParams {
                label: remote,
                provider: "SimpleFIN",
                columns: &ledgeline_core::feeds::normalize::FeedColumns::default(),
                journal_account: account,
                fallback_account: "expenses:unknown",
                categories_include: None,
                currency: Some("$"),
            },
        );
        std::fs::write(self.path(rules), text).expect("rules");
        let mut toml = self.read("ledgeline.toml");
        toml.push_str(&format!(
            "\n[[sync.account]]\nconnection = \"{connection}\"\nremoteId = \"{remote}\"\n\
             label = \"{remote}\"\njournalAccount = \"{account}\"\nrules = \"{rules}\"\n\
             target = \"sample.journal\"\nstartDate = 2026-07-04\nbalanceSign = 1\nenabled = true\n"
        ));
        std::fs::write(self.path("ledgeline.toml"), toml).expect("toml");
        ledgeline_core::feeds::normalize::account_short(connection, remote)
    }

    /// Both accounts linked by hand, then their first sync at [`NOW`].
    async fn first_sync_at_now(&self, connection: &str) -> Value {
        self.link_by_hand(
            connection,
            "ACT-lakeside-checking",
            "assets:bank:checking",
            "checking.sync.csv.rules",
        );
        self.link_by_hand(
            connection,
            "ACT-harbor-visa",
            "liabilities:cc:visa",
            "visa.sync.csv.rules",
        );
        let run = self.state.run_sync_now(NOW).await;
        assert_eq!(run["ok"], true, "{run}");
        run
    }
}

fn link_request(connection: &str, remote: &str, account: &str, rules: &str) -> Value {
    json!({
        "connectionId": connection,
        "remoteId": remote,
        "label": remote,
        "journalAccount": account,
        "newAccount": null,
        "startDate": "2026-07-04",
        "target": "sample.journal",
        "balanceSign": 1,
        "rules": {"mode": "new", "name": rules},
        "share": null
    })
}

/// The `[sync]` part of `text` parsed back.
fn sync_config(text: &str) -> ledgeline_core::feeds::config::SyncConfig {
    let (config, warnings) = ledgeline_core::feeds::config::parse(text);
    assert!(warnings.is_empty(), "{warnings:?}");
    config
}

fn account_in<'a>(status: &'a Value, remote: &str) -> &'a Value {
    status["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["remoteId"] == json!(remote))
        .unwrap_or_else(|| panic!("{remote} in {status}"))
}

fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else {
                out.push(path.display().to_string());
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Always: claims, keychain, ledgeline.toml, status, refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unlinked_ledger_reports_an_empty_status() {
    let tree = Tree::new();
    let status = tree.status().await;
    assert_eq!(
        status,
        json!({
            "autoSync": false, "available": true, "unavailableReason": null,
            "running": false, "lastRun": null, "nextAuto": null,
            "connections": [], "accounts": [], "unlinked": [],
            "problemCount": {"warning": 0, "error": 0}
        })
    );
    assert_eq!(tree.feed.requests(), 0);
}

#[tokio::test]
async fn connecting_keeps_the_key_in_the_keychain_and_lists_suggestions() {
    let tree = Tree::new();
    let (status, body) = tree
        .call(
            "POST",
            "/api/sync/connect",
            Some(json!({"setupToken": TOKEN, "name": "My Bridge"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["connectionId"].as_str().expect("id");
    assert_eq!(id.len(), 36, "a uuid");
    assert!(tree.secrets.contains(id));
    // One balances-only request.
    assert_eq!(tree.feed.requests(), 1);
    assert!(tree.feed.queries()[0].balances_only);

    let accounts = body["accounts"].as_array().expect("accounts");
    assert_eq!(accounts.len(), 3);
    let checking = &accounts[0];
    assert_eq!(checking["label"], "Lakeside Credit Union · Checking ••4821");
    assert_eq!(checking["balance"], "32292.23");
    assert_eq!(checking["currency"], "USD");
    assert_eq!(
        checking["suggestion"]["journalAccount"],
        "assets:bank:checking"
    );
    assert_eq!(
        checking["suggestion"]["startDate"], "2026-07-02",
        "the day after its last transaction"
    );
    assert_eq!(checking["suggestion"]["target"], "sample.journal");
    let savings = &accounts[2];
    assert_eq!(
        savings["suggestion"]["newAccountName"],
        "assets:bank:summit-online-bank:high-yield-savings"
    );
    assert_eq!(savings["suggestion"]["newAccountType"], "C");
    let visa = &accounts[1];
    assert_eq!(visa["suggestion"]["journalAccount"], "liabilities:cc:visa");

    // ledgeline.toml: the connection added, the [ui] table and comments kept.
    let toml = tree.read("ledgeline.toml");
    assert!(toml.starts_with(LEDGER_TOML), "{toml}");
    let config = sync_config(&toml);
    assert_eq!(config.connections.len(), 1);
    assert_eq!(config.connections[0].name, "My Bridge");
    assert!(!toml.contains("secret"), "no credential in ledgeline.toml");

    // The status lists them as not linked yet.
    let status = tree.status().await;
    assert_eq!(status["connections"][0]["connected"], true);
    assert_eq!(status["connections"][0]["state"], "ok");
    assert_eq!(status["connections"][0]["requestsToday"], 1);
    assert_eq!(
        status["connections"][0]["quota"],
        json!({"autoMax": 6, "manualWarn": 12, "manualMax": 16})
    );
    assert_eq!(status["unlinked"].as_array().map(Vec::len), Some(3));
}

#[tokio::test]
async fn a_bad_or_spent_token_is_a_problem_with_its_own_sentence() {
    let tree = Tree::new();
    let (status, body) = tree
        .call(
            "POST",
            "/api/sync/connect",
            Some(json!({"setupToken": "not a token!"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["problem"]["code"], "setup");

    *tree.feed.claim.lock().expect("lock") = Claim::Used;
    let (status, body) = tree
        .call(
            "POST",
            "/api/sync/connect",
            Some(json!({"setupToken": TOKEN})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["problem"]["code"], "token-used");
    assert!(
        body["problem"]["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("This token was already used."))
    );

    *tree.feed.claim.lock().expect("lock") = Claim::Edge;
    let (_, body) = tree
        .call(
            "POST",
            "/api/sync/connect",
            Some(json!({"setupToken": TOKEN})),
        )
        .await;
    assert_eq!(body["problem"]["code"], "edge-blocked");
    // Nothing was kept from any of them.
    assert_eq!(tree.read("ledgeline.toml"), LEDGER_TOML);
}

#[tokio::test]
async fn http_tokens_are_refused_without_the_loopback_escape() {
    let tree = Tree::new();
    use base64::Engine;
    let http = base64::engine::general_purpose::STANDARD.encode("http://127.0.0.1:9/claim/X");
    let (status, body) = tree
        .call(
            "POST",
            "/api/sync/connect",
            Some(json!({"setupToken": http})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["problem"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("https")),
        "{body}"
    );
    // A loopback-enabled test state accepts it as far as the claim.
    let loopback = tree.state.clone().with_sync_loopback_http();
    let request = Request::builder()
        .method("POST")
        .uri("/api/sync/connect")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"setupToken": http})).expect("json"),
        ))
        .expect("request");
    let response = router_with_state(loopback)
        .oneshot(request)
        .await
        .expect("responds");
    // Nothing listens on port 9: a network problem, not an https refusal.
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(body["problem"]["code"], "network", "{body}");
}

#[tokio::test]
async fn writes_are_refused_on_a_read_only_server() {
    let journal = common::fixture_journal();
    let state = AppState::from_journal(&journal)
        .with_feed_provider(FakeFeed::bridge())
        .with_secret_store(Arc::new(MemoryStore::new()));
    let router = router_with_state(state);
    for (method, uri) in [
        ("POST", "/api/sync/connect"),
        ("POST", "/api/sync/now"),
        ("POST", "/api/sync/link"),
        ("DELETE", "/api/sync/connections/x"),
        ("DELETE", "/api/sync/accounts/x"),
        ("PATCH", "/api/sync/accounts/x"),
        ("POST", "/api/sync/held/x/y"),
        ("PUT", "/api/sync/manual"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("responds");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[tokio::test]
async fn every_sync_route_needs_the_token() {
    use ledgeline::{AccessToken, Security, router_with_security};
    let tree = Tree::new();
    let token = AccessToken::parse("integration-test-token").expect("token");
    let router = router_with_security(tree.state.clone(), Security::local(token, 5099));
    for (method, uri) in [
        ("GET", "/api/sync/status"),
        ("POST", "/api/sync/connect"),
        ("GET", "/api/sync/manual"),
        ("GET", "/api/sync/connections/x/diagnostics"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "127.0.0.1:5099")
            .body(Body::empty())
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("responds");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn disconnecting_removes_the_keychain_item_and_keeps_the_links() {
    let tree = Tree::new();
    let id = tree.connect().await;
    // A link written by hand (no first sync needed for this test).
    let link = format!(
        "\n[[sync.account]]\nconnection = \"{id}\"\nremoteId = \"ACT-lakeside-checking\"\n\
         label = \"Checking\"\njournalAccount = \"assets:bank:checking\"\nrules = \"x.rules\"\n\
         target = \"sample.journal\"\nstartDate = 2026-07-04\nbalanceSign = 1\nenabled = true\n"
    );
    let mut toml = tree.read("ledgeline.toml");
    toml.push_str(&link);
    std::fs::write(tree.path("ledgeline.toml"), &toml).expect("write");

    let (status, body) = tree
        .call("DELETE", &format!("/api/sync/connections/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!tree.secrets.contains(&id));
    let config = sync_config(&tree.read("ledgeline.toml"));
    // ledgeline.toml is shared between computers: a connection that still
    // has links stays, so another Mac keeps syncing and this one can
    // reconnect under the same id (which keeps every `id:` prefix).
    assert_eq!(config.connections.len(), 1, "the connection stays");
    assert_eq!(config.accounts.len(), 1, "the link stays");
    // The connection card says so, and its Connect… names the connection.
    let connection = &body["connections"][0];
    assert_eq!(connection["state"], "not-connected", "{body}");
    let problem = connection["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .find(|p| p["code"] == "not-connected")
        .expect("a not-connected problem");
    assert_eq!(problem["action"]["ref"], id.as_str());
    assert!(tree.read("ledgeline.toml").starts_with(LEDGER_TOML));

    // Connect on this computer again: same connection id, links reattached.
    let (status, body) = tree
        .call(
            "POST",
            &format!("/api/sync/connections/{id}/token"),
            Some(json!({"setupToken": TOKEN})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(tree.secrets.contains(&id));
    let config = sync_config(&tree.read("ledgeline.toml"));
    assert_eq!(config.connections.len(), 1);
    assert_eq!(config.accounts[0].connection, id);
}

#[tokio::test]
async fn disconnecting_an_unlinked_connection_removes_its_entry() {
    let tree = Tree::new();
    let id = tree.connect().await;
    let (status, body) = tree
        .call("DELETE", &format!("/api/sync/connections/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!tree.secrets.contains(&id));
    assert!(
        sync_config(&tree.read("ledgeline.toml"))
            .connections
            .is_empty()
    );
}

#[tokio::test]
async fn a_link_with_paths_outside_the_journal_folder_is_a_setup_problem() {
    let tree = Tree::new();
    let id = tree.connect().await;
    let link = format!(
        "\n[[sync.account]]\nconnection = \"{id}\"\nremoteId = \"ACT-lakeside-checking\"\n\
         label = \"Checking\"\njournalAccount = \"assets:bank:checking\"\nrules = \"../../etc/passwd.rules\"\n\
         target = \"../elsewhere.journal\"\nstartDate = 2026-07-04\nbalanceSign = 1\nenabled = true\n"
    );
    let mut toml = tree.read("ledgeline.toml");
    toml.push_str(&link);
    std::fs::write(tree.path("ledgeline.toml"), &toml).expect("write");
    let before = tree.feed.requests();
    let run = tree.state.run_sync_now(NOW).await;
    assert_eq!(run["accounts"][0]["outcome"], "failed", "{run}");
    assert_eq!(run["accounts"][0]["problem"]["code"], "setup");
    assert_eq!(
        tree.feed.requests(),
        before,
        "nothing fetched for a bad link"
    );
}

#[tokio::test]
async fn auto_sync_off_never_touches_the_feed() {
    let tree = Tree::new();
    let _ = tree.connect().await;
    let before = tree.feed.requests();
    tree.state.run_sync_once(NOW).await;
    assert_eq!(tree.feed.requests(), before);
}

#[tokio::test]
async fn manual_accounts_list_the_unlinked_balance_sheet_and_save_a_url() {
    let tree = Tree::new();
    let (status, body) = tree.call("GET", "/api/sync/manual", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let accounts = body["accounts"].as_array().expect("accounts");
    assert!(
        accounts
            .iter()
            .any(|a| a["account"] == "assets:bank:checking")
    );
    assert!(!accounts.iter().any(|a| a["account"] == "expenses:shopping"));
    let (status, body) = tree
        .call(
            "PUT",
            "/api/sync/manual",
            Some(json!({"account": "assets:bank:wise:eur", "downloadUrl": "https://wise.example/statements", "nudge": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let wise = body["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["account"] == "assets:bank:wise:eur")
        .expect("wise");
    assert_eq!(wise["downloadUrl"], "https://wise.example/statements");
    assert_eq!(wise["nudge"], false);
    assert_eq!(wise["stale"], false);
    assert!(tree.read("ledgeline.toml").starts_with(LEDGER_TOML));
}

// ---------------------------------------------------------------------------
// hledger: link, first sync, idempotence, held rows, conflicts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn linking_writes_rules_config_and_the_first_sync_exactly() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    let before = tree.read("sample.journal");
    let run = tree.link_both(&id).await;
    assert_eq!(run["ok"], true, "{run}");
    let checking = run["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["label"] == "ACT-lakeside-checking")
        .expect("checking");
    assert_eq!(checking["outcome"], "added", "{run}");
    assert_eq!(checking["added"], 4);
    assert_eq!(checking["uncategorized"], 4);
    let visa = run["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["label"] == "ACT-harbor-visa")
        .expect("visa");
    assert_eq!(visa["added"], 2);
    assert_eq!(
        visa["held"], 1,
        "Safeway matches the journal's own purchase"
    );

    // The journal: exactly what was appended, nothing else moved.
    let after = tree.read("sample.journal");
    assert!(after.starts_with(&before));
    let appended = &after[before.len()..];
    let key_checking =
        ledgeline_core::feeds::normalize::account_short(&id, "ACT-lakeside-checking");
    let key_visa = ledgeline_core::feeds::normalize::account_short(&id, "ACT-harbor-visa");
    let expected = format!(
        "\n2026-07-06 Acme Corp  ; id:{c}-LCU-7001\n    assets:bank:checking       $4,200.00\n    expenses:unknown          $-4,200.00\n\n\
         2026-07-08 PG&E Utilities  ; id:{c}-LCU-7002\n    assets:bank:checking         $-96.40\n    expenses:unknown              $96.40\n\n\
         2026-07-10=2026-07-09 Trader Joe's  ; id:{c}-LCU-7003\n    assets:bank:checking         $-64.18\n    expenses:unknown              $64.18\n\n\
         2026-07-14 Venmo  ; id:{c}-LCU-7004\n    assets:bank:checking         $-40.00\n    expenses:unknown              $40.00\n\n\
         2026-07-09 Shell  ; id:{v}-HCS-502\n    liabilities:cc:visa         $-48.10\n    expenses:unknown             $48.10\n\n\
         2026-07-12 Amazon  ; id:{v}-HCS-503\n    liabilities:cc:visa         $-23.99\n    expenses:unknown             $23.99\n",
        c = key_checking,
        v = key_visa
    );
    assert_eq!(appended, expected, "appended:\n{appended}");

    // The rules files, the CSVs (no `.latest` beside them), the config.
    let rules = tree.read("checking.sync.csv.rules");
    assert!(rules.contains("currency $\n"), "{rules}");
    assert!(rules.ends_with("if %id .\n  comment id:%id\n"), "{rules}");
    let csvs = files_under(&tree.path("imports/sync"));
    assert_eq!(csvs.len(), 2, "{csvs:?}");
    assert!(csvs.iter().all(|f| f.ends_with(".csv")), "{csvs:?}");
    assert!(
        csvs.iter()
            .any(|f| f.contains("/simplefin-bridge/act-lakeside-checking-"))
    );
    let toml = tree.read("ledgeline.toml");
    assert!(toml.starts_with(LEDGER_TOML), "[ui] untouched:\n{toml}");
    let config = sync_config(&toml);
    assert_eq!(config.accounts.len(), 2);
    assert_eq!(config.accounts[0].rules, "checking.sync.csv.rules");
    assert_eq!(config.accounts[0].start_date, "2026-07-04");

    // The status: balances match, the held row is listed with its action.
    let status = tree.status().await;
    let checking = account_in(&status, "ACT-lakeside-checking");
    assert_eq!(
        checking["bankBalance"],
        json!({"amount": "32292.23", "date": "2026-07-15"})
    );
    assert_eq!(checking["balanceDiff"], Value::Null, "{checking}");
    assert_eq!(checking["ledgerBalance"], "$32,292.23");
    assert_eq!(checking["history"][0]["outcome"], "added");
    assert_eq!(checking["dataThrough"], "2026-07-14");
    let visa = account_in(&status, "ACT-harbor-visa");
    assert_eq!(visa["balanceDiff"], Value::Null, "{visa}");
    assert_eq!(visa["held"][0]["payee"], "Safeway");
    assert_eq!(visa["held"][0]["existing"]["date"], "2026-07-03");
    assert_eq!(visa["problems"][0]["code"], "held");
    assert_eq!(visa["problems"][0]["action"]["kind"], "review-held");
    assert_eq!(status["problemCount"], json!({"warning": 1, "error": 0}));
    assert_eq!(status["unlinked"].as_array().map(Vec::len), Some(1));
    // Shape: every contract field is there.
    for field in [
        "key",
        "connectionId",
        "remoteId",
        "label",
        "org",
        "journalAccount",
        "target",
        "rules",
        "categories",
        "startDate",
        "balanceSign",
        "enabled",
        "lastSuccess",
        "dataThrough",
        "lastAdded",
        "bankBalance",
        "ledgerBalance",
        "balanceDiff",
        "held",
        "history",
        "problems",
        "rulesChoices",
    ] {
        assert!(checking.get(field).is_some(), "{field} missing");
    }
    assert_eq!(checking["org"], "Lakeside Credit Union");
}

#[tokio::test]
async fn a_second_sync_adds_nothing_and_overlaps_five_days() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    tree.first_sync_at_now(&id).await;
    let after_first = tree.read("sample.journal");
    let requests = tree.feed.requests();

    let run = tree.state.run_sync_now(NOW + DAY).await;
    assert_eq!(run["ok"], true, "{run}");
    assert!(
        run["accounts"]
            .as_array()
            .expect("accounts")
            .iter()
            .all(|a| a["outcome"] == "nothing-new" && a["added"] == 0),
        "{run}"
    );
    assert_eq!(tree.read("sample.journal"), after_first, "idempotent");
    // One request, starting five days before the last success.
    let queries = tree.feed.queries();
    assert_eq!(queries.len(), requests + 1);
    let last = queries.last().expect("query");
    assert_eq!(
        last.start_date,
        Some(NOW - 5 * DAY),
        "five days before the last success"
    );

    // A row that posts late, dated inside the overlap, still lands.
    tree.feed.edit(|batch| {
        let checking = batch
            .accounts
            .iter_mut()
            .find(|a| a.id == "ACT-lakeside-checking")
            .expect("checking");
        let mut late = checking.transactions[1].clone();
        late.id = "LCU-7006".into();
        late.amount = "-7.25".into();
        late.description = "CORNER BAKERY".into();
        late.payee = Some("Corner Bakery".into());
        late.posted = 1_783_900_800; // 2026-07-13
        checking.transactions.push(late);
    });
    let run = tree.state.run_sync_now(NOW + 2 * DAY).await;
    let checking = run["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["added"] == 1)
        .expect("one added");
    assert_eq!(checking["outcome"], "added");
    assert!(
        tree.read("sample.journal")
            .contains("2026-07-13 Corner Bakery")
    );
}

#[tokio::test]
async fn reissued_ids_are_held_not_duplicated() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    tree.first_sync_at_now(&id).await;
    let after_first = tree.read("sample.journal");
    // The bank re-issues every checking id (Capital One did).
    tree.feed.edit(|batch| {
        for txn in &mut batch
            .accounts
            .iter_mut()
            .find(|a| a.id == "ACT-lakeside-checking")
            .expect("checking")
            .transactions
        {
            txn.id = format!("NEW-{}", txn.id);
        }
    });
    // The window reaches five days back from the last success: the one
    // re-issued row in it (Venmo, 2026-07-14) is held, not imported twice.
    let run = tree.state.run_sync_now(NOW + 60).await;
    let checking = run["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["held"] == 1)
        .unwrap_or_else(|| panic!("one held: {run}"));
    assert_eq!(checking["added"], 0, "{run}");
    assert_eq!(tree.read("sample.journal"), after_first, "nothing written");
}

#[tokio::test]
async fn held_rows_can_be_imported_or_marked_as_duplicates() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    tree.first_sync_at_now(&id).await;
    let key = ledgeline_core::feeds::normalize::account_short(&id, "ACT-harbor-visa");
    let held_id = format!("{key}-HCS-501");

    // It's a duplicate: the journal's Safeway gets the bank's id too.
    let (status, body) = tree
        .call(
            "POST",
            &format!("/api/sync/held/{key}/{held_id}"),
            Some(json!({"action": "duplicate"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let journal = tree.read("sample.journal");
    assert!(
        journal.contains(&format!(
            "2026-07-03 Safeway | weekly groceries  ; id:{held_id}\n"
        )),
        "{journal}"
    );
    assert_eq!(account_in(&body, "ACT-harbor-visa")["held"], json!([]));
    // And the next sync knows it by id.
    let run = tree.state.run_sync_now(NOW + DAY).await;
    assert!(
        run["accounts"]
            .as_array()
            .expect("a")
            .iter()
            .all(|a| a["held"] == 0),
        "{run}"
    );

    // Import anyway, on a fresh tree.
    let tree = Tree::new();
    let id = tree.connect().await;
    tree.first_sync_at_now(&id).await;
    let key = ledgeline_core::feeds::normalize::account_short(&id, "ACT-harbor-visa");
    let held_id = format!("{key}-HCS-501");
    let (status, body) = tree
        .call(
            "POST",
            &format!("/api/sync/held/{key}/{held_id}"),
            Some(json!({"action": "import"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        tree.read("sample.journal")
            .contains(&format!("2026-07-05 Safeway  ; id:{held_id}"))
    );
    assert_eq!(account_in(&body, "ACT-harbor-visa")["held"], json!([]));
    let (status, _) = tree
        .call(
            "POST",
            &format!("/api/sync/held/{key}/{held_id}"),
            Some(json!({"action": "import"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "resolved once");
}

#[tokio::test]
async fn a_conflict_mid_run_is_skipped_quietly_and_retried() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    // Link only checking, by hand, with its rules file, so the first sync is
    // a plain Sync now.
    let rules = ledgeline_core::rules::generate::feed_rules(
        &ledgeline_core::rules::generate::FeedRulesParams {
            label: "Checking",
            provider: "SimpleFIN",
            columns: &ledgeline_core::feeds::normalize::FeedColumns::default(),
            journal_account: "assets:bank:checking",
            fallback_account: "expenses:unknown",
            categories_include: None,
            currency: Some("$"),
        },
    );
    std::fs::write(tree.path("checking.sync.csv.rules"), rules).expect("rules");
    let mut toml = tree.read("ledgeline.toml");
    toml.push_str(&format!(
        "\n[[sync.account]]\nconnection = \"{id}\"\nremoteId = \"ACT-lakeside-checking\"\n\
         label = \"Checking\"\njournalAccount = \"assets:bank:checking\"\nrules = \"checking.sync.csv.rules\"\n\
         target = \"sample.journal\"\nstartDate = 2026-07-04\nbalanceSign = 1\nenabled = true\n"
    ));
    std::fs::write(tree.path("ledgeline.toml"), toml).expect("toml");
    let before = tree.read("sample.journal");

    tree.state.inject_sync_conflicts(1);
    let run = tree.state.run_sync_now(NOW).await;
    assert_eq!(run["accounts"][0]["outcome"], "skipped", "{run}");
    assert_eq!(run["accounts"][0]["problem"], Value::Null, "quiet");
    assert_eq!(tree.read("sample.journal"), before);

    let run = tree.state.run_sync_now(NOW + 600).await;
    assert_eq!(run["accounts"][0]["outcome"], "added", "{run}");
    assert_eq!(run["accounts"][0]["added"], 4);
}

#[tokio::test]
async fn auto_sync_runs_when_due_and_not_again_until_the_next() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    tree.link_both(&id).await;
    // Turn auto-sync on for this ledger, the way a pin does.
    let mut toml = tree.read("ledgeline.toml");
    toml = toml.replace(
        "theme = \"nord\" # keep\n",
        "theme = \"nord\" # keep\nautoSync = true\n",
    );
    std::fs::write(tree.path("ledgeline.toml"), toml).expect("toml");
    let requests = tree.feed.requests();
    // Linked just now (wall clock), so a tick at that time isn't due…
    let now = ledgeline_core::reports::periods::days_from_iso("2026-07-17") * DAY;
    let last = tree.status().await["connections"][0]["lastSuccess"]
        .as_i64()
        .expect("success");
    tree.state.run_sync_once(last + 60).await;
    assert_eq!(tree.feed.requests(), requests, "not due a minute later");
    // …but a day later it is.
    tree.state.run_sync_once(last.max(now) + DAY).await;
    assert_eq!(tree.feed.requests(), requests + 1);
}

#[tokio::test]
async fn diagnostics_never_hold_the_access_url() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    tree.link_both(&id).await;
    let request = Request::builder()
        .uri(format!("/api/sync/connections/{id}/diagnostics"))
        .body(Body::empty())
        .expect("request");
    let response = tree.router().oneshot(request).await.expect("responds");
    assert_eq!(response.status(), StatusCode::OK);
    let text = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .expect("b")
            .to_bytes()
            .to_vec(),
    )
    .expect("utf8");
    assert!(text.contains("host: fake.simplefin.test"), "{text}");
    assert!(
        !text.contains("secret") && !text.contains("user:"),
        "{text}"
    );
}

#[tokio::test]
async fn sharing_a_manual_file_previews_and_prints_identically() {
    require_hledger!();
    let tree = Tree::new();
    let shared = common::fixtures_dir().join("import/shared-categories");
    std::fs::create_dir_all(tree.path("imports")).expect("dir");
    for name in ["wells-ofx.csv.rules", "wells-ofx.csv"] {
        std::fs::copy(shared.join(name), tree.path(&format!("imports/{name}"))).expect("copy");
    }
    let (status, body) = tree
        .call(
            "POST",
            "/api/sync/share/preview",
            Some(json!({"manualRulesId": "imports/wells-ofx.csv.rules", "journalAccount": "assets:bank:checking"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["categoriesId"], "imports/wells-ofx.categories.rules");
    assert_eq!(body["moved"], 6);
    assert_eq!(body["identical"], true, "{body}");
    assert_eq!(body["differences"], Value::Null);
    let kept: Vec<&str> = body["kept"]
        .as_array()
        .expect("kept")
        .iter()
        .filter_map(|k| k["reason"].as_str())
        .collect();
    assert!(
        kept.contains(&"Sets the transaction comment, which sync uses for its id."),
        "{kept:?}"
    );
    let manual_diff = body["diffs"]["manual"].as_str().expect("diff");
    assert!(manual_diff.contains("-if WHOLE FOODS"), "{manual_diff}");
    assert!(
        manual_diff.contains("+include wells-ofx.categories.rules"),
        "{manual_diff}"
    );
    assert!(
        body["diffs"]["categories"]
            .as_str()
            .expect("diff")
            .contains("+if WHOLE FOODS")
    );
    // A preview writes nothing.
    assert!(!tree.path("imports/wells-ofx.categories.rules").exists());
}

#[tokio::test]
async fn a_link_preview_counts_and_shows_entries_but_writes_nothing() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    let before = tree.read("sample.journal");
    let (status, body) = tree
        .call(
            "POST",
            "/api/sync/link/preview",
            Some(json!({"links": [
                link_request(&id, "ACT-lakeside-checking", "assets:bank:checking", "checking.sync.csv.rules"),
                link_request(&id, "ACT-harbor-visa", "liabilities:cc:visa", "visa.sync.csv.rules"),
            ]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let checking = &body["accounts"][0];
    assert_eq!(checking["ok"], true, "{body}");
    assert_eq!(checking["new"], 4);
    assert_eq!(checking["uncategorized"], 4);
    assert!(
        checking["entries"]
            .as_str()
            .is_some_and(|e| e.contains("Acme Corp"))
    );
    let visa = &body["accounts"][1];
    assert_eq!(
        (visa["new"].clone(), visa["held"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(tree.read("sample.journal"), before);
    assert!(!tree.path("checking.sync.csv.rules").exists());
    assert_eq!(sync_config(&tree.read("ledgeline.toml")).accounts.len(), 0);
    // A second preview within ten minutes reuses the fetch.
    let requests = tree.feed.requests();
    let _ = tree
        .call(
            "POST",
            "/api/sync/link/preview",
            Some(json!({"links": [link_request(&id, "ACT-lakeside-checking", "assets:bank:checking", "checking.sync.csv.rules")]})),
        )
        .await;
    assert_eq!(tree.feed.requests(), requests, "no new request");

    // Linking straight after the preview imports from the same fetch: the
    // wizard shouldn't spend two of the Bridge's ~24 daily requests.
    let (status, run) = tree
        .call(
            "POST",
            "/api/sync/link",
            Some(json!({"links": [
                link_request(&id, "ACT-lakeside-checking", "assets:bank:checking", "checking.sync.csv.rules"),
                link_request(&id, "ACT-harbor-visa", "liabilities:cc:visa", "visa.sync.csv.rules"),
            ]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["accounts"][0]["added"], 4, "{run}");
    assert_eq!(
        tree.feed.requests(),
        requests,
        "link reuses the preview's fetch"
    );

    // Sync now always asks the bank again.
    let _ = tree.state.run_sync_now(NOW).await;
    assert_eq!(tree.feed.requests(), requests + 1);
}

#[tokio::test]
async fn a_new_account_gets_its_declaration_and_opening_balance_in_the_target() {
    require_hledger!();
    let tree = Tree::new();
    let id = tree.connect().await;
    let mut request = link_request(
        &id,
        "ACT-summit-savings",
        "assets:bank:summit-online-bank:high-yield-savings",
        "savings.sync.csv.rules",
    );
    request["newAccount"] =
        json!({"type": "C", "openingBalance": "20000.00", "openingDate": "2026-06-30"});
    request["startDate"] = json!("2026-07-01");
    let (status, run) = tree
        .call("POST", "/api/sync/link", Some(json!({"links": [request]})))
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["accounts"][0]["added"], 1, "{run}");
    let journal = tree.read("sample.journal");
    assert!(
        journal.contains(
            "\naccount assets:bank:summit-online-bank:high-yield-savings  ; type:C\n\n\
             2026-06-30 Opening balance\n    assets:bank:summit-online-bank:high-yield-savings  $20000.00\n    equity:opening balances\n"
        ),
        "{journal}"
    );
    assert!(
        journal.contains("2026-07-01 Summit Online Bank  ; id:"),
        "{journal}"
    );
    let status = tree.status().await;
    let savings = account_in(&status, "ACT-summit-savings");
    assert_eq!(
        savings["balanceDiff"],
        Value::Null,
        "balance matches: {savings}"
    );
    assert_eq!(savings["ledgerBalance"], "$20,041.10");
}

#[tokio::test]
async fn linking_with_share_moves_the_categories_and_includes_them_from_both() {
    require_hledger!();
    let tree = Tree::new();
    let shared = common::fixtures_dir().join("import/shared-categories");
    std::fs::create_dir_all(tree.path("imports")).expect("dir");
    for name in ["wells-ofx.csv.rules", "wells-ofx.csv"] {
        std::fs::copy(shared.join(name), tree.path(&format!("imports/{name}"))).expect("copy");
    }
    let id = tree.connect().await;
    let mut request = link_request(
        &id,
        "ACT-lakeside-checking",
        "assets:bank:checking",
        "checking.sync.csv.rules",
    );
    request["share"] = json!({"manualRulesId": "imports/wells-ofx.csv.rules"});
    let (status, run) = tree
        .call("POST", "/api/sync/link", Some(json!({"links": [request]})))
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["accounts"][0]["outcome"], "added", "{run}");
    let manual = tree.read("imports/wells-ofx.csv.rules");
    assert!(
        manual.ends_with("\ninclude wells-ofx.categories.rules\n"),
        "{manual}"
    );
    let categories = tree.read("imports/wells-ofx.categories.rules");
    assert!(categories.contains("if WHOLE FOODS"), "{categories}");
    // The sync file went beside the manual one, borrowing its names.
    let sync = tree.read("imports/checking.sync.csv.rules");
    assert!(
        sync.contains("fields date, amount, name, sfdescription, memo, id, transacted\n"),
        "{sync}"
    );
    assert!(
        sync.ends_with("include wells-ofx.categories.rules\nif %id .\n  comment id:%id\n"),
        "{sync}"
    );
    let config = sync_config(&tree.read("ledgeline.toml"));
    assert_eq!(config.accounts[0].rules, "imports/checking.sync.csv.rules");
    let status = tree.status().await;
    assert_eq!(
        account_in(&status, "ACT-lakeside-checking")["categories"],
        "imports/wells-ofx.categories.rules"
    );
}
