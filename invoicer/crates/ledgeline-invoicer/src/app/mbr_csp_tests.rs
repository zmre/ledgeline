//! The policies on what MBR serves and renders (security review SEC-1,
//! SEC-3, SEC-6): no inline script and nothing from another host anywhere,
//! a CSP sandbox on the books' content, scripts only on MBR's own UI and on
//! the page a PDF export prints (`tests.rs`,
//! `the_export_link_opens_the_page_once`), and raw HTML in notes sanitized.
//! The PDF itself is `tests/cli.rs`'s ignored `pdf_` tests: they print
//! under these policies in a real webview.

use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use mbr_core::csp::{self, ContentClass};

use super::mbr::{EXPORT_PASS_PARAM, SANDBOXED_CSP, STRICT_CSP, content_security_policy};
use super::tests::{App, HOST, app_on_prepared, authed, post_json, send};
use crate::starters;

fn csp_of(headers: &axum::http::HeaderMap) -> String {
    headers
        .get(header::CONTENT_SECURITY_POLICY)
        .expect("a CSP")
        .to_str()
        .unwrap()
        .to_string()
}

/// One directive of `policy` (`"img-src"` → `"img-src 'self' data:"`).
fn directive<'a>(policy: &'a str, name: &str) -> &'a str {
    policy
        .split(';')
        .map(str::trim)
        .find(|d| d.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("no {name} in {policy}"))
}

#[test]
fn the_policies_allow_no_inline_script_and_no_other_host() {
    for policy in [STRICT_CSP, SANDBOXED_CSP] {
        assert_eq!(directive(policy, "script-src"), csp::STRICT_SCRIPT_SRC);
        assert!(!policy.contains("unsafe-eval"), "{policy}");
        assert!(
            !policy.contains("https:") && !policy.contains("http:"),
            "{policy}"
        );
        assert!(!policy.contains('*'), "{policy}");
        assert_eq!(directive(policy, "default-src"), "default-src 'self'");
        assert_eq!(directive(policy, "img-src"), "img-src 'self' data:");
        assert_eq!(directive(policy, "media-src"), "media-src 'self'");
        assert_eq!(directive(policy, "frame-src"), "frame-src 'self'");
        assert_eq!(directive(policy, "connect-src"), "connect-src 'self'");
        assert_eq!(directive(policy, "object-src"), "object-src 'none'");
        assert_eq!(
            directive(policy, "frame-ancestors"),
            "frame-ancestors 'self'"
        );
    }
    // `'unsafe-inline'` only for styles (templates are a <style> block).
    assert_eq!(STRICT_CSP.matches("'unsafe-inline'").count(), 1);
    assert_eq!(
        directive(STRICT_CSP, "style-src"),
        "style-src 'self' 'unsafe-inline'"
    );
    // The sandbox: MBR's, with no `allow-scripts`.
    assert_eq!(directive(SANDBOXED_CSP, "sandbox"), csp::SANDBOX);
    assert!(!SANDBOXED_CSP.contains("allow-scripts"));
    assert!(!STRICT_CSP.contains("sandbox"));
    assert_eq!(
        SANDBOXED_CSP,
        format!("{}; {STRICT_CSP}", csp::SANDBOX),
        "the same policy, sandboxed"
    );

    let policy = content_security_policy();
    assert_eq!(policy.for_class(ContentClass::Pages), Some(SANDBOXED_CSP));
    assert_eq!(
        policy.for_class(ContentClass::UserFiles),
        Some(SANDBOXED_CSP)
    );
    assert_eq!(policy.for_class(ContentClass::MbrAssets), Some(STRICT_CSP));
}

