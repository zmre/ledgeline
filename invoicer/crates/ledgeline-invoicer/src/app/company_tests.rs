//! Drafts follow the company note; Send freezes the copy; Update from
//! company rewrites a sent one (invoicer PLAN D18). In-process over a
//! private copy of the accrual fixture.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{StatusCode, header};
use serde_json::{Value, json};

use super::native::mock::MockNative;
use super::tests::{App, app_on_prepared, authed, books_root, get, json_of, post_json, send};

const INVOICE_105: &str = "invoices/2026-11-10 Customer X Invoice 105.md";
const INVOICE_108: &str = "invoices/2026-11-17 Customer X Invoice 108.md";
const PAGE_105: &str = "/invoices/2026-11-10%20Customer%20X%20Invoice%20105/";
const PAGE_108: &str = "/invoices/2026-11-17%20Customer%20X%20Invoice%20108/";
const COMPANY: &str = "company/My Company.md";
const LOGO: &str = "<img class=\"logo\"";
/// MBR's own page: its chrome, which a page rendered here doesn't have.
const MBR_CHROME: &str = "<mbr-nav>";

fn edit(root: &Path, file: &str, from: &str, to: &str) {
    let path = root.join(file);
    let text = std::fs::read_to_string(&path).expect("reads");
    assert_eq!(text.matches(from).count(), 1, "{from:?} once in {file}");
    std::fs::write(&path, text.replacen(from, to, 1)).expect("writes");
}

fn read(app: &App, file: &str) -> String {
    std::fs::read_to_string(books_root(app).join(file)).expect("reads")
}

