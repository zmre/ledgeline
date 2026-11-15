//! Talking to a bank feed (plan 43 §3.1, contract WP-S "HTTP" and "Claim").
//!
//! [`FeedProvider`] is the seam: the engine asks it to claim a setup token
//! and to fetch `/accounts`, and never sees HTTP. [`SimpleFinClient`] is the
//! real one; tests and the e2e harness swap in fakes the way prices swap
//! `PriceFeed`.
//!
//! # What the real client promises
//!
//! - **Credentials only in a header.** The access URL is split by
//!   `AccessUrl::parse`; the request goes to its credential-free `base`, with
//!   Basic auth. No URL this module builds ever holds a username or password,
//!   so no error message, log line or redirect can leak one.
//! - **No redirects.** A 3xx is an error. A redirect is the one way a
//!   credential could be carried somewhere else, and SimpleFIN never sends one.
//! - **HTTPS only**, with certificates verified (rustls, the platform roots
//!   first, as `yahoo::http_client` does). Plain `http://` is accepted only
//!   for a loopback host, and only on a state built with
//!   `AppState::with_sync_loopback_http` (tests).
//! - **Our own User-Agent**, `Ledgeline/<version> (+https://github.com/zmre/ledgeline)`.
//!   SimpleFIN Bridge's Cloudflare edge answers 403 to agents it doesn't know,
//!   which looks exactly like "token already used"; [`FeedError::EdgeBlocked`]
//!   tells the two apart by the `cf-ray` header and an HTML body.
//! - **60 s timeout**, so a hung connection is a failed sync, not a stuck one.
//! - **Never `pending=1`, always `version=2`**, and `account=` repeated when a
//!   run is for some accounts only.

use std::time::Duration;

use async_trait::async_trait;
use ledgeline_core::feeds::FeedBatch;
use ledgeline_core::feeds::simplefin::{AccessUrl, SetupToken, parse_accounts};

/// The User-Agent every SimpleFIN request sends.
pub(crate) const USER_AGENT: &str = concat!(
    "Ledgeline/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/zmre/ledgeline)"
);

/// How long one request may take.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// What one `/accounts` request asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountsQuery {
    /// `start-date`, epoch seconds.
    pub start_date: Option<i64>,
    /// `end-date`, epoch seconds (exclusive).
    pub end_date: Option<i64>,
    /// `balances-only=1`: accounts and balances, no transactions.
    pub balances_only: bool,
    /// `account=` filters; empty = every account.
    pub accounts: Vec<String>,
}

impl AccountsQuery {
    /// The query string pairs, in a stable order.
    #[must_use]
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = vec![("version", "2".to_string())];
        if let Some(start) = self.start_date {
            pairs.push(("start-date", start.to_string()));
        }
        if let Some(end) = self.end_date {
            pairs.push(("end-date", end.to_string()));
        }
        if self.balances_only {
            pairs.push(("balances-only", "1".to_string()));
        }
        pairs.extend(self.accounts.iter().map(|id| ("account", id.clone())));
        pairs
    }
}

/// Why a request failed. None of these carries a URL or a credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeedError {
    /// The connection couldn't be made or broke.
    #[error("couldn't reach the server ({0})")]
    Network(String),
    /// No answer within the timeout.
    #[error("the server took too long to answer")]
    Timeout,
    /// A 5xx.
    #[error("the server had an error (HTTP {0})")]
    Server(u16),
    /// 402: the subscription lapsed.
    #[error("payment required (HTTP 402)")]
    Payment,
    /// 403 on `/accounts`: access revoked or disabled.
    #[error("access refused (HTTP 403)")]
    Forbidden,
    /// 403 from the claim endpoint itself: the token was already claimed.
    #[error("the setup token was already used (HTTP 403)")]
    TokenUsed,
    /// 403 from the Cloudflare edge in front of the server.
    #[error("refused by the server's firewall (HTTP 403)")]
    EdgeBlocked,
    /// A 3xx, which is never followed.
    #[error("the server answered with a redirect (HTTP {0}), which Ledgeline doesn't follow")]
    Redirect(u16),
    /// Any other unexpected status.
    #[error("unexpected answer (HTTP {0})")]
    Status(u16),
    /// A 200 whose body isn't what the protocol says.
    #[error("{0}")]
    BadResponse(String),
}

