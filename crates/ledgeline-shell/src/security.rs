//! Access control and response hardening for the local HTTP server: a
//! per-process bearer token, a `Host`-header guard, an exact-origin CORS
//! allowlist, and the security headers (CSP included) every response carries.
//!
//! # Threat model — stated honestly
//!
//! Ledgeline serves a complete financial journal, readable *and writable*, over
//! plain HTTP on loopback. Until this module existed the only thing between that
//! journal and every web page the user had open was the port number, and
//! `CorsLayer::permissive()` gave even that away: any origin could read
//! `/transactions` and `POST`/`DELETE` `/api/transactions` (SEC-1).
//!
//! ## What the controls here genuinely stop
//!
//! * **A web page sweeping loopback ports.** With no CORS layer on the default
//!   path a page on `https://evil.example` (or `file://`, or another
//!   `http://localhost:*` app) still gets its request *sent*, but cannot read
//!   any reply — and because every wire and `/api` route now demands the token,
//!   it cannot land a blind write either. The token is a 256-bit random value it
//!   has no way to guess and, absent CORS, no way to read.
//! * **A page on an ALLOWLISTED origin reading the token.** `--allow-origin`
//!   opens the API to a named dev origin, and for a while it opened the SPA
//!   shell too, because the CORS layer was installed over the whole router
//!   including the fallback. The shell then carried the token in its body, so one
//!   `fetch("http://127.0.0.1:<port>/")` from any page on that origin — another
//!   Vite project, a malicious dev dependency — read it and had full read/write
//!   on the journal with no interaction (SEC-12). The layer is now a
//!   `route_layer` over the explicit routes only; see [`protect`].
//! * **DNS rebinding.** [`host_guard`] rejects any request whose `Host` is not a
//!   loopback name on the port we actually bound, so re-pointing
//!   `attacker.example.com` at `127.0.0.1` buys nothing.
//! * **Silent exfiltration if an XSS sink ever appears.** The `connect-src
//!   'self'` CSP means injected script cannot post the journal anywhere.
//!
//! ## What they do NOT stop — do not claim otherwise
//!
//! * **A malicious process running as the SAME user.** It can read (and rewrite)
//!   the journal file directly; no HTTP control is relevant.
//! * **Anyone who can read the `--server` terminal.** Headless mode prints the
//!   launch URL, token included, because something else has to drive it.
//! * **Anything at all once `--host` is pointed off loopback.** That path is
//!   gated on an explicitly configured token (`$LEDGELINE_TOKEN` for Ledgeline,
//!   [`AppIdentity::token_env`] in general) and turns the `Host` guard off,
//!   because the legitimate `Host` values are then unknowable.
//!
//! ## Closed: another local process scraping the token from `GET /`
//!
//! The SPA shell is served without a token (the browser has nothing to present
//! before it has loaded the page), and HTTP gives us no way to tell the user's
//! own WebView from someone else's `curl`. The shell used to carry the token in
//! its body for the browser to bootstrap, so any process that could reach
//! loopback — another local user's included — could `GET /` and read it. The
//! token now appears in no response body at all:
//!
//! * the GUI hands it to its WebView out of band, with wry's initialization
//!   script, guarded to our exact origin ([`token_init_script`]);
//! * `--server` prints a launch URL with the token in its `#token=` fragment,
//!   which a browser never puts on the wire; the SPA moves it into
//!   `sessionStorage` and strips it from the address bar.
//!
//! That exposure is what made the following worth writing down. This threat
//! model used to bound the worst case by "the journal file's own permissions",
//! which was **wrong** — wrong enough to be a finding in its own right, because
//! it is not bounded at all. Whoever holds the token holds `PUT /api/prefs`, and
//! `hledgerPath` there names the binary spawned on every import. So a token
//! scraped once bought arbitrary CODE EXECUTION as the user, surviving restarts,
//! from which the journal's permissions follow trivially (SEC-14).
//! Ledgeline's `prefs::check_storable` refuses a group- or world-writable binary and runs
//! the candidate before persisting it. Read that narrowly:
//!
//! * it removes the PERSISTENT half for anything that is not plausibly hledger
//!   — a script that prints nothing recognisable is refused and never written
//!   down, so it does not run on every import forever after;
//! * it does **not** remove execution. The probe *is* an `exec`, so
//!   `PUT /api/prefs` still runs whatever the caller names, exactly once (it
//!   used to be twice), before refusing it. Bounding that would mean not probing
//!   at all, and then a bad path is discovered at import time instead — which is
//!   the failure mode `prefs.rs` exists to prevent;
//! * and an attacker who owns a binary that DOES print an hledger version banner
//!   walks through all of it unchanged.
//!
//! So it is defence in depth for whoever already holds the token, and keeping
//! the token out of `GET /` is what keeps everyone else from holding it.
//!
//! The token is per-process and never persisted: restarting the app invalidates
//! it, and no other process is told what it is.
//!
//! # Shared by every app on the shell
//!
//! Nothing here is Ledgeline's alone. An app names its token variable and its
//! SPA's globals in an [`AppIdentity`], adds any request headers its uploads
//! carry to the CORS allowlist ([`Security::cors_layer`]), and puts the whole
//! posture on its router with [`protect`] (the explicit routes) and [`harden`]
//! (everything).

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::AppIdentity;

/// Entropy in a generated token: 256 bits, hex-encoded to 64 characters.
const TOKEN_BYTES: usize = 32;

/// Bounds on an externally supplied token. The lower bound keeps a careless
/// `LEDGELINE_TOKEN=dev` from being treated as a real control.
const MIN_TOKEN_LEN: usize = 16;
const MAX_TOKEN_LEN: usize = 128;