/// `source` without its Tera comments (`{# … #}`), which document inline
/// script examples a browser never sees.
fn without_tera_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("{#") {
        out.push_str(&rest[..start]);
        match rest[start..].find("#}") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn the_shipped_templates_need_no_inline_script() {
    for starter in starters::STARTERS.iter().chain([&starters::BASE]) {
        let source = without_tera_comments(starter.source);
        let violations = csp::strict_script_violations(&source, &[]);
        assert!(violations.is_empty(), "{}: {violations:#?}", starter.file);
    }
}

/// `GET path` with the token: the status, the policy and the body.
async fn policy_of(app: &App, path: &str) -> (StatusCode, String, String) {
    let (status, headers, body) = send(&app.router, authed(path)).await;
    (status, csp_of(&headers), body)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_books_content_is_sandboxed_and_mbrs_ui_is_not() {
    let app = app_on_prepared("accrual", None, |books: &Path| {
        std::fs::write(
            books.join("customers/contracts/sow.html"),
            "<!doctype html><p>SOW</p><script>fetch('/.mbr/site.json')</script>",
        )
        .unwrap();
    });
    // A rendered note, the books' home page, a contract `.html` and an image.
    for path in [
        "/customers/contracts/Customer%20X%20MSA%202026/",
        "/",
        "/customers/contracts/sow.html",
        "/company/logo.png",
    ] {
        let (status, policy, _) = policy_of(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(policy, SANDBOXED_CSP, "{path}");
    }
    // A PDF keeps the strict policy without the sandbox, which would stop
    // the browser's PDF viewer.
    let (status, policy, _) = policy_of(
        &app,
        "/customers/contracts/Customer%20X%20SOW%203%20renewal%20%28signed%29.pdf",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(policy, STRICT_CSP);
    // MBR's own scripts, among them the field the SPA's prose editor loads.
    for path in [
        "/.mbr/mbr-bootstrap.js",
        "/.mbr/components/mbr-export-ready.min.js",
        "/.mbr/components/mbr-markdown-field.min.js",
    ] {
        let (status, policy, _) = policy_of(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(policy, STRICT_CSP, "{path}");
    }
    // A made-up export pass changes nothing.
    let (status, policy, _) = policy_of(
        &app,
        &format!("/customers/contracts/Customer%20X%20MSA%202026/?{EXPORT_PASS_PARAM}=made-up"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(policy, SANDBOXED_CSP);
    // Without the token: refused, as before.
    let request = Request::get("/customers/contracts/sow.html")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app.router, request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// What a pasted timesheet cell or a shared note could carry.
const HOSTILE: &str = "\n\n<script>alert('script')</script>\n\n\
| Item | Note |\n|---|---|\n| Widget<br>two lines | <img src=\"x\" onerror=\"alert('handler')\"> |\n\n\
<a href=\"javascript:alert('url')\">link</a>\n";

/// Asserts `page` carries none of [`HOSTILE`]'s script, and its `<br>`.
fn assert_defused(page: &str, what: &str) {
    for gone in ["alert('script')", "onerror", "javascript:"] {
        assert!(!page.contains(gone), "{what}: {gone} in {page}");
    }
    assert!(page.contains("Widget<br"), "{what}: {page}");
    let violations = csp::strict_script_violations(page, &[]);
    assert!(violations.is_empty(), "{what}: {violations:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_html_in_notes_is_sanitized_whatever_the_books_ask() {
    let note = "invoices/2026-11-10 Customer X Invoice 105.md";
    let app = app_on_prepared("accrual", None, |books: &Path| {
        // A books folder can't turn the floor off.
        std::fs::create_dir_all(books.join(".mbr")).unwrap();
        std::fs::write(
            books.join(".mbr/config.toml"),
            "raw_html = \"passthrough\"\n",
        )
        .unwrap();
        std::fs::write(
            books.join("customers/Hostile.md"),
            format!("# Hostile{HOSTILE}"),
        )
        .unwrap();
        let invoice = books.join(note);
        let saved = std::fs::read_to_string(&invoice).unwrap();
        std::fs::write(&invoice, format!("{saved}{HOSTILE}")).unwrap();
    });
    // MBR's pages: a plain note, and an invoice through its template.
    for path in [
        "/customers/Hostile/",
        "/invoices/2026-11-10%20Customer%20X%20Invoice%20105/",
    ] {
        let (status, _, page) = policy_of(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {page}");
        assert_defused(&page, path);
    }
    // The preview (MBR's standalone renderer), of unsaved text.
    let saved = std::fs::read_to_string(app.books.vault().root().join(note)).unwrap();
    let (status, _, text) = send(
        &app.router,
        post_json(
            "/api/preview",
            &serde_json::json!({"number": "105", "source": format!("{saved}\n<script>alert('script')</script>\n")}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_defused(json["html"].as_str().unwrap(), "preview");
}
