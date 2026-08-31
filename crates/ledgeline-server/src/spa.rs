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
//!   same-origin relative URLs — and, when the server requires one, the access
//!   token the SPA must present on every API call.
//!
//! The shell is deliberately the one thing served WITHOUT a token: the browser
//! has nothing to present until it has loaded the page. That is safe against a
//! hostile *web page* — no CORS layer ever covers this fallback, not even when
//! `--allow-origin` names a dev origin (SEC-12), so no page of any origin may
//! read the shell — but not against another local process. See the threat model
//! in [`crate::security`].
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

use crate::security::{self, AccessToken};

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
const EMBED_MARKER: &str = "<script>window.__LEDGELINE_EMBEDDED__=true</script>";

/// The marker script, extended with the per-process access token when the server
/// requires one. A token in the page body is unreadable cross-origin, so this is
/// how the same-origin SPA gets its credential for free.
///
/// [`AccessToken::parse`] restricts the token to `[A-Za-z0-9._-]`, so it cannot
/// break out of the JavaScript string literal it lands in.
fn embed_script(token: Option<&AccessToken>) -> Cow<'static, str> {
    match token {
        None => Cow::Borrowed(EMBED_MARKER),
        Some(token) => Cow::Owned(format!(
            "<script>window.__LEDGELINE_EMBEDDED__=true;window.__LEDGELINE_TOKEN__=\"{}\"</script>",
            token.as_str()
        )),
    }
}

/// Shown only when the SPA was never built AND `build.rs`'s placeholder is
/// somehow missing too — a belt-and-suspenders fallback, never the normal path.
const MISSING_SPA_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Ledgeline</title></head><body><h1>Ledgeline SPA not built</h1>\
<p>Run <code>bun run build</code> in <code>web/</code>, then rebuild.</p></body></html>";

/// The SPA shell with the embedded-mode marker (and access token, when there is
/// one) injected right after `<head>` — falling back to a prefix if the document
/// has no `<head>`.
fn injected_index(token: Option<&AccessToken>) -> String {
    let marker = embed_script(token);
    let raw = match SpaAssets::get("index.html") {
        Some(file) => String::from_utf8_lossy(&file.data).into_owned(),
        None => return MISSING_SPA_HTML.to_string(),
    };
    match raw.find("<head>") {
        Some(head) => {
            let at = head + "<head>".len();
            let mut out = String::with_capacity(raw.len() + marker.len());
            out.push_str(&raw[..at]);
            out.push_str(&marker);
            out.push_str(&raw[at..]);
            out
        }
        None => format!("{marker}{raw}"),
    }
}

/// The shell response, carrying a Content-Security-Policy computed from the
/// bytes we are about to send: `script-src` gets a `'sha256-…'` for each inline
/// script actually present, so the policy tracks both SvelteKit's per-build
/// bootstrap script and the token we just injected.
///
/// It also carries `Cache-Control: no-store` (SEC-13). The shell had no cache
/// headers at all, which leaves the decision to the browser's heuristics for a
/// document whose body contains the per-process access token: a disk cache
/// entry outlives the process that minted the token, and a token is exactly the
/// sort of thing that must never be written to disk on our say-so. The value
/// comes from [`security::no_store`] rather than a fresh literal — see its docs.
///
/// This is the shell ONLY. [`asset_response`] keeps the year-long immutable
/// cache on the content-hashed `_app/immutable/` assets, which carry no
/// credential and are the whole reason that cache exists.
fn shell_response(token: Option<&AccessToken>) -> Response {
    let html = injected_index(token);
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
pub(crate) async fn fallback(uri: Uri, token: Option<AccessToken>) -> Response {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == "index.html" {
        return shell_response(token.as_ref());
    }
    // An `/api/...` miss must be a real 404 — serving the shell here would break
    // the native client's engine-presence detection.
    if path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match SpaAssets::get(path) {
        Some(file) => asset_response(path, file.data),
        // Unknown non-asset path → hand SvelteKit's client-side router the shell.
        None => shell_response(token.as_ref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_token() -> AccessToken {
        AccessToken::parse("token-for-unit-tests").expect("well-formed")
    }

    #[test]
    fn embed_script_omits_the_token_when_there_is_none() {
        assert_eq!(embed_script(None), EMBED_MARKER);
    }

    #[test]
    fn embed_script_publishes_the_token_to_the_page() {
        let script = embed_script(Some(&sample_token()));
        assert!(script.contains("window.__LEDGELINE_EMBEDDED__=true"));
        assert!(script.contains("window.__LEDGELINE_TOKEN__=\"token-for-unit-tests\""));
    }

    /// The shell's CSP must cover the exact script bytes it ships, token and
    /// all — otherwise the browser silently refuses to boot the SPA.
    #[test]
    fn shell_csp_hashes_the_injected_token_script() {
        let html = injected_index(Some(&sample_token()));
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

    /// SEC-13. The shell's body carries the access token, so it may not be
    /// written to any cache — and it had no `Cache-Control` at all, which left
    /// that to the browser's heuristics.
    #[test]
    fn the_token_bearing_shell_is_never_stored() {
        for token in [None, Some(sample_token())] {
            let response = shell_response(token.as_ref());
            assert_eq!(
                response
                    .headers()
                    .get(header::CACHE_CONTROL)
                    .and_then(|value| value.to_str().ok()),
                Some("no-store"),
                "the shell must forbid caching whether or not it carries a token"
            );
        }
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