/// How long a browser may cache a CORS preflight for an allowlisted origin.
const PREFLIGHT_MAX_AGE: Duration = Duration::from_secs(600);

/// Policy up to and including the `script-src` allowlist's `'self'`; per-response
/// inline-script hashes are appended to this.
const CSP_HEAD: &str = "default-src 'self'; script-src 'self'";

/// Everything after `script-src`, shared by the shell and by every other
/// response. `connect-src 'self'` is the directive that neutralises the whole
/// exfiltration class; `style-src` needs `'unsafe-inline'` for the shell's
/// `style="display: contents"` attribute, and for the `<style>` element
/// CodeMirror's style-mod writes at run time in a document (the invoicer's
/// template editor; it uses `adoptedStyleSheets` only in a shadow root).
/// Dropping it would need a per-response nonce passed to `EditorView.cspNonce`.
const CSP_TAIL: &str = "; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; \
form-action 'none'; frame-ancestors 'none'";

/// Used only if the composed policy somehow will not fit in a header value —
/// strictly *tighter* than the real one, so a bug here fails closed.
const FALLBACK_CSP: &str =
    "default-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";

/// Failures building the security configuration.
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("generating an access token: {0}")]
    Random(#[from] getrandom::Error),
    #[error(
        "an access token must be {MIN_TOKEN_LEN}–{MAX_TOKEN_LEN} characters of [A-Za-z0-9._-] (it is embedded in a script and a URL, so the character set is restricted)"
    )]
    MalformedToken,
    /// [`MalformedToken`](Self::MalformedToken), from the environment
    /// variable `var` ([`token_from_env_or_random`]).
    #[error(
        "${var} must be {MIN_TOKEN_LEN}–{MAX_TOKEN_LEN} characters of [A-Za-z0-9._-] (it is embedded in a script and a URL, so the character set is restricted)"
    )]
    MalformedEnvToken { var: &'static str },
    #[error("--allow-origin {origin}: not a usable origin (expected e.g. http://localhost:5173)")]
    BadOrigin { origin: String },
}

/// A per-process bearer token. Never persisted, never logged except by explicit
/// request in `--server` mode, and compared in (best-effort) constant time.
#[derive(Clone)]
pub struct AccessToken(Arc<str>);

impl AccessToken {
    /// A fresh 256-bit token from the OS CSPRNG.
    ///
    /// # Errors
    /// [`SecurityError::Random`] if the platform entropy source fails.
    pub fn generate() -> Result<Self, SecurityError> {
        let mut bytes = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes)?;
        Ok(Self(hex(&bytes).into()))
    }

    /// Adopt a caller-supplied token, rejecting anything outside
    /// `[A-Za-z0-9._-]{16,128}`. The character set matters: the token is
    /// placed in the GUI's initialization script and in the `--server` launch
    /// URL's fragment, and a quote, `</script>` or `&` in either would be an
    /// injection or a mangled credential.
    ///
    /// # Errors
    /// [`SecurityError::MalformedToken`] if `raw` is too short, too long, or
    /// contains a character outside the allowed set.
    pub fn parse(raw: &str) -> Result<Self, SecurityError> {
        let well_formed = (MIN_TOKEN_LEN..=MAX_TOKEN_LEN).contains(&raw.len())
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        well_formed
            .then(|| Self(raw.into()))
            .ok_or(SecurityError::MalformedToken)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Best-effort constant-time comparison. The values are 256-bit random, so a
    /// timing oracle over loopback HTTP is not a practical attack; this only
    /// avoids handing one out for free. (Rust makes no formal guarantee that the
    /// optimiser preserves this; a dedicated crate would.)
    ///
    /// Public for an app that accepts the token (or a second secret of its
    /// own) somewhere [`token_guard`] does not reach: the invoicer's gate over
    /// MBR's pages, which a browser loads by navigation, with a cookie.
    #[must_use]
    pub fn matches(&self, presented: &str) -> bool {
        let (expected, actual) = (self.0.as_bytes(), presented.as_bytes());
        expected.len() == actual.len()
            && expected
                .iter()
                .zip(actual)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

/// `raw` re-serialized when it is a plain https URL: no port, no credentials.
/// The first test an app's "open this in the system browser" allowlist
/// applies, before its own hosts. The one judge of such URLs, for the
/// window's navigation and an app's links alike (not `gui`-only, so a
/// server-only build vets them the same way).
#[must_use]
pub fn plain_https(raw: &str) -> Option<url::Url> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let plain = url.scheme() == "https"
        && url.host_str().is_some()
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    plain.then_some(url)
}

/// `raw` re-serialized when it is a `mailto:` URL with an address, such as
/// `mailto:ap@example.com?subject=Invoice%20108`. Headers (`subject`, `body`,
/// …) are fine; a bare `mailto:` or one with only headers is refused. The
/// sibling of [`plain_https`] for an app's "open this in the mail app" links.
#[must_use]
pub fn plain_mailto(raw: &str) -> Option<url::Url> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let addressed = url.scheme() == "mailto" && !url.path().trim().is_empty();
    addressed.then_some(url)
}

/// The process's token together with where it came from. `from_env` is what
/// SEC-9 keys on: a non-loopback bind is only allowed when the operator chose
/// the token deliberately.
pub struct ProcessToken {
    pub token: AccessToken,
    /// `true` when the value came from the app's token variable
    /// ([`AppIdentity::token_env`]) rather than the CSPRNG.
    pub from_env: bool,
}

