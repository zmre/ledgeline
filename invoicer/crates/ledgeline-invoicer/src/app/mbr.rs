//! MBR's router at the server's root (ARCHITECTURE §6.1), behind a gate.
//!
//! **Why a gate.** Ledgeline's posture is that nothing about the books is
//! served without the token: HTTP can't tell the user's webview from another
//! local process. MBR's pages are the books' notes (invoices, customers'
//! contact details), so they need the same protection, but a browser loads
//! them by navigation (an iframe, a link), which can't carry an
//! `Authorization` header. So a page is served when the request carries
//! either:
//!
//! - the bearer token (scripts, `curl`); or
//! - the session cookie, a second per-process secret the SPA obtains with
//!   `POST /api/session` (token-gated): `HttpOnly`, `SameSite=Strict`,
//!   `Path=/`, and named after the port, since a cookie is shared by every
//!   port on a host.
//!
//! **CSP.** MBR's pages need inline scripts (its page config) and may show
//! remote images, so they get their own policy ([`MBR_CSP`]) instead of the
//! shell's script-hash one; `connect-src 'self'` still stops a script from
//! sending anything elsewhere, and `frame-ancestors 'self'` lets the SPA show
//! a page in a frame (browse and preview modes).

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use ledgeline_shell::security::{self, AccessToken, SecurityError};
use tower::ServiceExt as _;

use super::AppState;

/// The policy on MBR's pages. See the module docs.
pub(crate) const MBR_CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; \
style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; media-src 'self' https:; \
font-src 'self' data:; connect-src 'self'; frame-src 'self' https:; object-src 'none'; \
base-uri 'self'; form-action 'self'; frame-ancestors 'self'";

/// Who may see MBR's pages: the bearer token, or the session cookie.
pub(crate) struct MbrGate {
    /// `None`: nobody (the session secret couldn't be made).
    keys: Option<(AccessToken, AccessToken)>,
    cookie_name: String,
}

impl MbrGate {
    /// A gate for the server on `port`, with a fresh session secret.
    ///
    /// # Errors
    /// [`SecurityError::Random`] when the OS has no entropy.
    pub(crate) fn new(token: AccessToken, port: u16) -> Result<Self, SecurityError> {
        Ok(Self {
            keys: Some((token, AccessToken::generate()?)),
            cookie_name: cookie_name(port),
        })
    }

    /// A gate that lets nothing through.
    pub(crate) fn closed() -> Self {
        Self {
            keys: None,
            cookie_name: cookie_name(0),
        }
    }

    /// Whether `headers` carry the token or the session cookie.
    pub(crate) fn allows(&self, headers: &HeaderMap) -> bool {
        let Some((token, secret)) = &self.keys else {
            return false;
        };
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(security::bearer_token)
            .is_some_and(|t| token.matches(t));
        bearer
            || headers
                .get_all(header::COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(';'))
                .filter_map(|pair| pair.trim().split_once('='))
                .any(|(name, value)| name == self.cookie_name && secret.matches(value))
    }

    /// `POST /api/session`'s answer: the cookie, or `503` with a closed gate.
    pub(crate) fn session_response(&self) -> Response {
        let Some((_, secret)) = &self.keys else {
            return (StatusCode::SERVICE_UNAVAILABLE, "MBR pages are off\n").into_response();
        };
        let cookie = format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict",
            self.cookie_name,
            secret.as_str()
        );
        match HeaderValue::from_str(&cookie) {
            Ok(value) => (
                StatusCode::NO_CONTENT,
                [
                    (header::SET_COOKIE, value),
                    (header::CACHE_CONTROL, security::no_store()),
                ],
            )
                .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

/// The session cookie's name for the server on `port`.
fn cookie_name(port: u16) -> String {
    format!("ledgeline_invoicer_{port}")
}

/// The router's fallback: everything not `/api` or `/app` goes to the open
/// books' MBR router, through the gate.
pub(crate) async fn fallback(
    State(state): State<AppState>,
    Extension(gate): Extension<Arc<MbrGate>>,
    mut request: Request,
) -> Response {
    if !gate.allows(request.headers()) {
        return refused();
    }
    let Some(books) = state.books() else {
        return no_books();
    };
    // MBR has no use for the credential.
    request.headers_mut().remove(header::AUTHORIZATION);
    let mut response = match books.mbr().clone().oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    response
        .headers_mut()
        .entry(header::CONTENT_SECURITY_POLICY)
        .or_insert(HeaderValue::from_static(MBR_CSP));
    response
}

/// A page asked for without the token or the session cookie.
fn refused() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer")),
            (header::CACHE_CONTROL, security::no_store()),
        ],
        Html(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Ledgeline Invoicer</title>\
             </head><body><p>This page belongs to Ledgeline Invoicer. Open it from the app.</p>\
             </body></html>",
        ),
    )
        .into_response()
}

/// A page asked for while no books are open.
fn no_books() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CACHE_CONTROL, security::no_store())],
        Html(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Ledgeline Invoicer</title>\
             </head><body><p>No books are open. Choose File → Open….</p></body></html>",
        ),
    )
        .into_response()
}
