//! The SPA shell's Content-Security-Policy, checked against the shell that is
//! ACTUALLY embedded in this build.
//!
//! The policy is computed at request time from the served bytes
//! (`security::shell_csp`), and every other CSP test feeds it hand-written HTML
//! or `build.rs`'s placeholder page, which carries no script at all. So until
//! this file, nothing ever checked the policy against a real SvelteKit shell:
//! an upgrade that emitted an import map, an inline event handler, or a script
//! tag the production scanner mis-parses would break the app to a blank window
//! with every test green.
//!
//! Two deliberate choices:
//!
//! * **An independent parser.** The checks re-derive the script inventory
//!   with a small attribute-aware tag scanner of their own rather than calling
//!   the production one, and hash with `sha2` here rather than the shell's
//!   `csp_sha256`. The scanner and the assertions are in
//!   `crates/ledgeline-shell/tests/support/shell_csp_oracle.rs`, shared with
//!   the invoicer's twin of this test.
//! * **`LEDGELINE_REQUIRE_REAL_SPA=1` makes a placeholder a FAILURE.** A plain
//!   `cargo test` on a fresh checkout embeds the placeholder, and the invariants
//!   still hold for it, so the test runs and passes there. The
//!   `spaShellCsp` Nix check embeds the real `spaBuild` output and sets the
//!   variable, so a silently-placeholder build cannot pass for a real one.

mod common;
#[path = "../../ledgeline-shell/tests/support/shell_csp_oracle.rs"]
mod oracle;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use common::fixture_journal;
use http_body_util::BodyExt;
use ledgeline::{AccessToken, AppState, Security, router_with_security};
use oracle::{assert_shell_csp, directive, scan};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const PORT: u16 = 5099;
const REQUIRE_REAL_SPA: &str = "LEDGELINE_REQUIRE_REAL_SPA";

/// `GET /` through the full secured router: `(Content-Security-Policy, body)`.
async fn served_shell() -> (String, String) {
    let token = AccessToken::parse("spa-shell-csp-test-token").expect("well-formed token");
    let router = router_with_security(
        AppState::from_journal(&fixture_journal()),
        Security::local(token, PORT),
    );
    let request = Request::builder()
        .method("GET")
        .uri("/")
        .header(header::HOST, format!("127.0.0.1:{PORT}"))
        .body(Body::empty())
        .expect("request builds");
    let response = router.oneshot(request).await.expect("router responds");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the shell must be served"
    );
    let csp = response
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("the shell carries a CSP");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let html = String::from_utf8(bytes.to_vec()).expect("the shell is UTF-8");
    (csp, html)
}

fn sha256_source(script: &str) -> String {
    let digest = Sha256::digest(script.as_bytes());
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

#[tokio::test]
async fn the_shell_csp_admits_exactly_the_inline_scripts_it_serves() {
    let (csp, html) = served_shell().await;
    assert_shell_csp(
        &csp,
        &html,
        std::env::var_os(REQUIRE_REAL_SPA).is_some(),
        sha256_source,
        "Run `bun run build` in web/ first (or `nix build .#spaShellCsp`) to check the real one.",
    );
}

/// The scanner is the oracle, so it gets its own known answers.
#[test]
fn the_independent_scanner_reads_attributes_the_way_a_browser_does() {
    let html = "<!-- <script>commented</script> --><HTML><body>\
        <div data-x='a > b' onClick=\"go()\">\
        <SCRIPT type=module data-src=\"/no.js\">window.a=1</SCRIPT>\
        <script async src=/a.js></script>\
        <script type=\"importmap\">{}</script>";
    let (tags, scripts) = scan(html);
    assert_eq!(scripts.len(), 3, "the commented-out script is not a script");
    assert_eq!(scripts[0].body, "window.a=1");
    assert_eq!(scripts[0].tag.attr("src"), None, "data-src is not src");
    assert_eq!(scripts[1].tag.attr("src"), Some("/a.js"));
    assert_eq!(scripts[1].tag.attr("async"), Some(""));
    assert_eq!(scripts[2].tag.attr("type"), Some("importmap"));
    let div = tags
        .iter()
        .find(|tag| tag.name == "div")
        .expect("div parsed");
    assert_eq!(
        div.attr("data-x"),
        Some("a > b"),
        "a quoted `>` does not end the tag"
    );
    assert_eq!(div.attr("onclick"), Some("go()"));
    assert_eq!(
        directive(
            "default-src 'self'; script-src 'self' 'sha256-x'",
            "script-src"
        ),
        ["'self'", "'sha256-x'"]
    );
}