/// `$<var>` when set and non-empty, otherwise a fresh random token. `var` is
/// the app's [`AppIdentity::token_env`]: set it when a cross-origin client
/// (vite dev, the Playwright e2e harness) has to know the token before the
/// server starts.
///
/// # Errors
/// [`SecurityError::MalformedEnvToken`] if the environment value is not a
/// well-formed token, or [`SecurityError::Random`] if entropy is unavailable.
pub fn token_from_env_or_random(var: &'static str) -> Result<ProcessToken, SecurityError> {
    match std::env::var(var).ok().filter(|raw| !raw.is_empty()) {
        Some(raw) => Ok(ProcessToken {
            token: AccessToken::parse(&raw)
                .map_err(|_| SecurityError::MalformedEnvToken { var })?,
            from_env: true,
        }),
        None => Ok(ProcessToken {
            token: AccessToken::generate()?,
            from_env: false,
        }),
    }
}

/// The access-control posture a router enforces. Cheaply cloneable (the lists
/// are shared), because both the middleware state and the SPA handler hold one.
#[derive(Clone)]
pub struct Security {
    /// Required on every wire and `/api` route; `None` disables the token guard.
    token: Option<AccessToken>,
    /// Acceptable `Host` values; `None` disables the guard (non-loopback binds).
    hosts: Option<Arc<[String]>>,
    /// Exact origins allowed cross-origin; empty means no CORS layer at all.
    origins: Arc<[HeaderValue]>,
}

impl Security {
    /// No token, no `Host` guard, no CORS.
    ///
    /// This is the shape an app's in-process test router uses (Ledgeline's
    /// `app` and `router_with_state`) so the `tower::oneshot` harnesses can
    /// drive routes directly. **Never use it for a socket-bound server** — see
    /// [`protect`] and [`harden`].
    #[must_use]
    pub fn open() -> Self {
        Self {
            token: None,
            hosts: None,
            origins: Arc::from([]),
        }
    }

    /// The loopback posture: require `token` on every wire and `/api` route, and
    /// reject any `Host` that is not a loopback name on `port`.
    #[must_use]
    pub fn local(token: AccessToken, port: u16) -> Self {
        let hosts = ["127.0.0.1", "localhost", "[::1]"]
            .into_iter()
            .map(|host| format!("{host}:{port}"))
            .collect::<Vec<_>>();
        Self {
            token: Some(token),
            hosts: Some(hosts.into()),
            origins: Arc::from([]),
        }
    }

    /// The posture for IN-PROCESS dispatch — a mobile host's custom-scheme
    /// handler feeding requests to the router with `Router::oneshot`, no socket
    /// at all (`ledgeline-ffi`).
    ///
    /// * **The token is still required.** A scheme handler receives every
    ///   request its web view makes, including one from a foreign page that
    ///   somehow got loaded into it (a link followed, an injected frame): the
    ///   browser enforces CORS on the *response*, not on whether the request is
    ///   delivered, so a blind write would land without it. The host hands the
    ///   token to its own pages out of band ([`token_init_script`]), as the
    ///   desktop WebView gets it.
    /// * **No `Host` guard.** It exists against DNS rebinding, which needs a
    ///   socket to rebind to. There is none, and the authority a custom scheme
    ///   presents is whatever the host chose.
    /// * **No CORS**, as on every default posture.
    #[must_use]
    pub fn in_process(token: AccessToken) -> Self {
        Self::any_host(token)
    }

    /// Require `token` but accept any `Host`. Only for an explicitly requested
    /// non-loopback bind, where the legitimate `Host` values are unknowable —
    /// which also means this posture has NO anti-DNS-rebinding control.
    #[must_use]
    pub fn any_host(token: AccessToken) -> Self {
        Self {
            token: Some(token),
            hosts: None,
            origins: Arc::from([]),
        }
    }

    /// Permit these EXACT origins to read responses cross-origin (`--allow-origin`).
    /// Never a wildcard: each entry is matched literally by the browser.
    ///
    /// # Errors
    /// [`SecurityError::BadOrigin`] if an entry is not a valid header value or
    /// does not look like a scheme-qualified origin.
    pub fn allow_origins<S: AsRef<str>>(self, origins: &[S]) -> Result<Self, SecurityError> {
        let parsed = origins
            .iter()
            .map(|origin| {
                let origin = origin.as_ref().trim_end_matches('/');
                let usable = origin.starts_with("http://") || origin.starts_with("https://");
                HeaderValue::from_str(origin)
                    .ok()
                    .filter(|_| usable)
                    .ok_or_else(|| SecurityError::BadOrigin {
                        origin: origin.to_string(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            origins: parsed.into(),
            ..self
        })
    }

    /// The CORS layer for the allowlisted origins, or `None` (the default) when
    /// there are none — in which case no CORS layer is installed at all and the
    /// server is same-origin only.
    ///
    /// `extra_headers` are request headers beyond `Authorization`,
    /// `Content-Type` and `Accept` that the app's own client sends (Ledgeline's
    /// statement upload names its file in one); an allowlisted origin's
    /// preflight for any other header is refused.
    #[must_use]
    pub fn cors_layer(&self, extra_headers: &[HeaderName]) -> Option<CorsLayer> {
        if self.origins.is_empty() {
            return None;
        }
        Some(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(self.origins.iter().cloned()))
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                ])
                // The token travels in `Authorization`, not a cookie, so the
                // browser never needs to attach credentials — and leaving this
                // false keeps an allowlisted dev origin from riding along on any
                // ambient authority.
                .allow_credentials(false)
                .allow_headers(
                    [header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT]
                        .into_iter()
                        .chain(extra_headers.iter().cloned())
                        .collect::<Vec<_>>(),
                )
                .max_age(PREFLIGHT_MAX_AGE),
        )
    }
}

