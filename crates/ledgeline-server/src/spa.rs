//! Serve the built SvelteKit SPA same-origin, in-process.
//!
//! The SPA is embedded from `web/build` via `rust-embed` (release builds bake
//! the bytes into the binary; debug builds read them from disk for fast SPA
//! iteration). It is served from the same axum app — and therefore the same
//! origin — as the wire (`/version`, `/transactions`, …) and `/api/*` routes,
//! which is what lets the packaged GUI skip the cross-origin setup modal.
//!
//! Routing contract (installed as the router's `fallback`, so the explicit wire
//! and `/api/*` routes always win):
//! - `/` and `/index.html` → the SPA shell (`index.html`), with a small marker
//!   script injected so the SPA knows it is running embedded and should use
//!   same-origin relative URLs.
//! - a real embedded asset path (e.g. `/_app/immutable/...`, `/robots.txt`) →
//!   that file, with a guessed `Content-Type` (and a long immutable cache for
//!   the content-hashed `_app/immutable/` assets).
//! - any other non-`/api/` path → the SPA shell too, so SvelteKit's client-side
//!   router can handle deep links (`/holdings`, `/reports`, …).
//! - a `/api/...` miss → `404` (never the shell), so the native client's
//!   "is this the engine?" probe still works.

use std::borrow::Cow;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

/// The built SPA. Resolved relative to this crate so the workspace layout
/// (`crates/ledgeline-server` → `web/build`) is explicit; `build.rs` guarantees
/// the folder exists even on a fresh checkout.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/build"]
struct SpaAssets;

/// Marker injected into the served `index.html`. The SPA reads
/// `window.__LEDGELINE_EMBEDDED__` at startup and, when set, targets same-origin
/// relative URLs instead of a stored server URL — so the packaged app needs no
/// setup modal and is immune to a stale/ephemeral port in `localStorage`.
const EMBED_MARKER: &str = "<script>window.__LEDGELINE_EMBEDDED__=true</script>";

/// Shown only when the SPA was never built AND `build.rs`'s placeholder is
/// somehow missing too — a belt-and-suspenders fallback, never the normal path.
const MISSING_SPA_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Ledgeline</title></head><body><h1>Ledgeline SPA not built</h1>\
<p>Run <code>bun run build</code> in <code>web/</code>, then rebuild.</p></body></html>";

/// The SPA shell with the embedded-mode marker injected right after `<head>`
/// (falling back to a prefix if the document has no `<head>`).
fn injected_index() -> String {
    let raw = match SpaAssets::get("index.html") {
        Some(file) => String::from_utf8_lossy(&file.data).into_owned(),
        None => return MISSING_SPA_HTML.to_string(),
    };
    match raw.find("<head>") {
        Some(head) => {
            let at = head + "<head>".len();
            let mut out = String::with_capacity(raw.len() + EMBED_MARKER.len());
            out.push_str(&raw[..at]);
            out.push_str(EMBED_MARKER);
            out.push_str(&raw[at..]);
            out
        }
        None => format!("{EMBED_MARKER}{raw}"),
    }
}

/// Serve an embedded asset with a guessed content type, caching the
/// content-hashed `_app/immutable/` assets aggressively.
fn asset_response(path: &str, data: Cow<'static, [u8]>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Body::from(data.into_owned()).into_response();
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    if path.starts_with("_app/immutable/") {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    response
}

/// Router `fallback`: serve the SPA (shell + assets) for everything the explicit
/// wire / `/api/*` routes did not match. See the module docs for the contract.
pub(crate) async fn fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == "index.html" {
        return Html(injected_index()).into_response();
    }
    // An `/api/...` miss must be a real 404 — serving the shell here would break
    // the native client's engine-presence detection.
    if path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match SpaAssets::get(path) {
        Some(file) => asset_response(path, file.data),
        // Unknown non-asset path → hand SvelteKit's client-side router the shell.
        None => Html(injected_index()).into_response(),
    }
}
