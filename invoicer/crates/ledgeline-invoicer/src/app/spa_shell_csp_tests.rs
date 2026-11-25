//! The SPA shell's Content-Security-Policy, checked against the shell that is
//! ACTUALLY embedded in this build: the invoicer's twin of Ledgeline's
//! `crates/ledgeline-server/tests/spa_shell_csp.rs`, with the same oracle
//! (`crates/ledgeline-shell/tests/support/shell_csp_oracle.rs`).
//!
//! A unit test rather than an integration test because the crate is a binary
//! and its router is `pub(crate)`. The `invoicerSpaShellCsp` Nix check embeds
//! the real `invoicerSpaBuild` and sets `LEDGELINE_REQUIRE_REAL_SPA`, under
//! which build.rs's placeholder is a failure; a plain `cargo test` passes on
//! either. The browser-level twin is `invoicer/web/e2e/csp.same-origin.e2e.ts`.

#[path = "../../../../../crates/ledgeline-shell/tests/support/shell_csp_oracle.rs"]
mod oracle;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};

use super::tests::{HOST, app, send};

const REQUIRE_REAL_SPA: &str = "LEDGELINE_REQUIRE_REAL_SPA";

#[tokio::test(flavor = "multi_thread")]
async fn spa_shell_csp_admits_exactly_the_inline_scripts_it_serves() {
    let app = app();
    // The shell itself and a deep link (the adapter's index.html fallback):
    // the same bytes, but each is a separate response with its own policy.
    for path in ["/app/", "/app/invoices/105"] {
        let request = Request::get(path)
            .header(header::HOST, HOST)
            .body(Body::empty())
            .expect("request");
        let (status, headers, html) = send(&app.router, request).await;
        assert_eq!(status, StatusCode::OK, "{path}: the shell must be served");
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_else(|| panic!("{path}: the shell carries a CSP"));
        oracle::assert_shell_csp(
            csp,
            &html,
            std::env::var_os(REQUIRE_REAL_SPA).is_some(),
            |script| format!("'{}'", ledgeline_shell::security::csp_sha256(script)),
            "Run `just build-invoicer` first (or `nix build .#invoicerSpaShellCsp`) to check \
             the real one.",
        );
    }
}