/// Is `candidate` a document of the app's own SPA: on `origin` exactly
/// (`http://127.0.0.1:<port>`, no trailing slash) and under `spa_base`
/// ([`AppIdentity::spa_base`])?
///
/// The test the desktop window applies to every page its main frame commits
/// to. `candidate` is parsed the way the WebView parses it (WHATWG URL rules,
/// via `url`), so dot segments are resolved first, `%2e%2e` included, and
/// `/app/../note.md` is judged as `/note.md`. A percent-encoded slash stays
/// encoded, so `/app%2f..` is not under `/app/`; the server routes on the
/// same encoded path, so whatever this accepts, the server hands to the SPA.
/// The comparison is case-sensitive, as the server's routes are.
///
/// Refused: any other scheme, host or port (credentials in the URL change the
/// host, not the origin, so they cannot smuggle one in), `about:`, `data:`
/// and `javascript:` URLs (their origins are opaque), `blob:` URLs (even our
/// own: a blob document is not the SPA), and on our own
/// origin any path outside `spa_base`, except the base itself without its
/// trailing slash (`/app`, which the invoicer's router also serves the SPA
/// on).
#[must_use]
pub fn in_spa(origin: &str, spa_base: &str, candidate: &str) -> bool {
    let Ok(url) = url::Url::parse(candidate) else {
        return false;
    };
    // `blob:http://…` has its creator's (tuple) origin, so the scheme is
    // checked too: only an http(s) document is ever the SPA.
    let candidate_origin = url.origin();
    matches!(url.scheme(), "http" | "https")
        && candidate_origin.is_tuple()
        && candidate_origin.ascii_serialization() == origin
        && path_in_spa(spa_base, url.path())
}

/// Is the (already normalized) `path` under `spa_base`? `/` covers every path.
fn path_in_spa(spa_base: &str, path: &str) -> bool {
    spa_base == "/" || path.starts_with(spa_base) || path == spa_base.trim_end_matches('/')
}

/// The initialization script that hands the WebView its access token, as the
/// `window` global the app's SPA reads ([`AppIdentity::token_global`]).
///
/// Shared by the desktop GUI (wry's initialization script, the `gui` module) and
/// a mobile host (a `WKUserScript` at document start — see `ledgeline-ffi`).
///
/// This is how the token reaches the SPA WITHOUT appearing in any response body:
/// the shell is served unauthenticated, so anything that can reach loopback can
/// read it, and it used to carry the token for exactly that reason. wry runs this
/// at document start, before the page's own scripts, outside the page's CSP —
/// but on EVERY document the main frame loads, whatever it is. So the script
/// checks two things before it hands anything over:
///
/// * `location.origin` is ours exactly. The navigation handler already keeps
///   the WebView on our origin; this means a document from anywhere else would
///   still get nothing, should one ever load.
/// * `location.pathname` is under the app's SPA base
///   ([`AppIdentity::spa_base`], the same rule as [`in_spa`]), so a page that
///   shares the origin but is not the SPA (the invoicer's MBR pages) gets
///   nothing either, even if it reached the main frame. An app whose SPA is
///   the whole origin (`/`) gets the origin check alone, which keeps its
///   script, the iOS host's included, exactly as it was.
///
/// The values are serialized as JSON string literals rather than interpolated
/// raw. [`AccessToken::parse`] already restricts the token to a safe character
/// set, and the origin and base are ours, so this is belt and braces — but it
/// costs nothing and does not depend on any of them staying true.
#[must_use]
pub fn token_init_script(app: &AppIdentity, origin: &str, token: &AccessToken) -> String {
    let literal = |value: &str| serde_json::Value::from(value).to_string();
    let path_guard = if app.spa_base == "/" {
        String::new()
    } else {
        format!(
            " && (window.location.pathname === {bare} \
             || window.location.pathname.startsWith({base}))",
            bare = literal(app.spa_base.trim_end_matches('/')),
            base = literal(app.spa_base),
        )
    };
    format!(
        "if (window.location.origin === {origin}{path_guard}) {{ window.{global} = {token}; }}",
        origin = literal(origin),
        global = app.token_global,
        token = literal(token.as_str()),
    )
}

/// Reject any request whose `Host` is not one this server answers to.
///
/// This is the anti-DNS-rebinding control: an attacker page on
/// `attacker.example.com` whose DNS has been re-pointed at `127.0.0.1` still
/// sends `Host: attacker.example.com`, which never matches. Applied to the WHOLE
/// router (the SPA shell included) so a rebound page cannot even fetch the
/// shell.
pub async fn host_guard(
    State(security): State<Security>,
    request: Request,
    next: Next,
) -> Response {
    let Some(allowed) = security.hosts.clone() else {
        return next.run(request).await;
    };
    // HTTP/1.1 carries the authority in `Host`; HTTP/2's `:authority` pseudo
    // header lands in the URI instead.
    let presented = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.as_str().to_owned())
        });
    match presented {
        Some(host) if allowed.iter().any(|ok| ok.eq_ignore_ascii_case(&host)) => {
            next.run(request).await
        }
        _ => (
            StatusCode::FORBIDDEN,
            "Rejected: this server only answers to a loopback Host.\n",
        )
            .into_response(),
    }
}

