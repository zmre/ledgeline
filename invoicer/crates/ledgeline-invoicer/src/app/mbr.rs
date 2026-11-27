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
//! **CSP** (security review SEC-1, SEC-3, SEC-6; MBR 0.7.5's M17 hooks).
//! MBR's pages carry no inline script since 0.7.5 (their page data is JSON
//! that `/.mbr/mbr-bootstrap.js` reads), so every policy here is
//! [`mbr_core::csp::STRICT_SCRIPT_SRC`]: scripts from this server only, never
//! inline, never a handler attribute. Nothing loads from another host
//! (`img-src 'self' data:`, no remote media or frames), so previewing or
//! exporting an invoice sends none of it anywhere. MBR sends the policy per
//! route class ([`content_security_policy`], set in the books' MBR
//! configuration):
//!
//! - rendered notes and the books' own files get [`SANDBOXED_CSP`]: a CSP
//!   sandbox with no `allow-scripts`, so nothing in them runs, even opened
//!   in a tab of its own (a contract `.html` in `--server` mode). The SPA's
//!   frames are script-less already; this holds without them.
//! - MBR's own UI under `/.mbr/` (its scripts, styles, the components the
//!   SPA loads) gets [`STRICT_CSP`], the same policy without the sandbox.
//!
//! The one page that must run scripts is the one a PDF export prints (MBR's
//! export-ready signal). Its export link redirects with a one-time **export
//! pass** in the query ([`EXPORT_PASS_PARAM`]); the page request that spends
//! it gets [`STRICT_CSP`] instead of the sandbox. The export webview may
//! navigate only within this server's origin (MBR 0.7.5's M16), so a script
//! there can read what the gate lets it read but send it nowhere.
//! `frame-ancestors 'self'` lets the SPA show a page in a frame (browse and
//! preview modes).
//!
//! **Drafts.** A draft follows the company note ([`invoicer_core::company`]),
//! which MBR, rendering the file as written, can't do. So a page request for
//! a draft whose file holds an older copy of the company is answered here
//! with [`super::preview::draft_page`] under the same policies; every other
//! page is MBR's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Extension;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use ledgeline_shell::security::{self, AccessToken, SecurityError};
use mbr_core::csp::{ContentClass, ContentSecurityPolicy};
use tower::ServiceExt as _;

use super::AppState;

/// The directives every policy here shares. See the module docs.
macro_rules! strict_directives {
    () => {
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; media-src 'self'; font-src 'self' data:; connect-src 'self'; \
frame-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; \
frame-ancestors 'self'"
    };
}

/// The policy on what may run scripts from this server: MBR's own UI
/// (`/.mbr/`), and the page a PDF export prints. See the module docs.
pub(crate) const STRICT_CSP: &str = strict_directives!();

/// The policy on the books' content: rendered notes, the books' own files,
/// the invoice preview. [`STRICT_CSP`] in a CSP sandbox
/// ([`mbr_core::csp::SANDBOX`]): no script runs at all.
pub(crate) const SANDBOXED_CSP: &str = concat!("sandbox allow-same-origin; ", strict_directives!());

/// MBR's `Content-Security-Policy` per route class, for the books' MBR
/// configuration (which a books folder's own `.mbr/config.toml` can't
/// change: the invoicer sets it after reading that).
pub(crate) fn content_security_policy() -> ContentSecurityPolicy {
    ContentSecurityPolicy {
        pages: Some(SANDBOXED_CSP.to_string()),
        user_files: Some(SANDBOXED_CSP.to_string()),
        mbr_assets: Some(STRICT_CSP.to_string()),
    }
}

