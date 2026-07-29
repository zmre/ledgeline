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
//! * **A process running as a DIFFERENT local user — the token does not stop
//!   this on its own.** Such a process can open a TCP connection to loopback,
//!   `GET /`, and read the token straight out of the SPA shell we serve it: the
//!   shell has to carry the token for the browser to bootstrap, and HTTP gives
//!   us no way to tell the user's own WebView from someone else's `curl`. What
//!   actually limits that attacker is the loopback-only bind (see
//!   `main::plan_security`) and the journal file's own permissions. Closing
//!   it properly means handing the token to the WebView out-of-band (wry's
//!   initialization script) so it never appears in a response body.
//! * **Anything at all once `--host` is pointed off loopback.** That path is
//!   gated on an explicitly configured `LEDGELINE_TOKEN` and turns the `Host`
//!   guard off, because the legitimate `Host` values are then unknowable.
//!
//! The token is per-process and never persisted: restarting the app invalidates
//! it, and no other process is told what it is.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Environment variable holding a caller-chosen token. Set it when a
/// cross-origin client (vite dev, the Playwright e2e harness) has to know the
/// token before the server starts; leave it unset and one is generated.
pub const TOKEN_ENV: &str = "LEDGELINE_TOKEN";

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
/// `style="display: contents"` attribute.
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
        "${TOKEN_ENV} must be {MIN_TOKEN_LEN}–{MAX_TOKEN_LEN} characters of [A-Za-z0-9._-] (it is embedded in a page script, so the character set is restricted)"
    )]
    MalformedToken,
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
    /// interpolated into a `<script>` string literal in the served shell, and a
    /// quote or `</script>` there would be an injection.
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
    fn matches(&self, presented: &str) -> bool {
        let (expected, actual) = (self.0.as_bytes(), presented.as_bytes());
        expected.len() == actual.len()
            && expected
                .iter()
                .zip(actual)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

/// The process's token together with where it came from. `from_env` is what
/// SEC-9 keys on: a non-loopback bind is only allowed when the operator chose
/// the token deliberately.
pub struct ProcessToken {
    pub token: AccessToken,
    /// `true` when the value came from `$LEDGELINE_TOKEN` rather than the CSPRNG.
    pub from_env: bool,
}

/// `$LEDGELINE_TOKEN` when set and non-empty, otherwise a fresh random token.
///
/// # Errors
/// [`SecurityError::MalformedToken`] if the environment value is not a
/// well-formed token, or [`SecurityError::Random`] if entropy is unavailable.
pub fn token_from_env_or_random() -> Result<ProcessToken, SecurityError> {
    match std::env::var(TOKEN_ENV).ok().filter(|raw| !raw.is_empty()) {
        Some(raw) => Ok(ProcessToken {
            token: AccessToken::parse(&raw)?,
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
    /// This is the shape [`crate::app`] and [`crate::router_with_state`] use so
    /// the `tower::oneshot` test harnesses can drive routes directly. **Never
    /// use it for a socket-bound server** — see [`crate::router_with_security`].
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

    /// The token the SPA shell must advertise, if any.
    pub(crate) fn token(&self) -> Option<AccessToken> {
        self.token.clone()
    }

    /// The CORS layer for the allowlisted origins, or `None` (the default) when
    /// there are none — in which case no CORS layer is installed at all and the
    /// server is same-origin only.
    pub(crate) fn cors_layer(&self) -> Option<CorsLayer> {
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
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
                .max_age(PREFLIGHT_MAX_AGE),
        )
    }
}

/// Reject any request whose `Host` is not one this server answers to.
///
/// This is the anti-DNS-rebinding control: an attacker page on
/// `attacker.example.com` whose DNS has been re-pointed at `127.0.0.1` still
/// sends `Host: attacker.example.com`, which never matches. Applied to the WHOLE
/// router (the SPA shell included) so a rebound page cannot even fetch the page
/// that carries the token.
pub(crate) async fn host_guard(
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
pub(crate) async fn token_guard(
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
fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

/// The Content-Security-Policy for every response that carries no inline script
/// (JSON, static assets, errors).
pub(crate) fn base_csp() -> HeaderValue {
    csp_value("")
}

/// The policy for one rendering of the SPA shell: the base policy plus a
/// `'sha256-…'` source for each inline `<script>` the shell actually contains.
///
/// Computed from the served bytes rather than baked in, so it stays correct
/// across SPA rebuilds (SvelteKit's bootstrap script changes every build) and
/// across the token we inject into it.
pub(crate) fn shell_csp(html: &str) -> HeaderValue {
    let hashes = inline_script_hashes(html)
        .iter()
        .map(|hash| format!(" '{hash}'"))
        .collect::<String>();
    csp_value(&hashes)
}

fn csp_value(script_sources: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{CSP_HEAD}{script_sources}{CSP_TAIL}"))
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
pub(crate) fn csp_sha256(script: &str) -> String {
    let digest = Sha256::digest(script.as_bytes());
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(Security::local(sample_token(), 5000).cors_layer().is_none());
        assert!(
            Security::local(sample_token(), 5000)
                .allow_origins(&["http://localhost:4173"])
                .expect("valid origin")
                .cors_layer()
                .is_some()
        );
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
        let csp = shell_csp(html);
        let csp = csp.to_str().expect("ascii");
        assert!(csp.contains(&format!("'{}'", csp_sha256("a"))));
        assert!(csp.contains(&format!("'{}'", csp_sha256("b"))));
        assert!(csp.contains("connect-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains("unsafe-eval"));
    }

    fn sample_token() -> AccessToken {
        AccessToken::parse("token-for-unit-tests").expect("well-formed")
    }
}