/// Require `Authorization: Bearer <token>` on every route this wraps.
///
/// Installed with `route_layer`, so it covers exactly the wire and `/api` routes
/// and never the SPA shell or its static assets — which must stay reachable for
/// the browser to bootstrap at all.
pub async fn token_guard(
    State(security): State<Security>,
    request: Request,
    next: Next,
) -> Response {
    let Some(expected) = security.token.clone() else {
        return next.run(request).await;
    };
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token);
    match presented {
        Some(token) if expected.matches(token) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "Missing or invalid access token.\n",
        )
            .into_response(),
    }
}

/// The credential from an `Authorization: Bearer <token>` value (scheme matched
/// case-insensitively, per RFC 7235).
#[must_use]
pub fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

/// `Cache-Control: no-store` — "do not write this anywhere".
///
/// The canonical spelling for every app on the shell. JSON routes go through
/// [`no_store_json`]; anything that needs the header and is not returning
/// `Json` (e.g. the HTML shell in [`spa`](crate::spa)) uses this value
/// directly rather than a fresh copy of the literal.
#[must_use]
pub fn no_store() -> HeaderValue {
    HeaderValue::from_static("no-store")
}

/// A JSON response marked [`no_store`], with no `ETag` — the one helper every
/// Ledgeline `*_api` module uses for reads that are not derived from the
/// journal snapshot (import, rules, aliases, budget, prices, projections,
/// accounts, QB-journal, profiles, benchmarks).
///
/// The `ETag` is a single per-journal generation counter shared by every read
/// route, so there is no third option for these: tagging them would either
/// invalidate the SPA's cached `/transactions` body for a change that affects
/// no transaction, or hand out a stale document under a fresh tag.
pub fn no_store_json<T: serde::Serialize>(body: T) -> Response {
    ([(header::CACHE_CONTROL, no_store())], axum::Json(body)).into_response()
}

/// The Content-Security-Policy for every response that carries no inline script
/// (JSON, static assets, errors).
#[must_use]
pub fn base_csp() -> HeaderValue {
    csp_value(CSP_TAIL)
}

/// The policy for one rendering of the SPA shell: the base policy plus a
/// `'sha256-…'` source for each inline `<script>` the shell actually contains,
/// and the app's `frame-src` when it has one ([`AppIdentity::spa_frame_src`]).
///
/// Computed from the served bytes rather than baked in, so it stays correct
/// across SPA rebuilds (SvelteKit's bootstrap script changes every build) and
/// across the token we inject into it.
#[must_use]
pub fn shell_csp(app: &AppIdentity, html: &str) -> HeaderValue {
    let hashes = inline_script_hashes(html)
        .iter()
        .map(|hash| format!(" '{hash}'"))
        .collect::<String>();
    let frames = app
        .spa_frame_src
        .map(|sources| format!("; frame-src {sources}"))
        .unwrap_or_default();
    csp_value(&format!("{hashes}{CSP_TAIL}{frames}"))
}

/// [`CSP_HEAD`] followed by `rest` (the script hashes, [`CSP_TAIL`], and any
/// per-app directives), or [`FALLBACK_CSP`] if that is no header value.
fn csp_value(rest: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{CSP_HEAD}{rest}"))
        .unwrap_or_else(|_| HeaderValue::from_static(FALLBACK_CSP))
}

/// `sha256-<base64>` CSP source expressions for every inline `<script>` in
/// `html`, in document order (elements with a `src` attribute are external and
/// covered by `'self'` instead).
///
/// A deliberately small scanner: the only HTML this ever sees is our own
/// generated shell, whose script tags have no attributes containing `>`.
fn inline_script_hashes(html: &str) -> Vec<String> {
    let mut hashes = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        let Some(open_len) = rest[open..].find('>') else {
            break;
        };
        let body_start = open + open_len + 1;
        let Some(close) = rest[body_start..].find("</script>") else {
            break;
        };
        let body_end = body_start + close;
        if !rest[open..body_start].contains("src=") {
            hashes.push(csp_sha256(&rest[body_start..body_end]));
        }
        rest = &rest[body_end + "</script>".len()..];
    }
    hashes
}

/// The `sha256-<base64>` source expression for one inline script's exact text.
#[must_use]
pub fn csp_sha256(script: &str) -> String {
    let digest = Sha256::digest(script.as_bytes());
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

/// Is `host` a loopback address (or the `localhost` name)? Anything else — a LAN
/// address, or the `0.0.0.0` / `::` wildcards — publishes the app's data beyond
/// this machine.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// The access-control decisions taken at startup, before the socket is bound.
///
/// Split from [`Security`] itself because the `Host` guard has to name the port
/// we ACTUALLY bound, and `--port 0` (the GUI's default) only reveals that at
/// bind time — so the plan is made early and [`SecurityPlan::build`] finishes it.
pub struct SecurityPlan {
    token: AccessToken,
    /// Whether the bind host resolved to loopback; drives the `Host` guard.
    loopback: bool,
    /// Exact origins from `--allow-origin`, validated by [`build`](Self::build).
    origins: Vec<String>,
}

impl SecurityPlan {
    /// A plan for serving `token` on `host`, opening the API to `origins`.
    #[must_use]
    pub fn new(token: AccessToken, host: &str, origins: Vec<String>) -> Self {
        Self {
            token,
            loopback: is_loopback_host(host),
            origins,
        }
    }

    /// Whether the bind host is loopback. Anything else should be refused
    /// unless the operator chose the token deliberately (SEC-9).
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        self.loopback
    }

    /// Finish the plan now that `port` is known.
    ///
    /// # Errors
    /// [`SecurityError::BadOrigin`] for an unusable `--allow-origin` entry.
    pub fn build(&self, port: u16) -> Result<Security, SecurityError> {
        let base = if self.loopback {
            Security::local(self.token.clone(), port)
        } else {
            Security::any_host(self.token.clone())
        };
        base.allow_origins(&self.origins)
    }
}