/// The accrual books with 108 drafted before the company had a logo, on
/// Classic (a starter, whose base includes MBR's export-ready partial).
fn before_the_logo(native: Option<Arc<MockNative>>) -> App {
    app_on_prepared(
        "accrual",
        native.map(|n| n as Arc<dyn super::native::Native>),
        |root| {
            edit(root, INVOICE_108, "invoicer_logo: /company/logo.png\n", "");
            edit(
                root,
                INVOICE_108,
                "/invoices/tpl/myco-default.html",
                "/invoices/tpl/classic.html",
            );
        },
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_made_before_the_logo_shows_it_in_the_preview_export_and_sent_copy() {
    let native = Arc::new(MockNative::everything());
    let app = before_the_logo(Some(Arc::clone(&native)));
    let draft = read(&app, INVOICE_108);
    assert!(!draft.contains("invoicer_logo"));

    // The preview: saved, and the editor's unsaved text.
    let (status, page) = json_of(&app, post_json("/api/preview", &json!({"number": "108"}))).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page["html"].as_str().unwrap().contains(LOGO));
    let (status, page) = json_of(
        &app,
        post_json(
            "/api/preview",
            &json!({"path": INVOICE_108, "source": draft.replace("Next month", "This month")}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let html = page["html"].as_str().unwrap();
    assert!(html.contains(LOGO) && html.contains("This month"), "{html}");

    // The page the app's frame and MBR's browser show, under MBR's policy.
    let (status, headers, page) = send(&app.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains(LOGO), "{page}");
    assert!(page.contains("Platform subscription, 1 year"), "{page}");
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        super::mbr::SANDBOXED_CSP
    );
    // Never the preview's flag: what a customer gets has no "Your logo" box.
    assert!(!page.contains("logo-slot"));

    // The export prints that page: the logo is there, under the policy that
    // lets the export-ready signal run.
    let (status, body) = json_of(&app, post_json("/api/invoices/108/pdf", &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let url = native.printed.lock().unwrap().last().unwrap().url.clone();
    let link = url.strip_prefix("http://127.0.0.1:5199").unwrap();
    let (status, headers, _) = send(&app.router, get(link).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = headers
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.starts_with(PAGE_108), "{location}");
    let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    let cookie = cookie.split(';').next().unwrap().to_string();
    let request = get(&location)
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let (status, headers, page) = send(&app.router, request).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains(LOGO), "{page}");
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        super::mbr::STRICT_CSP
    );
    let violations = mbr_core::csp::strict_script_violations(&page, &[]);
    assert!(violations.is_empty(), "{violations:#?}");
    // The base's export-ready partial is in it, so the print waits for the
    // logo and fonts as it does on MBR's page.
    assert!(page.contains("mbr-export-ready.min.js"), "{page}");

    // Showing, previewing and exporting wrote nothing.
    assert_eq!(read(&app, INVOICE_108), draft);

    // Send freezes the copy: the sent file carries the logo, and MBR serves
    // it as it is, chrome and all.
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/108/send",
            &json!({"via": "email", "today": "2026-11-17"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = read(&app, INVOICE_108);
    assert!(
        sent.contains("invoicer_logo: /company/logo.png\n"),
        "{sent}"
    );
    let (status, _, page) = send(&app.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains(LOGO) && page.contains(MBR_CHROME), "{page}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_current_draft_and_other_notes_are_mbrs_own_pages() {
    let app = app_on_prepared("accrual", None, |_| {});
    for page_url in [PAGE_108, PAGE_105, "/customers/Customer%20X/"] {
        let (status, _, page) = send(&app.router, authed(page_url)).await;
        assert_eq!(status, StatusCode::OK, "{page_url}: {page}");
        assert!(page.contains(MBR_CHROME), "{page_url}");
    }
}

/// 105 is sent, with a kept PDF's `sent_hash`; the company has moved since.
fn sent_then_moved() -> App {
    app_on_prepared("accrual", None, |root| {
        let path = root.join(INVOICE_105);
        let text = std::fs::read_to_string(&path).unwrap();
        let hash = invoicer_core::sent::hash(&text);
        std::fs::write(
            &path,
            text.replacen("paid: no\n", &format!("paid: no\nsent_hash: {hash}\n"), 1),
        )
        .unwrap();
        edit(root, COMPANY, "1234 Background Blvd.", "77 Harbor Way");
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sent_invoice_keeps_what_was_sent_until_updated_from_the_company() {
    let app = sent_then_moved();
    let original = read(&app, INVOICE_105);

    // Shown as sent, with the hint.
    let (status, _, page) = send(&app.router, authed(PAGE_105)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("1234 Background Blvd.") && !page.contains("77 Harbor Way"));
    let (status, detail) = json_of(&app, authed("/api/invoices/105")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["company_changed"], true);
    assert_eq!(read(&app, INVOICE_105), original);
    // A draft follows instead; it never has the hint.
    let (_, detail) = json_of(&app, authed("/api/invoices/108")).await;
    assert_eq!(detail["company_changed"], false);
    assert!(
        detail["invoicer"]["address"]
            .as_str()
            .unwrap()
            .starts_with("77 Harbor Way"),
        "{detail}"
    );

    // Update from company: only the address line, re-booked (nothing to
    // change in the books), `sent_hash` left for the next export to compare.
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/company", &json!({"today": "2026-11-17"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], json!(["invoicer_address"]));
    assert_eq!(body["written"], true);
    assert_eq!(body["rebooked"]["changed"], false, "{body}");
    assert_eq!(
        read(&app, INVOICE_105),
        original.replacen("  1234 Background Blvd.\n", "  77 Harbor Way\n", 1)
    );
    let (_, delivery) = json_of(&app, authed("/api/invoices/105/pdf")).await;
    assert_eq!(delivery["changed_since_sent"], true, "{delivery}");
    let (_, detail) = json_of(&app, authed("/api/invoices/105")).await;
    assert_eq!(detail["company_changed"], false);

    // Again: nothing to write.
    let (status, body) = json_of(&app, post_json("/api/invoices/105/company", &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["written"], false);
    assert_eq!(body["rebooked"], Value::Null);

    // A void invoice is history.
    let void = read(&app, "invoices/2026-10-01 Tailspin Invoice 103.md");
    let (status, body) = json_of(&app, post_json("/api/invoices/103/company", &json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["error"].as_str().unwrap().contains("void"), "{body}");
    assert_eq!(
        read(&app, "invoices/2026-10-01 Tailspin Invoice 103.md"),
        void
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sent_invoice_in_a_closed_period_is_refused_before_its_file_changes() {
    // 101 is dated 2026-09-01; the books are closed through 2026-09-30.
    let app = sent_then_moved();
    let file = "invoices/2026-09-01 Contoso Invoice 101.md";
    let before = read(&app, file);
    let (_, detail) = json_of(&app, authed("/api/invoices/101")).await;
    assert_eq!(detail["company_changed"], true);
    let (status, body) = json_of(&app, post_json("/api/invoices/101/company", &json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["error"].as_str().unwrap().contains("closed"), "{body}");
    assert_eq!(read(&app, file), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_company_nothing_changes() {
    let app = app_on_prepared("accrual", None, |root| {
        edit(root, "ledgeline.toml", "company = \"[[My Company]]\"\n", "");
        edit(root, INVOICE_108, "invoicer_logo: /company/logo.png\n", "");
    });
    let draft = read(&app, INVOICE_108);
    // The draft is MBR's page of the file as it is: no logo to follow.
    let (status, _, page) = send(&app.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(MBR_CHROME) && !page.contains(LOGO), "{page}");
    let (_, detail) = json_of(&app, authed("/api/invoices/105")).await;
    assert_eq!(detail["company_changed"], false);
    let (status, body) = json_of(&app, post_json("/api/invoices/105/company", &json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("no company"),
        "{body}"
    );
    // Sending keeps the copy the draft has.
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/108/send", &json!({"today": "2026-11-17"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = read(&app, INVOICE_108);
    assert!(!sent.contains("invoicer_logo"), "{sent}");
    assert_eq!(
        sent.lines()
            .filter(|l| l.starts_with("invoicer"))
            .collect::<Vec<_>>(),
        draft
            .lines()
            .filter(|l| l.starts_with("invoicer"))
            .collect::<Vec<_>>()
    );
}