/// The query parameter an export link's redirect carries the export pass in.
pub(crate) const EXPORT_PASS_PARAM: &str = "ledgeline-invoicer-export";

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
    /// Unspent export passes (see the module docs): pass → ((), minted).
    passes: Mutex<HashMap<String, ((), Instant)>>,
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
            passes: Mutex::new(HashMap::new()),
        })
    }

    /// A gate that lets nothing through.
    pub(crate) fn closed() -> Self {
        Self {
            keys: None,
            cookie_name: cookie_name(0),
            tickets: Mutex::new(HashMap::new()),
            passes: Mutex::new(HashMap::new()),
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
        hold(&self.tickets, ticket.clone(), page.to_string());
        Some(format!("{EXPORT_LINK_PREFIX}{ticket}"))
    }

    /// The page an export link was minted for, once: the ticket is spent
    /// whether or not it was still good.
    fn redeem(&self, ticket: &str) -> Option<String> {
        spend(&self.tickets, ticket)
    }

    /// A fresh export pass, good once within [`TICKET_TTL`]. `None` with no
    /// entropy.
    fn mint_pass(&self) -> Option<String> {
        let pass = AccessToken::generate().ok()?.as_str().to_string();
        hold(&self.passes, pass.clone(), ());
        Some(pass)
    }

    /// Whether `pass` was a good export pass; spent either way.
    fn spend_pass(&self, pass: &str) -> bool {
        spend(&self.passes, pass).is_some()
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

/// Keeps `value` under `key` in `held` for [`TICKET_TTL`], dropping what
/// expired, and the oldest beyond [`MAX_TICKETS`].
fn hold<V>(held: &Mutex<HashMap<String, (V, Instant)>>, key: String, value: V) {
    let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    held.retain(|_, (_, minted)| now.duration_since(*minted) < TICKET_TTL);
    while held.len() >= MAX_TICKETS {
        let Some(oldest) = held
            .iter()
            .min_by_key(|(_, (_, minted))| *minted)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        held.remove(&oldest);
    }
    held.insert(key, (value, now));
}

/// What [`hold`] kept under `key`, once, if it is still good.
fn spend<V>(held: &Mutex<HashMap<String, (V, Instant)>>, key: &str) -> Option<V> {
    let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
    let (value, minted) = held.remove(key)?;
    (minted.elapsed() < TICKET_TTL).then_some(value)
}

/// `GET /.ledgeline-invoicer/export/{ticket}`: an export link. Good once:
/// the session cookie and a redirect to its page, with an export pass;
/// otherwise `401`.
pub(crate) async fn export_link(
    Extension(gate): Extension<Arc<MbrGate>>,
    Path(ticket): Path<String>,
) -> Response {
    let (Some(page), Some(cookie)) = (gate.redeem(&ticket), gate.cookie()) else {
        return refused();
    };
    let Some(pass) = gate.mint_pass() else {
        return refused();
    };
    let separator = if page.contains('?') { '&' } else { '?' };
    let Ok(location) =
        HeaderValue::from_str(&format!("{page}{separator}{EXPORT_PASS_PARAM}={pass}"))
    else {
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
    let path_class = ContentClass::for_path(request.uri().path());
    let pass = export_pass(request.uri().query()).map(str::to_string);
    // A draft follows the company, which MBR can't do from the file: its
    // page is rendered here when the file holds an older copy
    // ([`super::preview::draft_page`]).
    if path_class == ContentClass::Pages && matches!(*request.method(), Method::GET | Method::HEAD)
    {
        let page = Arc::clone(&books);
        let path = request.uri().path().to_string();
        if let Ok(Some(html)) =
            tokio::task::spawn_blocking(move || super::preview::draft_page(&page, &path)).await
        {
            let strict = pass.is_some_and(|pass| gate.spend_pass(&pass));
            return draft_response(html, strict);
        }
    }
    let mut response = match books.mbr().clone().oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    let class = response
        .extensions()
        .get::<ContentClass>()
        .copied()
        .unwrap_or(path_class);
    let page_ok = response.status() == StatusCode::OK;
    let headers = response.headers();
    let strict = match class {
        // The page a PDF export prints: its export pass is spent here, on
        // the page itself (not on a redirect MBR answers first, which keeps
        // the query).
        ContentClass::Pages => {
            is_html(headers) && page_ok && pass.is_some_and(|pass| gate.spend_pass(&pass))
        }
        // A PDF is shown by the browser's viewer, which a CSP sandbox stops;
        // it has no script that reaches this origin's pages.
        ContentClass::UserFiles => is_pdf(headers),
        ContentClass::MbrAssets => false,
    };
    let headers = response.headers_mut();
    if strict {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(STRICT_CSP),
        );
    } else {
        headers
            .entry(header::CONTENT_SECURITY_POLICY)
            .or_insert(HeaderValue::from_static(SANDBOXED_CSP));
    }
    response
}

/// A draft's page rendered here ([`fallback`]), with the policy MBR's page
/// would have had: [`STRICT_CSP`] for the page a PDF export prints (its
/// export pass spent), else [`SANDBOXED_CSP`].
fn draft_response(html: String, strict: bool) -> Response {
    let policy = if strict { STRICT_CSP } else { SANDBOXED_CSP };
    (
        [
            (header::CACHE_CONTROL, security::no_store()),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(policy),
            ),
        ],
        Html(html),
    )
        .into_response()
}

/// The export pass in a request's query, if it carries one.
fn export_pass(query: Option<&str>) -> Option<&str> {
    query?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == EXPORT_PASS_PARAM)
        .map(|(_, value)| value)
}

/// Whether a response's `Content-Type` is `essence`.
fn content_type_is(headers: &HeaderMap, essence: &str) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case(essence))
}

fn is_html(headers: &HeaderMap) -> bool {
    content_type_is(headers, "text/html")
}

fn is_pdf(headers: &HeaderMap) -> bool {
    content_type_is(headers, "application/pdf")
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