/// Put the token guard and the CORS allowlist on every route registered on
/// `router` so far — call it after the last API route and BEFORE the SPA
/// fallback, which must stay reachable without a token.
///
/// Both are `route_layer`s, never `layer`s: they reach exactly the explicit
/// routes, so the fallback (the SPA shell, its assets, deep links) is served
/// same-origin only and never carries a CORS header (SEC-12). The CORS layer
/// wraps the token guard, which is required: a preflight carries no
/// `Authorization` and must be answered rather than refused. The default
/// posture is no CORS layer at all, so a browser refuses to hand any
/// cross-origin page the responses (SEC-1). `cors_headers` are the app's extra
/// request headers ([`Security::cors_layer`]).
pub fn protect<S>(router: Router<S>, security: &Security, cors_headers: &[HeaderName]) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let router = router.route_layer(middleware::from_fn_with_state(
        security.clone(),
        token_guard,
    ));
    match security.cors_layer(cors_headers) {
        Some(cors) => router.route_layer(cors),
        None => router,
    }
}

/// The layers every response passes through, applied to the WHOLE router (the
/// SPA shell included): the `Host` guard (anti-DNS-rebinding), a handler panic
/// as a `500` rather than a dropped connection (SEC-2), `nosniff`,
/// `no-referrer`, and the base CSP for any response without its own (SEC-7) —
/// `if_not_present`, so the SPA shell's policy, which additionally hashes the
/// inline scripts it just rendered, is left alone.
///
/// Call it last, after the fallback and any app-specific layers that belong
/// inside the guard (Ledgeline's compression).
pub fn harden<S>(router: Router<S>, security: Security) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .layer(middleware::from_fn_with_state(security, host_guard))
        .layer(CatchPanicLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            base_csp(),
        ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_mailto_needs_an_address() {
        for allowed in [
            " mailto:ap@example.com ",
            "mailto:ap@example.com?subject=Invoice%20108&body=Hello",
            "MAILTO:ap@example.com",
        ] {
            assert!(plain_mailto(allowed).is_some(), "{allowed}");
        }
        assert_eq!(
            plain_mailto("mailto:ap@example.com?subject=Invoice 108")
                .map(String::from)
                .as_deref(),
            Some("mailto:ap@example.com?subject=Invoice%20108"),
            "answered re-serialized"
        );
        for refused in [
            "mailto:",
            "mailto:?subject=hi",
            "https://example.com/",
            "tel:+15551234567",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(plain_mailto(refused).is_none(), "{refused}");
        }
    }

    #[test]
    fn generated_tokens_are_long_and_unique() {
        let first = AccessToken::generate().expect("entropy available");
        let second = AccessToken::generate().expect("entropy available");
        assert_eq!(first.as_str().len(), TOKEN_BYTES * 2);
        assert!(first.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(
            first.as_str(),
            second.as_str(),
            "two generated tokens must differ"
        );
    }

    #[test]
    fn parse_rejects_short_and_unsafe_tokens() {
        assert!(AccessToken::parse("dev").is_err(), "too short");
        assert!(
            AccessToken::parse(&"x".repeat(MAX_TOKEN_LEN + 1)).is_err(),
            "too long"
        );
        // The token is interpolated into a <script> string literal, so anything
        // that could break out of it must be refused.
        assert!(AccessToken::parse("abcdefghijklmnop\"").is_err());
        assert!(AccessToken::parse("abcdefghij</script>klmno").is_err());
        assert!(AccessToken::parse("dev-token_for.e2e").is_ok());
    }

    #[test]
    fn token_matches_only_itself() {
        let token = AccessToken::parse("dev-token_for.e2e").expect("well-formed");
        assert!(token.matches("dev-token_for.e2e"));
        assert!(!token.matches("dev-token_for.e2f"));
        assert!(!token.matches("dev-token_for.e2"), "prefix must not match");
        assert!(!token.matches(""));
    }

    #[test]
    fn bearer_token_parses_case_insensitively() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer abc"), Some("abc"));
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
    }

    #[test]
    fn allow_origins_rejects_wildcards_and_bare_hosts() {
        assert!(Security::open().allow_origins(&["*"]).is_err());
        assert!(Security::open().allow_origins(&["localhost:5173"]).is_err());
        assert!(
            Security::open()
                .allow_origins(&["http://localhost:5173"])
                .is_ok()
        );
    }

    #[test]
    fn no_allowed_origins_means_no_cors_layer() {
        assert!(
            Security::local(sample_token(), 5000)
                .cors_layer(&[])
                .is_none()
        );
        assert!(
            Security::local(sample_token(), 5000)
                .allow_origins(&["http://localhost:4173"])
                .expect("valid origin")
                .cors_layer(&[])
                .is_some()
        );
    }

    #[test]
    fn loopback_hosts_are_recognised_and_nothing_else_is() {
        for host in [
            "127.0.0.1",
            "localhost",
            "LOCALHOST",
            "::1",
            "[::1]",
            "127.0.0.2",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "0.0.0.0",
            "::",
            "192.168.1.5",
            "example.com",
            "localhost.evil",
        ] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn an_env_token_names_its_variable_when_malformed() {
        let error = SecurityError::MalformedEnvToken {
            var: "LEDGELINE_TOKEN",
        };
        assert!(
            error
                .to_string()
                .starts_with("$LEDGELINE_TOKEN must be 16–128 characters"),
            "{error}"
        );
    }

    /// The token goes to the WebView by initialization script, and only to a
    /// document on our exact origin — the script itself runs on every document
    /// the main frame loads.
    const WHOLE_ORIGIN: AppIdentity = AppIdentity {
        name: "Ledgeline",
        bundle_id: "com.zmre.ledgeline",
        log_prefix: "ledgeline",
        token_env: "LEDGELINE_TOKEN",
        token_global: "__LEDGELINE_TOKEN__",
        embedded_global: "__LEDGELINE_EMBEDDED__",
        spa_base: "/",
        spa_frame_src: None,
    };

    const UNDER_APP: AppIdentity = AppIdentity {
        name: "Ledgeline Invoicer",
        bundle_id: "com.zmre.ledgeline.invoicer",
        log_prefix: "ledgeline-invoicer",
        token_env: "LEDGELINE_INVOICER_TOKEN",
        token_global: "__LEDGELINE_TOKEN__",
        embedded_global: "__LEDGELINE_EMBEDDED__",
        spa_base: "/app/",
        spa_frame_src: Some("'self' https: mailto:"),
    };

    #[test]
    fn the_token_script_is_guarded_by_our_exact_origin() {
        // A SPA that is the whole origin: the origin check alone, unchanged
        // from before the path guard (the iOS host relies on this).
        let script = token_init_script(&WHOLE_ORIGIN, "http://127.0.0.1:5000", &sample_token());
        assert_eq!(
            script,
            "if (window.location.origin === \"http://127.0.0.1:5000\") \
             { window.__LEDGELINE_TOKEN__ = \"token-for-unit-tests\"; }"
        );
    }

    /// SEC-6: a SPA under a base hands the token only to documents under it,
    /// not to every page that shares the origin.
    #[test]
    fn the_token_script_is_guarded_by_the_spa_base_too() {
        let script = token_init_script(&UNDER_APP, "http://127.0.0.1:5000", &sample_token());
        assert_eq!(
            script,
            "if (window.location.origin === \"http://127.0.0.1:5000\" \
             && (window.location.pathname === \"/app\" \
             || window.location.pathname.startsWith(\"/app/\"))) \
             { window.__LEDGELINE_TOKEN__ = \"token-for-unit-tests\"; }"
        );
    }

    /// The script's guard and [`in_spa`] are one rule: evaluate the guard
    /// against what `location` would hold for each URL (`url` parses as WebKit
    /// does) and compare.
    #[test]
    fn the_token_scripts_path_guard_agrees_with_in_spa() {
        let origin = "http://127.0.0.1:5000";
        for candidate in [
            "http://127.0.0.1:5000/app",
            "http://127.0.0.1:5000/app/",
            "http://127.0.0.1:5000/app/invoices/7?x=1#y",
            "http://127.0.0.1:5000/app/../notes/x.md",
            "http://127.0.0.1:5000/app/%2e%2e/notes/x.md",
            "http://127.0.0.1:5000/app%2f../notes/x.md",
            "http://127.0.0.1:5000/apps/",
            "http://127.0.0.1:5000/App/",
            "http://127.0.0.1:5000/customers/contracts/sow.html",
            "http://127.0.0.1:5000/",
        ] {
            let url = url::Url::parse(candidate).expect("test URL parses");
            let pathname = url.path();
            let guard = pathname == "/app" || pathname.starts_with("/app/");
            assert_eq!(
                guard,
                in_spa(origin, UNDER_APP.spa_base, candidate),
                "{candidate}"
            );
        }
    }

    #[test]
    fn in_spa_allows_only_documents_under_the_base_on_our_origin() {
        let origin = "http://127.0.0.1:5000";
        let base = UNDER_APP.spa_base;
        for allowed in [
            "http://127.0.0.1:5000/app",
            "http://127.0.0.1:5000/app/",
            "http://127.0.0.1:5000/app/new",
            "http://127.0.0.1:5000/app/invoices/2026-001?tab=pdf#top",
            "http://127.0.0.1:5000/app/customers/a%20b",
            // Dot segments that stay inside the base.
            "http://127.0.0.1:5000/app/x/../y",
            "http://127.0.0.1:5000/./app/",
            // An encoded slash is not a separator: the server routes this
            // under `/app/` too, to the SPA.
            "http://127.0.0.1:5000/app/..%2fnotes",
            // The host is case-insensitive and `url` lowercases it.
            "HTTP://127.0.0.1:5000/app/",
        ] {
            assert!(in_spa(origin, base, allowed), "{allowed}");
        }
        for refused in [
            // Our origin, but not the SPA: MBR's notes, files, UI and the API.
            "http://127.0.0.1:5000/",
            "http://127.0.0.1:5000",
            "http://127.0.0.1:5000/customers/contracts/sow.html",
            "http://127.0.0.1:5000/notes/acme.md",
            "http://127.0.0.1:5000/.mbr/notes",
            "http://127.0.0.1:5000/.ledgeline-invoicer/preview",
            "http://127.0.0.1:5000/api/books",
            // Prefix and case tricks.
            "http://127.0.0.1:5000/apps",
            "http://127.0.0.1:5000/app.html",
            "http://127.0.0.1:5000/application/",
            "http://127.0.0.1:5000/App/",
            "http://127.0.0.1:5000/APP/",
            "http://127.0.0.1:5000//app/",
            // Dot segments that climb out, plain and encoded.
            "http://127.0.0.1:5000/app/../notes/acme.md",
            "http://127.0.0.1:5000/app/%2e%2e/notes/acme.md",
            "http://127.0.0.1:5000/app/%2E%2E/notes/acme.md",
            "http://127.0.0.1:5000/app/.%2e/notes/acme.md",
            "http://127.0.0.1:5000/app/..\\notes/acme.md",
            "http://127.0.0.1:5000/app%2f../notes/acme.md",
            "http://127.0.0.1:5000/%61pp/",
            // Other ports, hosts and schemes.
            "http://127.0.0.1:5001/app/",
            "http://127.0.0.1:50000/app/",
            "http://127.0.0.1/app/",
            "https://127.0.0.1:5000/app/",
            "http://localhost:5000/app/",
            "http://[::1]:5000/app/",
            "http://127.0.0.1:5000.evil.example/app/",
            "http://127.0.0.1:5000@evil.example/app/",
            "http://evil.example/app/",
            "https://evil.example/app/",
            "file:///app/",
            "ws://127.0.0.1:5000/app/",
            // Opaque origins.
            "about:blank",
            "about:srcdoc",
            "data:text/html,<script>alert(1)</script>",
            "javascript:alert(1)",
            "blob:http://127.0.0.1:5000/0b0e6a4c-0000-4000-8000-000000000000",
            // Not URLs at all.
            "",
            "/app/",
            "not a url",
        ] {
            assert!(!in_spa(origin, base, refused), "{refused}");
        }
    }

    #[test]
    fn a_whole_origin_spa_allows_every_path_on_our_origin_and_nothing_else() {
        let origin = "http://127.0.0.1:5000";
        let base = WHOLE_ORIGIN.spa_base;
        for allowed in [
            "http://127.0.0.1:5000/",
            "http://127.0.0.1:5000",
            "http://127.0.0.1:5000/reports?tab=1",
            "http://127.0.0.1:5000/new",
            "http://127.0.0.1:5000/settings?rule=x",
        ] {
            assert!(in_spa(origin, base, allowed), "{allowed}");
        }
        for refused in [
            "http://127.0.0.1:5001/",
            "http://127.0.0.1:5000@evil.example/",
            "https://evil.example/",
            "about:blank",
            "data:text/html,x",
            "javascript:alert(1)",
            "blob:http://127.0.0.1:5000/0b0e6a4c-0000-4000-8000-000000000000",
        ] {
            assert!(!in_spa(origin, base, refused), "{refused}");
        }
    }

    #[test]
    fn inline_script_hashes_skips_external_scripts() {
        // Known-answer: sha256("") is the empty-string digest, base64-encoded.
        let html = "<head><script>window.x=1</script><script src=\"/a.js\"></script></head>";
        let hashes = inline_script_hashes(html);
        assert_eq!(hashes.len(), 1, "only the inline script is hashed");
        assert_eq!(hashes[0], csp_sha256("window.x=1"));
    }

    #[test]
    fn csp_sha256_matches_the_known_answer_for_the_empty_script() {
        // sha256 of the empty string, base64 — the standard test vector.
        assert_eq!(
            csp_sha256(""),
            "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn shell_csp_carries_every_inline_hash_and_the_exfiltration_guard() {
        let html = "<head><script>a</script></head><body><script>b</script></body>";
        for app in [&WHOLE_ORIGIN, &UNDER_APP] {
            let csp = shell_csp(app, html);
            let csp = csp.to_str().expect("ascii");
            assert!(csp.contains(&format!("'{}'", csp_sha256("a"))));
            assert!(csp.contains(&format!("'{}'", csp_sha256("b"))));
            assert!(csp.contains("connect-src 'self'"));
            assert!(csp.contains("frame-ancestors 'none'"));
            assert!(!csp.contains("unsafe-eval"));
        }
    }

    /// An app without `spa_frame_src` (Ledgeline) gets exactly the shared
    /// policy, byte for byte: no `frame-src`, so `default-src 'self'` rules.
    #[test]
    fn shell_csp_has_no_frame_src_unless_the_app_asks() {
        let html = "<script>a</script>";
        let hash = csp_sha256("a");
        assert_eq!(
            shell_csp(&WHOLE_ORIGIN, html).to_str().expect("ascii"),
            format!(
                "default-src 'self'; script-src 'self' '{hash}'; \
                 style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                 connect-src 'self'; font-src 'self'; object-src 'none'; \
                 base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
            )
        );
        assert_eq!(
            base_csp().to_str().expect("ascii"),
            "default-src 'self'; script-src 'self'; \
             style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
             connect-src 'self'; font-src 'self'; object-src 'none'; \
             base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
        );
    }

    #[test]
    fn shell_csp_appends_the_apps_frame_src() {
        let html = "<script>a</script>";
        let with = shell_csp(&UNDER_APP, html);
        let without = shell_csp(&WHOLE_ORIGIN, html);
        assert_eq!(
            with.to_str().expect("ascii"),
            format!(
                "{}; frame-src 'self' https: mailto:",
                without.to_str().expect("ascii")
            )
        );
        assert!(
            !base_csp().to_str().expect("ascii").contains("frame-src"),
            "only the shell carries the app's frame-src"
        );
    }

    /// A per-app directive that cannot be a header value fails closed.
    #[test]
    fn a_bad_frame_src_falls_back_to_the_tighter_policy() {
        let broken = AppIdentity {
            spa_frame_src: Some("'self'\n"),
            ..UNDER_APP
        };
        assert_eq!(
            shell_csp(&broken, "").to_str().expect("ascii"),
            FALLBACK_CSP
        );
    }

    fn sample_token() -> AccessToken {
        AccessToken::parse("token-for-unit-tests").expect("well-formed")
    }
}
