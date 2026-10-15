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
//!
//! The shell is deliberately the one thing served WITHOUT a token: the browser
//! has nothing to present until it has loaded the page. So it must never CARRY
//! the token either. HTTP cannot tell the user's WebView from another local
//! process's `curl`, and when the shell embedded the token it handed it to
//! anything on the machine that could reach loopback. The token now reaches the
//! SPA out of band: the GUI injects it with wry's initialization script
//! (`gui.rs`), and `--server` prints a launch URL carrying it in the `#token=`
//! fragment, which a browser never sends. See the threat model in
//! [`crate::security`].
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

use crate::security;

/// The built SPA, embedded from a directory INSIDE this crate.
///
/// `build.rs` populates `spa/` — mirroring `web/build` in a workspace checkout,
/// leaving the shipped copy alone in a published crate, and writing a
/// placeholder when there is neither. It is deliberately not
/// `$CARGO_MANIFEST_DIR/../../web/build`: `cargo package` refuses to include
/// files outside the package root, so pointing at the workspace path would make
/// this crate unpublishable and `cargo install ledgeline-server` would serve the
/// placeholder page to every user. See `build.rs` for the three cases.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/spa"]
struct SpaAssets;

/// Marker injected into the served `index.html`. The SPA reads
/// `window.__LEDGELINE_EMBEDDED__` at startup and, when set, targets same-origin
/// relative URLs instead of a stored server URL — so the packaged app needs no
/// setup modal and is immune to a stale/ephemeral port in `localStorage`.
///
/// It carries NO credential, and must not: see the module docs.
const EMBED_MARKER: &str = "<script>window.__LEDGELINE_EMBEDDED__=true</script>";

/// Shown only when the SPA was never built AND `build.rs`'s placeholder is
/// somehow missing too — a belt-and-suspenders fallback, never the normal path.
const MISSING_SPA_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Ledgeline</title></head><body><h1>Ledgeline SPA not built</h1>\
<p>Run <code>bun run build</code> in <code>web/</code>, then rebuild.</p></body></html>";

/// The SPA shell with the embedded-mode marker injected right after `<head>` —
/// falling back to a prefix if the document has no `<head>`.
fn injected_index() -> String {
    let marker = EMBED_MARKER;
    let raw = match SpaAssets::get("index.html") {
        Some(file) => String::from_utf8_lossy(&file.data).into_owned(),
        None => return MISSING_SPA_HTML.to_string(),
    };
    match raw.find("<head>") {
        Some(head) => {
            let at = head + "<head>".len();
            let mut out = String::with_capacity(raw.len() + marker.len());
            out.push_str(&raw[..at]);
            out.push_str(marker);
            out.push_str(&raw[at..]);
            out
        }
        None => format!("{marker}{raw}"),
    }
}

/// The shell response, carrying a Content-Security-Policy computed from the
/// bytes we are about to send: `script-src` gets a `'sha256-…'` for each inline
/// script actually present, so the policy tracks both SvelteKit's per-build
/// bootstrap script and the marker we just injected.
///
/// It also carries `Cache-Control: no-store` (SEC-13). The shell had no cache
/// headers at all, which left the decision to the browser's heuristics for a
/// document whose body then contained the per-process access token. The body no
/// longer carries it, but keeping the header means a regression that put a
/// credential back in the body would not also write it to disk. The value comes
/// from [`security::no_store`] rather than a fresh literal — see its docs.
///
/// This is the shell ONLY. [`asset_response`] keeps the year-long immutable
/// cache on the content-hashed `_app/immutable/` assets, which carry no
/// credential and are the whole reason that cache exists.
fn shell_response() -> Response {
    let html = injected_index();
    let csp = security::shell_csp(&html);
    let mut response = Html(html).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    headers.insert(header::CACHE_CONTROL, security::no_store());
    response
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
        return shell_response();
    }
    // An `/api/...` miss must be a real 404 — serving the shell here would break
    // the native client's engine-presence detection.
    if path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match SpaAssets::get(path) {
        Some(file) => asset_response(path, file.data),
        // Unknown non-asset path → hand SvelteKit's client-side router the shell.
        None => shell_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell is served unauthenticated, so it must never carry the token —
    /// only the token-free embedded marker.
    #[test]
    fn the_shell_carries_the_marker_and_no_token() {
        let html = injected_index();
        assert!(html.contains(EMBED_MARKER));
        assert!(!html.contains("__LEDGELINE_TOKEN__"));
    }

    /// The shell's CSP must cover the exact script bytes it ships, marker and
    /// all — otherwise the browser silently refuses to boot the SPA.
    #[test]
    fn shell_csp_hashes_the_injected_marker_script() {
        let html = injected_index();
        let csp = security::shell_csp(&html);
        let csp = csp.to_str().expect("ascii policy");
        for script in inline_scripts(&html) {
            let hash = security::csp_sha256(script);
            assert!(
                csp.contains(&hash),
                "CSP is missing the hash for an inline script it serves: {script:?}"
            );
        }
        assert!(csp.contains("connect-src 'self'"));
    }

    /// SEC-13. The shell had no `Cache-Control` at all, which left caching to
    /// the browser's heuristics.
    #[test]
    fn the_shell_is_never_stored() {
        assert_eq!(
            shell_response()
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
    }

    /// The other half of SEC-13: hardening the shell must not cost the assets
    /// their cache. They are content-hashed and carry no credential, so the
    /// year-long immutable entry is exactly right for them.
    #[test]
    fn immutable_assets_keep_their_long_cache() {
        let response = asset_response("_app/immutable/chunks/x.js", Cow::Borrowed(b"//"));
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("public, max-age=31536000, immutable")
        );
    }

    /// Every inline `<script>` body in `html`, mirroring what a browser sees.
    fn inline_scripts(html: &str) -> Vec<&str> {
        html.split("<script")
            .skip(1)
            .filter_map(|chunk| chunk.split_once('>'))
            .filter(|(open, _)| !open.contains("src="))
            .filter_map(|(_, rest)| rest.split_once("</script>"))
            .map(|(body, _)| body)
            .collect()
    }
}