impl FeedError {
    /// Whether waiting and retrying may help: the backoff applies.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Timeout | Self::Server(_))
    }
}

/// A bank feed. `Send + Sync` and `dyn`, so `AppState` can hold one and tests
/// can hand it a fake.
#[async_trait]
pub trait FeedProvider: Send + Sync {
    /// Exchange a setup token for an access URL: `POST` the claim URL once,
    /// and return the body (the access URL, as text, still to be parsed).
    ///
    /// # Errors
    /// [`FeedError`]; a 403 is [`FeedError::TokenUsed`] or
    /// [`FeedError::EdgeBlocked`].
    async fn claim(&self, token: &SetupToken) -> Result<String, FeedError>;

    /// `GET {base}/accounts` with `query`.
    ///
    /// # Errors
    /// [`FeedError`].
    async fn accounts(
        &self,
        access: &AccessUrl,
        query: &AccountsQuery,
    ) -> Result<FeedBatch, FeedError>;

    /// The symbol of a SimpleFIN custom currency (points, miles) described
    /// at `url`, or `None` when there isn't a usable one. Only ever asked of
    /// a URL on the connection's own host (plan 43 §7); the default answers
    /// `None`, which the caller turns into the commodity `?` and a warning.
    ///
    /// # Errors
    /// [`FeedError`] for a failed request.
    async fn currency_symbol(
        &self,
        _access: &AccessUrl,
        _url: &str,
    ) -> Result<Option<String>, FeedError> {
        Ok(None)
    }
}

/// A custom currency's `abbr` (or `name`) as a commodity symbol: letters
/// only, at most 12, or `None`. The text is the server's, so nothing else
/// survives into a rules file.
#[must_use]
pub fn currency_symbol_from(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    ["abbr", "name"].iter().find_map(|key| {
        let text = value.get(key)?.as_str()?;
        let symbol: String = text
            .chars()
            .filter(|c| c.is_alphabetic())
            .take(12)
            .collect();
        (!symbol.is_empty()).then_some(symbol)
    })
}

/// Whether `url` is on `access`'s own host (and scheme), so a currency may be
/// fetched from it without sending anything anywhere new.
#[must_use]
pub fn same_host(access: &AccessUrl, url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let scheme = access.base.split("://").next().unwrap_or_default();
    parsed.scheme() == scheme
        && parsed.host_str() == Some(access.host.as_str())
        && parsed.username().is_empty()
        && parsed.password().is_none()
}

/// The real SimpleFIN client.
pub struct SimpleFinClient {
    client: reqwest::Client,
}

impl SimpleFinClient {
    /// A client that refuses plain `http://` unless `allow_loopback_http`
    /// (tests), with the standard timeout.
    #[must_use]
    pub fn new(allow_loopback_http: bool) -> Self {
        Self::with_timeout(allow_loopback_http, REQUEST_TIMEOUT)
    }

    /// [`Self::new`] with a different timeout (tests).
    #[must_use]
    pub(crate) fn with_timeout(allow_loopback_http: bool, timeout: Duration) -> Self {
        let builder = || {
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .https_only(!allow_loopback_http)
        };
        let client = builder()
            .build()
            .or_else(|_| {
                builder()
                    .tls_certs_only(crate::yahoo::bundled_roots())
                    .build()
            })
            .unwrap_or_default();
        Self { client }
    }
}

/// A transport failure as a [`FeedError`], with no URL in it.
fn transport(error: &reqwest::Error) -> FeedError {
    if error.is_timeout() {
        FeedError::Timeout
    } else if error.is_connect() {
        FeedError::Network("connection failed".to_string())
    } else if error.is_redirect() {
        FeedError::Redirect(0)
    } else if error.is_builder() {
        FeedError::Network("the address isn't usable".to_string())
    } else {
        FeedError::Network("the request failed".to_string())
    }
}

/// Whether a 403 came from Cloudflare's edge rather than the server: a
/// `cf-ray` header and an HTML body.
fn is_edge_block(headers: &reqwest::header::HeaderMap, body: &[u8]) -> bool {
    let html_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("html"));
    let html_body = body
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| *b == b'<');
    headers.contains_key("cf-ray") && (html_type || html_body)
}

