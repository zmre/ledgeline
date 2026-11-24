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
//! **The PDF export's webview** (PLAN 3c) can carry neither: it is a fresh
//! hidden view loading one URL. It loads a one-time **export link**
//! (`/.ledgeline-invoicer/export/<ticket>`), minted by the token-gated PDF
//! route for one page: redeemed once, within [`TICKET_TTL`], it sets the
//! session cookie and redirects to that page, so the page's own requests
//! (styles, images, the export-ready partial) pass the gate as the SPA's
//! frames do. The ticket names the page server-side, so the link redirects
//! nowhere else.
//!
//! **CSP.** MBR's pages need inline scripts (its page config) and may show
//! remote images, so they get their own policy ([`MBR_CSP`]) instead of the
//! shell's script-hash one; `connect-src 'self'` still stops a script from
//! sending anything elsewhere, and `frame-ancestors 'self'` lets the SPA show
//! a page in a frame (browse and preview modes).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Extension;
use axum::extract::{Path, Request, State};
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

/// Where export links live: a hidden folder, which MBR never serves.
pub(crate) const EXPORT_LINK_PREFIX: &str = "/.ledgeline-invoicer/export/";

/// How long an export link stays good: long enough for a webview to start.
pub(crate) const TICKET_TTL: Duration = Duration::from_secs(60);

/// At most this many unredeemed links are held; the oldest go first.
const MAX_TICKETS: usize = 64;

/// Who may see MBR's pages: the bearer token, or the session cookie.
pub(crate) struct MbrGate {
    /// `None`: nobody (the session secret couldn't be made).
    keys: Option<(AccessToken, AccessToken)>,
    cookie_name: String,
    /// Unredeemed export links: ticket → (page, minted).
    tickets: Mutex<HashMap<String, (String, Instant)>>,
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
            tickets: Mutex::new(HashMap::new()),
        })
    }

    /// A gate that lets nothing through.
    pub(crate) fn closed() -> Self {
        Self {
            keys: None,
            cookie_name: cookie_name(0),
            tickets: Mutex::new(HashMap::new()),
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

    /// A one-time export link's path for `page` (a path on this server,
    /// `/invoices/A%20B/`). `None` with a closed gate, or no entropy.
    pub(crate) fn export_link(&self, page: &str) -> Option<String> {
        self.keys.as_ref()?;
        let ticket = AccessToken::generate().ok()?.as_str().to_string();
        let mut tickets = self.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        tickets.retain(|_, (_, minted)| now.duration_since(*minted) < TICKET_TTL);
        while tickets.len() >= MAX_TICKETS {
            let Some(oldest) = tickets
                .iter()
                .min_by_key(|(_, (_, minted))| *minted)
                .map(|(t, _)| t.clone())
            else {
                break;
            };
            tickets.remove(&oldest);
        }
        tickets.insert(ticket.clone(), (page.to_string(), now));
        Some(format!("{EXPORT_LINK_PREFIX}{ticket}"))
    }

    /// The page an export link was minted for, once: the ticket is spent
    /// whether or not it was still good.
    fn redeem(&self, ticket: &str) -> Option<String> {
        let mut tickets = self.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        let (page, minted) = tickets.remove(ticket)?;
        (minted.elapsed() < TICKET_TTL).then_some(page)
    }

    /// The session cookie's `Set-Cookie` value.
    fn cookie(&self) -> Option<HeaderValue> {
        let (_, secret) = self.keys.as_ref()?;
        HeaderValue::from_str(&format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict",
            self.cookie_name,
            secret.as_str()
        ))
        .ok()
    }

    /// `POST /api/session`'s answer: the cookie, or `503` with a closed gate.
    pub(crate) fn session_response(&self) -> Response {
        if self.keys.is_none() {
            return (StatusCode::SERVICE_UNAVAILABLE, "MBR pages are off\n").into_response();
        }
        match self.cookie() {
            Some(value) => (
                StatusCode::NO_CONTENT,
                [
                    (header::SET_COOKIE, value),
                    (header::CACHE_CONTROL, security::no_store()),
                ],
            )
                .into_response(),
            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

/// `GET /.ledgeline-invoicer/export/{ticket}`: an export link. Good once:
/// the session cookie and a redirect to its page; otherwise `401`.
pub(crate) async fn export_link(
    Extension(gate): Extension<Arc<MbrGate>>,
    Path(ticket): Path<String>,
) -> Response {
    let (Some(page), Some(cookie)) = (gate.redeem(&ticket), gate.cookie()) else {
        return refused();
    };
    let Ok(location) = HeaderValue::from_str(&page) else {
        return refused();
    };
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie),
            (header::LOCATION, location),
            (header::CACHE_CONTROL, security::no_store()),
        ],
    )
        .into_response()
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
pub(crate) fn refused() -> Response {
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
pub(crate) fn no_books() -> Response {
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