/// Read a response, mapping the statuses every endpoint shares.
async fn read(
    response: reqwest::Response,
) -> Result<(u16, reqwest::header::HeaderMap, Vec<u8>), FeedError> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response
        .bytes()
        .await
        .map_err(|error| transport(&error))?
        .to_vec();
    match status {
        200..=299 => Ok((status, headers, body)),
        300..=399 => Err(FeedError::Redirect(status)),
        402 => Err(FeedError::Payment),
        500..=599 => Err(FeedError::Server(status)),
        _ => Ok((status, headers, body)),
    }
}

#[async_trait]
impl FeedProvider for SimpleFinClient {
    async fn claim(&self, token: &SetupToken) -> Result<String, FeedError> {
        let response = self
            .client
            .post(&token.claim_url)
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .send()
            .await
            .map_err(|error| transport(&error))?;
        let (status, headers, body) = read(response).await?;
        match status {
            200..=299 => String::from_utf8(body)
                .map(|text| text.trim().to_string())
                .map_err(|_| FeedError::BadResponse("SimpleFIN's answer isn't text.".into())),
            403 if is_edge_block(&headers, &body) => Err(FeedError::EdgeBlocked),
            403 => Err(FeedError::TokenUsed),
            other => Err(FeedError::Status(other)),
        }
    }

    async fn accounts(
        &self,
        access: &AccessUrl,
        query: &AccountsQuery,
    ) -> Result<FeedBatch, FeedError> {
        let response = self
            .client
            .get(access.endpoint("accounts"))
            .basic_auth(&access.username, Some(&access.password))
            .query(&query.pairs())
            .send()
            .await
            .map_err(|error| transport(&error))?;
        let (status, headers, body) = read(response).await?;
        match status {
            200..=299 => {
                parse_accounts(&body).map_err(|error| FeedError::BadResponse(error.to_string()))
            }
            403 if is_edge_block(&headers, &body) => Err(FeedError::EdgeBlocked),
            403 => Err(FeedError::Forbidden),
            other => Err(FeedError::Status(other)),
        }
    }

    async fn currency_symbol(
        &self,
        access: &AccessUrl,
        url: &str,
    ) -> Result<Option<String>, FeedError> {
        if !same_host(access, url) {
            return Ok(None);
        }
        // No credentials: the description is public, and nothing but the
        // access URL's own endpoints ever gets the Basic auth header.
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| transport(&error))?;
        let (status, _, body) = read(response).await?;
        Ok((200..300)
            .contains(&status)
            .then(|| currency_symbol_from(&body))
            .flatten())
    }
}

// ---------------------------------------------------------------------------
// The fake bridge (`--features test-fakes`, `LEDGELINE_FEED_FAKE=<dir>`)
// ---------------------------------------------------------------------------

/// The host a fake setup token must decode to.
#[cfg(feature = "test-fakes")]
pub const FAKE_HOST: &str = "fake.simplefin.test";

/// A provider that never touches the network: `<dir>/accounts.json`, read on
/// every request, filtered by the query the way the Bridge filters (by posted
/// date, by account, balances only). For e2e and docs screenshots.
#[cfg(feature = "test-fakes")]
pub struct FakeBridge {
    dir: std::path::PathBuf,
}

#[cfg(feature = "test-fakes")]
impl FakeBridge {
    /// Serve `dir/accounts.json`.
    #[must_use]
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

/// `batch` as the Bridge would answer `query`: only the asked accounts, only
/// transactions posted in `[start, end)`, none at all for balances only.
/// Shared by the fake bridge and the tests' own fakes.
#[must_use]
pub fn filter_batch(mut batch: FeedBatch, query: &AccountsQuery) -> FeedBatch {
    if !query.accounts.is_empty() {
        batch
            .accounts
            .retain(|account| query.accounts.contains(&account.id));
    }
    for account in &mut batch.accounts {
        if query.balances_only {
            account.transactions.clear();
        } else {
            account.transactions.retain(|txn| {
                query.start_date.is_none_or(|start| txn.posted >= start)
                    && query.end_date.is_none_or(|end| txn.posted < end)
            });
        }
    }
    batch
}

#[cfg(feature = "test-fakes")]
#[async_trait]
impl FeedProvider for FakeBridge {
    async fn claim(&self, token: &SetupToken) -> Result<String, FeedError> {
        if token.host == FAKE_HOST {
            Ok(format!("https://fake:fake@{FAKE_HOST}/simplefin"))
        } else {
            Err(FeedError::TokenUsed)
        }
    }

    async fn accounts(
        &self,
        _access: &AccessUrl,
        query: &AccountsQuery,
    ) -> Result<FeedBatch, FeedError> {
        let body = std::fs::read(self.dir.join("accounts.json"))
            .map_err(|error| FeedError::BadResponse(format!("fake bridge: {}", error.kind())))?;
        let batch =
            parse_accounts(&body).map_err(|error| FeedError::BadResponse(error.to_string()))?;
        Ok(filter_batch(batch, query))
    }
}

// ---------------------------------------------------------------------------
// Tests: the real reqwest client against an in-process axum "SimpleFIN"
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, Uri, header};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use ledgeline_core::feeds::ProblemKind;
    use ledgeline_core::feeds::simplefin::decode_setup_token_with;
    use std::sync::{Arc, Mutex};

    /// What the fake server saw.
    #[derive(Default)]
    struct Seen {
        uris: Vec<String>,
        headers: Vec<HeaderMap>,
    }

    /// What the fake server answers.
    #[derive(Clone, Copy)]
    enum Script {
        Ok,
        Forbidden,
        CloudflareForbidden,
        Payment,
        ServerError,
        Slow,
        Redirect,
        ConAuth,
        Quota,
        RangeCap,
    }

    #[derive(Clone)]
    struct Fake {
        script: Script,
        seen: Arc<Mutex<Seen>>,
        port: u16,
    }

    const V2_OK: &str = r#"{"errlist":[],"connections":[{"conn_id":"CON-1","name":"First Tech","org_id":"o1","org_url":"https://firsttech.example"}],
      "accounts":[{"id":"ACT-1","name":"Checking","conn_id":"CON-1","currency":"USD","balance":"100.25","balance-date":1794268800,
      "transactions":[{"id":"T1","posted":1794268800,"amount":"-4.50","description":"COFFEE","payee":"Coffee","memo":""}]}]}"#;

    fn record(fake: &Fake, uri: &Uri, headers: &HeaderMap) {
        let mut seen = fake.seen.lock().expect("lock");
        seen.uris.push(uri.to_string());
        seen.headers.push(headers.clone());
    }

    async fn claim_handler(State(fake): State<Fake>, uri: Uri, headers: HeaderMap) -> Response {
        record(&fake, &uri, &headers);
        match fake.script {
            Script::Forbidden => (StatusCode::FORBIDDEN, "Forbidden").into_response(),
            Script::CloudflareForbidden => (
                StatusCode::FORBIDDEN,
                [("cf-ray", "8abc-SJC"), ("content-type", "text/html")],
                "<!DOCTYPE html><title>Attention Required!</title>",
            )
                .into_response(),
            _ => {
                format!("http://user%40x:p%3Ass@127.0.0.1:{}/simplefin", fake.port).into_response()
            }
        }
    }

    async fn accounts_handler(State(fake): State<Fake>, uri: Uri, headers: HeaderMap) -> Response {
        record(&fake, &uri, &headers);
        let body = |text: &str| {
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                text.to_string(),
            )
                .into_response()
        };
        match fake.script {
            Script::Ok => body(V2_OK),
            Script::Forbidden => (StatusCode::FORBIDDEN, "nope").into_response(),
            Script::CloudflareForbidden => (
                StatusCode::FORBIDDEN,
                [
                    ("cf-ray", "8abc-SJC"),
                    ("content-type", "text/html; charset=UTF-8"),
                ],
                "<html>blocked</html>",
            )
                .into_response(),
            Script::Payment => (StatusCode::PAYMENT_REQUIRED, "pay").into_response(),
            Script::ServerError => (StatusCode::BAD_GATEWAY, "oops").into_response(),
            Script::Slow => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                body(V2_OK)
            }
            Script::Redirect => (
                StatusCode::FOUND,
                [(header::LOCATION, "https://evil.example/steal")],
            )
                .into_response(),
            Script::ConAuth => body(
                r#"{"errlist":[{"code":"con.auth","msg":"Please reauthenticate","conn_id":"CON-1"}],
                    "connections":[{"conn_id":"CON-1","name":"Chase"}],"accounts":[]}"#,
            ),
            Script::Quota => body(
                r#"{"errlist":[{"code":"gen.api","msg":"You are approaching your daily request quota"}],"accounts":[]}"#,
            ),
            Script::RangeCap => body(
                r#"{"errlist":[{"code":"gen.api","msg":"Date range limited to 90 days"}],"accounts":[]}"#,
            ),
        }
    }

    /// Serve `script` on 127.0.0.1 and return the base URL and what it saw.
    async fn serve(script: Script) -> (String, Arc<Mutex<Seen>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let fake = Fake {
            script,
            seen: Arc::clone(&seen),
            port,
        };
        let app = Router::new()
            .route("/simplefin/claim/{token}", post(claim_handler))
            .route("/simplefin/accounts", get(accounts_handler))
            .with_state(fake);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://127.0.0.1:{port}"), seen)
    }

    fn access(base: &str) -> AccessUrl {
        let with_creds = base.replacen("http://", "http://user%40x:p%3Ass@", 1);
        AccessUrl::parse(&format!("{with_creds}/simplefin"), true).expect("access url")
    }

    fn token(base: &str) -> SetupToken {
        use base64::Engine;
        let url = format!("{base}/simplefin/claim/ABC123");
        let encoded = base64::engine::general_purpose::STANDARD.encode(url);
        decode_setup_token_with(&encoded, true).expect("token decodes")
    }

    #[tokio::test]
    async fn a_claim_returns_the_access_url_from_an_empty_post() {
        let (base, seen) = serve(Script::Ok).await;
        let client = SimpleFinClient::new(true);
        let access_text = client.claim(&token(&base)).await.expect("claimed");
        let parsed = AccessUrl::parse(&access_text, true).expect("parses");
        assert_eq!(parsed.username, "user@x");
        assert_eq!(parsed.password, "p:ss");
        let seen = seen.lock().expect("lock");
        assert_eq!(seen.uris, ["/simplefin/claim/ABC123"]);
        assert_eq!(
            seen.headers[0]
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
            Some(USER_AGENT)
        );
        assert!(USER_AGENT.starts_with("Ledgeline/"));
    }

    #[tokio::test]
    async fn a_claim_403_is_token_used_unless_cloudflare_answered() {
        let (base, _) = serve(Script::Forbidden).await;
        let client = SimpleFinClient::new(true);
        assert_eq!(client.claim(&token(&base)).await, Err(FeedError::TokenUsed));
        let (base, _) = serve(Script::CloudflareForbidden).await;
        assert_eq!(
            client.claim(&token(&base)).await,
            Err(FeedError::EdgeBlocked)
        );
    }

    #[tokio::test]
    async fn accounts_sends_basic_auth_and_never_puts_credentials_in_the_url() {
        let (base, seen) = serve(Script::Ok).await;
        let client = SimpleFinClient::new(true);
        let query = AccountsQuery {
            start_date: Some(100),
            end_date: Some(200),
            balances_only: false,
            accounts: vec!["ACT-1".into(), "ACT-2".into()],
        };
        let batch = client.accounts(&access(&base), &query).await.expect("ok");
        assert_eq!(batch.accounts.len(), 1);
        assert_eq!(batch.accounts[0].transactions[0].amount, "-4.50");
        let seen = seen.lock().expect("lock");
        let uri = &seen.uris[0];
        assert_eq!(
            uri,
            "/simplefin/accounts?version=2&start-date=100&end-date=200&account=ACT-1&account=ACT-2"
        );
        assert!(!uri.contains("user") && !uri.contains("p%3Ass") && !uri.contains("pending"));
        use base64::Engine;
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("user@x:p:ss")
        );
        assert_eq!(
            seen.headers[0]
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok()),
            Some(expected.as_str())
        );
        assert_eq!(
            seen.headers[0]
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
            Some(USER_AGENT)
        );
    }

    #[tokio::test]
    async fn balances_only_asks_for_no_transactions() {
        let query = AccountsQuery {
            balances_only: true,
            ..AccountsQuery::default()
        };
        assert_eq!(
            query.pairs(),
            [
                ("version", "2".to_string()),
                ("balances-only", "1".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn statuses_map_to_their_errors() {
        let client = SimpleFinClient::new(true);
        for (script, expected) in [
            (Script::Forbidden, FeedError::Forbidden),
            (Script::CloudflareForbidden, FeedError::EdgeBlocked),
            (Script::Payment, FeedError::Payment),
            (Script::ServerError, FeedError::Server(502)),
            (Script::Redirect, FeedError::Redirect(302)),
        ] {
            let (base, _) = serve(script).await;
            assert_eq!(
                client
                    .accounts(&access(&base), &AccountsQuery::default())
                    .await,
                Err(expected)
            );
        }
        assert!(FeedError::Server(502).is_transient());
        assert!(FeedError::Timeout.is_transient());
        assert!(!FeedError::Forbidden.is_transient());
    }

    #[tokio::test]
    async fn a_redirect_is_never_followed() {
        let (base, seen) = serve(Script::Redirect).await;
        let client = SimpleFinClient::new(true);
        let _ = client
            .accounts(&access(&base), &AccountsQuery::default())
            .await;
        // One request, to our server; nothing went to the Location.
        assert_eq!(seen.lock().expect("lock").uris.len(), 1);
    }

    #[tokio::test]
    async fn a_slow_server_times_out() {
        let (base, _) = serve(Script::Slow).await;
        let client = SimpleFinClient::with_timeout(true, Duration::from_millis(200));
        assert_eq!(
            client
                .accounts(&access(&base), &AccountsQuery::default())
                .await,
            Err(FeedError::Timeout)
        );
    }

    #[tokio::test]
    async fn errlist_problems_come_back_in_the_batch() {
        let client = SimpleFinClient::new(true);
        let (base, _) = serve(Script::ConAuth).await;
        let batch = client
            .accounts(&access(&base), &AccountsQuery::default())
            .await
            .expect("ok");
        assert_eq!(batch.problems[0].kind, ProblemKind::ConAuth);
        assert_eq!(batch.problems[0].conn_id.as_deref(), Some("CON-1"));

        let (base, _) = serve(Script::Quota).await;
        let batch = client
            .accounts(&access(&base), &AccountsQuery::default())
            .await
            .expect("ok");
        assert_eq!(batch.problems[0].kind, ProblemKind::Quota);

        let (base, _) = serve(Script::RangeCap).await;
        let batch = client
            .accounts(&access(&base), &AccountsQuery::default())
            .await
            .expect("ok");
        assert_eq!(batch.problems[0].kind, ProblemKind::GenApi);
    }

    #[tokio::test]
    async fn plain_http_is_refused_without_the_loopback_escape() {
        let (base, seen) = serve(Script::Ok).await;
        let client = SimpleFinClient::new(false);
        assert!(
            client
                .accounts(&access(&base), &AccountsQuery::default())
                .await
                .is_err()
        );
        assert!(seen.lock().expect("lock").uris.is_empty());
    }

    #[test]
    fn custom_currency_symbols_are_letters_from_the_same_host_only() {
        assert_eq!(
            currency_symbol_from(br#"{"name":"Airline Miles","abbr":"MI LES 9!"}"#).as_deref(),
            Some("MILES")
        );
        assert_eq!(
            currency_symbol_from(br#"{"name":"Points"}"#).as_deref(),
            Some("Points")
        );
        assert_eq!(currency_symbol_from(br#"{"abbr":"123"}"#), None);
        assert_eq!(currency_symbol_from(b"nope"), None);
        let access = AccessUrl::parse("https://u:p@bridge.example/simplefin", false).expect("url");
        assert!(same_host(&access, "https://bridge.example/currency/points"));
        assert!(!same_host(&access, "https://evil.example/currency/points"));
        assert!(!same_host(&access, "http://bridge.example/currency/points"));
        assert!(!same_host(&access, "https://x:y@bridge.example/c"));
    }

    #[tokio::test]
    async fn a_currency_is_fetched_only_from_the_access_host() {
        let (base, seen) = serve(Script::Ok).await;
        let client = SimpleFinClient::new(true);
        // Another host: nothing is sent.
        assert_eq!(
            client
                .currency_symbol(&access(&base), "http://other.invalid/currency")
                .await,
            Ok(None)
        );
        assert!(seen.lock().expect("lock").uris.is_empty());
    }

    #[test]
    fn the_fake_filter_keeps_the_window_and_the_accounts() {
        let batch = parse_accounts(V2_OK.as_bytes()).expect("parses");
        let none = filter_batch(
            batch.clone(),
            &AccountsQuery {
                start_date: Some(1_794_268_801),
                ..AccountsQuery::default()
            },
        );
        assert!(none.accounts[0].transactions.is_empty());
        let other = filter_batch(
            batch,
            &AccountsQuery {
                accounts: vec!["ACT-9".into()],
                ..AccountsQuery::default()
            },
        );
        assert!(other.accounts.is_empty());
    }
}
