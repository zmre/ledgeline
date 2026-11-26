//! The app's router in-process, over a private copy of the accrual fixture.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use ledgeline_shell::AppDirs;
use ledgeline_shell::security::{AccessToken, Security};
use tower::ServiceExt as _;

use super::books::OpenBooks;
use super::{AppState, router};

pub(super) const TOKEN: &str = "router-test-token-0123456789";
const PORT: u16 = 5199;
pub(super) const HOST: &str = "127.0.0.1:5199";

pub(super) fn copy_tree(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).expect("mkdir");
    for entry in std::fs::read_dir(src).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = dest.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// A copy of `invoicer/fixtures/accrual`, and its main journal.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    fixture_named("accrual")
}

/// A copy of `invoicer/fixtures/{name}`, and its main journal.
fn fixture_named(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    copy_tree(&src, &dir.path().join("books"));
    let journal = dir.path().join("books/books.journal");
    (dir, journal)
}

pub(super) struct App {
    _dir: tempfile::TempDir,
    pub(super) books: Arc<OpenBooks>,
    pub(super) router: Router,
}

/// Must run inside a tokio runtime (MBR spawns on it).
pub(super) fn app() -> App {
    app_with(None)
}

/// [`app`] with a native side (the PDF tests' mock).
fn app_with(native: Option<Arc<dyn super::native::Native>>) -> App {
    app_on("accrual", native)
}

/// [`app_with`] over the fixture `name` (`accrual`, `cash`).
pub(super) fn app_on(name: &str, native: Option<Arc<dyn super::native::Native>>) -> App {
    app_on_prepared(name, native, |_| {})
}

/// [`app_on`], with `prepare` run on the copied books folder before the
/// books open (to set file times, say).
pub(super) fn app_on_prepared(
    name: &str,
    native: Option<Arc<dyn super::native::Native>>,
    prepare: impl FnOnce(&Path),
) -> App {
    let (dir, journal) = fixture_named(name);
    prepare(&dir.path().join("books"));
    let books = Arc::new(OpenBooks::open(&journal, false).expect("opens"));
    let app_dirs = AppDirs::under(dir.path().join("config"));
    let state = AppState::with_native(Some(Arc::clone(&books)), app_dirs, native);
    let token = AccessToken::parse(TOKEN).expect("token");
    let security = Security::local(token.clone(), PORT);
    App {
        _dir: dir,
        books,
        router: router(state, security, token, PORT),
    }
}

pub(super) async fn send(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = router.clone().oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

pub(super) fn get(path: &str) -> axum::http::request::Builder {
    Request::get(path).header(header::HOST, HOST)
}

pub(super) fn authed(path: &str) -> Request<Body> {
    get(path)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("request")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_api_needs_the_token() {
    let app = app();
    let (status, _, _) = send(
        &app.router,
        get("/api/version").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, headers, body) = send(&app.router, authed("/api/version")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("ledgeline-invoicer"), "{body}");
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_api_answers_the_cli_shapes() {
    let app = app();
    let (status, _, body) = send(&app.router, authed("/api/books")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let books: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(books["basis"], "accrual");
    assert_eq!(books["nextNumber"], "2005");
    assert_eq!(books["journal"], "books.journal");

    let (status, _, body) = send(
        &app.router,
        authed("/api/invoices?status=sent&today=2026-11-22"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: serde_json::Value = serde_json::from_str(&body).expect("json");
    let list = list.as_array().expect("array");
    assert!(!list.is_empty());
    assert!(list.iter().all(|d| d["status"] == "sent"), "{body}");

    let (status, _, body) = send(&app.router, authed("/api/invoices/105")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"number\":\"105\""), "{body}");
    // One envelope everywhere (RA-4): `{error, code}`, the code the status's.
    let (status, missing) = json_of(&app, authed("/api/invoices/999")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["code"], "not-found", "{missing}");
    assert!(missing["error"].is_string(), "{missing}");
    let (status, bad) = json_of(&app, authed("/api/invoices?status=nope")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(bad["code"], "bad-request", "{bad}");

    let (status, _, body) = send(&app.router, authed("/api/customers")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Contoso"), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_prefs_are_the_shared_theme_and_scale() {
    let app = app();
    let (status, _, body) = send(&app.router, authed("/api/ui-prefs")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let prefs: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(prefs["prefs"]["theme"], "default");
    assert_eq!(prefs["prefs"]["uiScale"], 100);
    let patch = Request::patch("/api/ui-prefs")
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"theme":"catppuccin"}"#))
        .unwrap();
    let (status, _, body) = send(&app.router, patch).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("catppuccin"), "{body}");
    // A key only Ledgeline knows is refused, not written.
    let patch = Request::patch("/api/ui-prefs")
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"autoPrices":true}"#))
        .unwrap();
    let (status, _, _) = send(&app.router, patch).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A file that won't be overwritten says so without naming the other
    // app (RA-13).
    let app_dirs = AppDirs::under(books_root(&app).parent().unwrap().join("config"));
    let file = app_dirs.config_file("ui-prefs.toml").unwrap();
    std::fs::write(&file, "theme = [\n").unwrap();
    let patch = Request::patch("/api/ui-prefs")
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"theme":"gruvbox"}"#))
        .unwrap();
    let (status, _, body) = send(&app.router, patch).await;
    assert!(status.is_client_error(), "{status} {body}");
    assert!(body.contains("not overwritten"), "{body}");
    assert!(!body.contains("Ledgeline"), "{body}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "theme = [\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn mbr_pages_need_the_token_or_the_session_cookie() {
    let app = app();
    let (status, _, _) = send(&app.router, get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, headers, body) = send(&app.router, authed("/")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("/.mbr/"), "an MBR page: {body}");
    let csp = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(csp, super::mbr::SANDBOXED_CSP);

    let session = Request::post("/api/session")
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&app.router, session).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
        "{cookie}"
    );
    let pair = cookie.split(';').next().unwrap();
    assert!(pair.starts_with("ledgeline_invoicer_5199="), "{pair}");

    let with_cookie = get("/")
        .header(header::COOKIE, format!("other=1; {pair}"))
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app.router, with_cookie).await;
    assert_eq!(status, StatusCode::OK);
    let forged = get("/")
        .header(header::COOKIE, "ledgeline_invoicer_5199=nope")
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app.router, forged).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_spa_is_served_under_app_without_a_token() {
    let app = app();
    for path in ["/app", "/app/", "/app/invoices/105"] {
        let (status, headers, body) =
            send(&app.router, get(path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert!(body.contains("__LEDGELINE_EMBEDDED__"), "{path}: {body}");
        assert!(!body.contains(TOKEN));
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("script-src 'self' 'sha256-"), "{csp}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_foreign_host_is_refused_everywhere() {
    let app = app();
    for path in ["/api/version", "/", "/app/"] {
        let request = Request::get(path)
            .header(header::HOST, "evil.example")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = send(&app.router, request).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_reconciler_follows_changes() {
    let app = app();
    let books = &app.books;
    // The fixture is in step: nothing to write on open.
    let first = books.reconcile_all().expect("ran");
    assert!(first.changed.is_empty(), "{:?}", first.changed);

    // A hand edit to a paid invoice's mirror is put back.
    let root = books.vault().root().to_path_buf();
    let file = root.join("invoices/2026-09-01 Contoso Invoice 101.md");
    let text = std::fs::read_to_string(&file).expect("reads");
    std::fs::write(&file, text.replace("paid: 2026-10-05", "paid: no")).expect("writes");
    let report = books
        .apply_changes(std::slice::from_ref(&file))
        .expect("ran");
    assert_eq!(report.changed.len(), 1, "{:?}", report.changed);
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("paid: 2026-10-05")
    );
    // The write comes back as an event; it is a no-op.
    assert!(books.apply_changes(std::slice::from_ref(&file)).is_none());

    // A journal change that moves no invoice reconciles nothing.
    let journal = root.join("books.journal");
    let text = std::fs::read_to_string(&journal).expect("reads");
    std::fs::write(
        &journal,
        format!("{text}\n2026-11-20 Coffee\n    expenses:office  $4\n    assets:checking\n"),
    )
    .expect("writes");
    assert!(
        books
            .apply_changes(std::slice::from_ref(&journal))
            .is_none()
    );

    // A settings file whose `[invoicer]` and `[books]` read as before (a
    // touch, a Save's echo, a theme pin) reconciles nothing.
    let settings = root.join("ledgeline.toml");
    assert!(
        books
            .apply_changes(std::slice::from_ref(&settings))
            .is_none()
    );
    // A settings change re-opens the session and reconciles everything.
    let text = std::fs::read_to_string(&settings).expect("reads");
    std::fs::write(
        &settings,
        format!("{text}\n[invoicer.words]\ntax = [\"VAT\"]\n"),
    )
    .expect("writes");
    let all = books
        .apply_changes(std::slice::from_ref(&settings))
        .expect("ran");
    assert!(all.checked > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_paint_reads_the_shared_prefs() {
    let (dir, journal) = fixture();
    let books = Arc::new(OpenBooks::open(&journal, false).expect("opens"));
    let app_dirs = AppDirs::under(dir.path().join("state"));
    app_dirs.create_all().unwrap();
    let prefs = app_dirs.config_file("ui-prefs.toml").unwrap();
    std::fs::write(prefs, "theme = \"gruvbox\"\nuiScale = 125\n").unwrap();
    let state = AppState::new(Some(books), app_dirs);
    let appearance = super::prefs::boot_appearance(&state);
    assert_eq!(
        (appearance.light, appearance.dark, appearance.scale),
        ("gruvbox-light", "gruvbox-dark", 125)
    );
}

pub(super) fn post_json(path: &str, body: &serde_json::Value) -> Request<Body> {
    Request::post(path)
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// `POST /api/preview` with `body`: the status, the page (or the error) and
/// the `Server-Timing` header.
async fn preview(app: &App, body: serde_json::Value) -> (StatusCode, String, Option<String>) {
    let (status, headers, text) = send(&app.router, post_json("/api/preview", &body)).await;
    let json: serde_json::Value = serde_json::from_str(&text).expect("json");
    let page = json["html"]
        .as_str()
        .or_else(|| json["error"].as_str())
        .unwrap_or_default()
        .to_string();
    let timing = headers
        .get("server-timing")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (status, page, timing)
}

/// Every file under `root`, with its bytes: what "writes nothing" compares.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("reads");
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

/// The cells of the bill table's rows, as MBR renders them, are all there:
/// discounts, subtotal, deposit and total, with the books column's header.
fn assert_bill_table_105(page: &str) {
    for text in [
        "Senior architect hours",
        "Friends and family discount",
        "-10%",
        "-$800",
        "Junior engineer",
        "<strong>Subtotal</strong>",
        "<strong>$16,700</strong>",
        "Less deposit (Invoice 104)",
        "-$8,350",
        "<strong>Total due</strong>",
        "<strong>$8,350</strong>",
        // The books column is in the page; the template's rule hides it.
        "<em>Account</em>",
        "table:has(thead th:last-child &gt; em:only-child)",
    ] {
        assert!(
            page.contains(text) || page.contains(&text.replace("&gt;", ">")),
            "missing {text:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_renders_saved_and_unsaved_text_and_writes_nothing() {
    let app = app();
    let root = app.books.vault().root().to_path_buf();
    let before = snapshot(&root);

    // The saved file, through its own template (the fixture's).
    let (status, page, timing) = preview(&app, serde_json::json!({"number": "105"})).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Invoice 105"), "{page}");
    assert!(page.contains("Bill to Customer X"), "{page}");
    // The snapshot's phone numbers, the company's and the customer's.
    assert!(page.contains("+1 (555) 010-2000"), "{page}");
    assert!(page.contains("+1 (303) 555-0142"), "{page}");
    assert_bill_table_105(&page);
    // Both parties' addresses and contacts.
    for party in [
        "From My Company",
        "1234 Background Blvd.",
        "finance@mycompany.com",
        "1234 Main St. #555",
        "accounts_receivable@customerx.com",
    ] {
        assert!(page.contains(party), "missing {party:?}");
    }
    // A preview is the sheet alone: no MBR chrome outside MBR's server.
    assert!(!page.contains("<mbr-search>"), "{page}");
    let timing = timing.expect("Server-Timing");
    assert!(timing.starts_with("render;dur="), "{timing}");

    // Unsaved text, as the editor sends it.
    let file = root.join("invoices/2026-11-10 Customer X Invoice 105.md");
    let saved = std::fs::read_to_string(&file).unwrap();
    let edited = saved.replace("Senior architect hours", "Principal architect hours");
    let (status, page, _) = preview(
        &app,
        serde_json::json!({"path": "invoices/2026-11-10 Customer X Invoice 105.md", "source": edited}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Principal architect hours"), "{page}");
    assert!(!page.contains("Senior architect hours"));

    // A starter that isn't copied in yet renders from the shipped text.
    for (template, marker) in [
        ("/invoices/tpl/classic.html", "class=\"letterhead\""),
        ("/invoices/tpl/modern.html", "class=\"meta\""),
    ] {
        let (status, page, _) = preview(
            &app,
            serde_json::json!({"number": "105", "template": template}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{template}: {page}");
        assert!(page.contains(marker), "{template}: {page}");
        assert!(page.starts_with("<!doctype html>"), "{template}: {page}");
        assert_bill_table_105(&page);
        // MBR's export-ready partial (M5) is included.
        assert!(page.contains("mbr-export-ready"), "{template}: {page}");
        assert!(page.contains("PO #PY75334, rev. 2"), "{template}");
        assert!(
            page.contains("1234 Main St. #555<br>"),
            "{template}: {page}"
        );
        assert!(page.contains("Net 30"), "{template}");
    }

    assert_eq!(snapshot(&root), before, "a preview writes nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_starters_render_every_kind_of_row_and_state() {
    let app = app();
    for template in ["/invoices/tpl/classic.html", "/invoices/tpl/modern.html"] {
        let render = |number: &'static str| {
            preview(
                &app,
                serde_json::json!({"number": number, "template": template}),
            )
        };
        // Dates read as words; paid, void and draft are marked.
        let (_, paid, _) = render("101").await;
        assert!(paid.contains("paid\">Paid<"), "{template}: {paid}");
        let (_, void, _) = render("103").await;
        assert!(void.contains("void\">Void<"), "{template}: {void}");
        let (_, draft, _) = render("108").await;
        assert!(draft.contains("draft\">Draft<"), "{template}: {draft}");
        assert!(draft.contains("Platform subscription, 1 year"));
        let (_, open, _) = render("105").await;
        for state in ["paid\">Paid<", "void\">Void<", "draft\">Draft<"] {
            assert!(!open.contains(state), "{template}: {state}");
        }
        assert!(
            open.contains("November 10, 2026") || open.contains("Nov 10, 2026"),
            "{template}: {open}"
        );
        // Both phone numbers: the company's and the bill-to's.
        for phone in ["+1 (555) 010-2000", "+1 (303) 555-0142"] {
            assert!(open.contains(phone), "{template}: {phone}");
        }

        // A deposit invoice says what it is a deposit for.
        let (status, deposit, _) = render("104").await;
        assert_eq!(status, StatusCode::OK, "{deposit}");
        assert!(deposit.contains("Deposit for"), "{template}");
        assert!(deposit.contains("proposal P-12"), "{template}");
        assert!(deposit.contains("$8,350.00"), "{template}");

        // Taxes, discounts on a subtotal and a deferred row (ARCHITECTURE
        // §3.3's worked example), as unsaved text.
        let source = format!(
            "---\ntitle: Sample\ntype: invoice\ntemplate: {template}\nstatus: draft\n\
             invoice_num: 106\ndate: 2026-11-17\ndue_date: 2026-12-17\n\
             invoicee_name: Customer X\ninvoicer_name: My Company\n---\n\n{WORKED_EXAMPLE}"
        );
        let (status, page, _) = preview(
            &app,
            serde_json::json!({"path": "invoices/sample.md", "source": source}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{template}: {page}");
        for text in [
            "Loyalty discount",
            "Early-payment discount",
            "-2%",
            "Sales tax",
            "8.25%",
            "$1,637.21",
            "Less deposit (Invoice 104)",
            "<strong>$16,482.21</strong>",
            "Annual support plan",
        ] {
            assert!(page.contains(text), "{template}: {text}");
        }
        // The books column's cells are there for the CSS to hide.
        assert!(page.contains("liabilities:sales-tax"), "{template}");
    }
}

/// ARCHITECTURE §3.3's worked example: every row kind.
const WORKED_EXAMPLE: &str = "\
| Description                | Qty | Rate          | Amount         | *Account*                                                                  |
|:---------------------------|----:|--------------:|---------------:|:---------------------------------------------------------------------------|
| Website redesign           | 1   | $12,000       | $12,000.00     | revenue:projects                                                           |
| Senior architect hours     | 10  | $250          | $2,500.00      | revenue:consulting                                                         |
| Loyalty discount           |     | -10%          | -$250.00       |                                                                            |
| Annual support plan        | 1   | $6,000        | $6,000.00      | liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01  |
|                            |     |               |                |                                                                            |
|                            |     | **Subtotal**  | **$20,250.00** |                                                                            |
| Early-payment discount     |     | -2%           | -$405.00       |                                                                            |
|                            |     | **Subtotal**  | **$19,845.00** |                                                                            |
| Sales tax                  |     | 8.25%         | $1,637.21      | liabilities:sales-tax                                                      |
| Less deposit (Invoice 104) |     |               | -$5,000.00     | liabilities:customer-deposits                                              |
|                            |     | **Total due** | **$16,482.21** |                                                                            |
";

#[tokio::test(flavor = "multi_thread")]
async fn preview_refuses_what_it_cannot_render() {
    let app = app();
    let root = app.books.vault().root().to_path_buf();
    std::fs::write(
        root.join("invoices/tpl/broken.html"),
        "<p>{{ frontmatter.invoice_num </p>",
    )
    .unwrap();
    for (body, want) in [
        (serde_json::json!({}), StatusCode::BAD_REQUEST),
        (serde_json::json!({"number": "999"}), StatusCode::NOT_FOUND),
        (
            serde_json::json!({"path": "../outside.md", "source": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"path": ".mbr/x.md", "source": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"path": "invoices/x.txt", "source": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"path": "invoices/missing.md"}),
            StatusCode::NOT_FOUND,
        ),
        (
            serde_json::json!({"number": "105", "template": "/invoices/tpl/nope.html"}),
            StatusCode::NOT_FOUND,
        ),
        (
            serde_json::json!({"number": "105", "template": "/invoices/tpl/x.css"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"number": "105", "template": "/invoices/tpl/broken.html"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            serde_json::json!({"number": "105", "unknown": true}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, page, _) = preview(&app, body.clone()).await;
        assert_eq!(status, want, "{body}: {page}");
    }
    // The token is needed, as on every API route.
    let request = Request::post("/api/preview")
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"number":"105"}"#))
        .unwrap();
    let (status, _, _) = send(&app.router, request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_list_the_books_then_the_starters() {
    let app = app();
    let (status, _, body) = send(&app.router, authed("/api/templates")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: serde_json::Value = serde_json::from_str(&body).unwrap();
    let rows: Vec<(String, bool, bool)> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["path"].as_str().unwrap().to_string(),
                t["starter"].as_bool().unwrap(),
                t["installed"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("/invoices/tpl/myco-default.html".to_string(), false, true),
            ("/invoices/tpl/classic.html".to_string(), true, false),
            ("/invoices/tpl/modern.html".to_string(), true, false),
        ]
    );
    assert_eq!(list[0]["name"], "MyCo default");

    // A template written while the app runs is listed once the watcher
    // reports it (`.html` files refresh the index too).
    let root = app.books.vault().root().to_path_buf();
    let copied = root.join("invoices/tpl/classic.html");
    std::fs::write(&copied, crate::starters::DEFAULT.source).unwrap();
    app.books.apply_changes(std::slice::from_ref(&copied));
    let (_, _, body) = send(&app.router, authed("/api/templates")).await;
    let list: serde_json::Value = serde_json::from_str(&body).unwrap();
    let classic = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"] == "/invoices/tpl/classic.html")
        .expect("listed");
    assert_eq!(classic["installed"], true);
    assert_eq!(classic["starter"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_books_panel_says_what_the_journal_holds() {
    let app = app();
    let books = |number: &'static str| {
        let router = app.router.clone();
        async move {
            let (status, _, body) =
                send(&router, authed(&format!("/api/invoices/{number}/books"))).await;
            assert_eq!(status, StatusCode::OK, "{number}: {body}");
            serde_json::from_str::<serde_json::Value>(&body).unwrap()
        }
    };

    // Open, applying the deposit.
    let open = books("105").await;
    assert_eq!(open["basis"], "accrual");
    assert_eq!(open["bookings"].as_array().unwrap().len(), 1);
    let booking = &open["bookings"][0];
    assert_eq!(booking["date"], "2026-11-10");
    assert_eq!(booking["file"], "books.journal");
    let accounts: Vec<&str> = booking["postings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["account"].as_str().unwrap())
        .collect();
    assert!(
        accounts.contains(&"liabilities:customer-deposits"),
        "{accounts:?}"
    );
    assert_eq!(open["receivable"]["state"], "open");
    assert_eq!(open["receivable"]["outstanding"], "$8,350.00");
    assert_eq!(open["books_paid_text"], "no");
    assert_eq!(open["in_step"], true);

    // Paid in two parts.
    let paid = books("101").await;
    assert_eq!(paid["receivable"]["state"], "paid");
    assert_eq!(paid["receivable"]["payments"].as_array().unwrap().len(), 2);
    assert_eq!(paid["books_paid_text"], "2026-10-05");
    assert_eq!(paid["in_step"], true);

    // Void: the reversal is listed.
    let void = books("103").await;
    assert_eq!(void["voids"].as_array().unwrap().len(), 1);

    // A draft is never booked (I2).
    let draft = books("108").await;
    assert_eq!(draft["status"], "draft");
    assert!(draft["bookings"].as_array().unwrap().is_empty());
    assert!(draft["receivable"].is_null());

    let (status, _, _) = send(&app.router, authed("/api/invoices/999/books")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Where each payment came from, and the one line on top.
    let payment = &paid["receivable"]["payments"][0];
    assert_eq!(payment["date"], "2026-09-20");
    assert_eq!(payment["amount"], "$1,500.00");
    assert_eq!(payment["kind"], "payment");
    assert_eq!(payment["description"], "CONTOSO ACH");
    assert_eq!(payment["from"], serde_json::json!(["assets:bank:checking"]));
    assert_eq!(payment["file"], "books.journal");
    assert_eq!(
        paid["settlement"],
        serde_json::json!({"state": "paid", "total": "$4,000.00", "paid": "$4,000.00",
            "outstanding": "$0.00", "paid_on": "2026-10-05", "due_date": "2026-10-01"})
    );
    assert_eq!(open["settlement"]["state"], "unpaid");
    assert_eq!(open["settlement"]["outstanding"], "$8,350.00");
    assert_eq!(open["settlement"]["due_date"], "2026-12-10");
    let partial = books("107").await;
    assert_eq!(partial["settlement"]["state"], "partial");
    assert_eq!(partial["settlement"]["paid"], "$1,000.00");
    assert_eq!(partial["settlement"]["outstanding"], "$2,000.00");
    assert_eq!(void["settlement"]["state"], "void");
    assert!(draft["settlement"].is_null());
    for view in [&open, &paid, &partial] {
        assert!(view["mirror_problem"].is_null(), "{view}");
    }

    // A mirror the books disagree with is said in plain words (the file is
    // edited behind the reconciler's back).
    let root = books_root(&app);
    let file = root.join("invoices/2026-09-01 Contoso Invoice 101.md");
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("paid: 2026-10-05", "paid: no")).unwrap();
    {
        let mut session = app.books.session();
        let (index, vault) = session.parts_mut();
        let note = vault.vault_path(&file).unwrap();
        index.refresh(vault, &[note]);
    }
    let stale = books("101").await;
    assert_eq!(stale["in_step"], false);
    let problem = stale["mirror_problem"].as_str().expect("a mirror problem");
    assert!(
        problem.contains("says it is unpaid")
            && problem.contains("paid in full on 2026-10-05")
            && problem.contains("ledgeline-invoicer reconcile"),
        "{problem}"
    );
}

/// Cash books: the payments applied, and the one line from them.
#[tokio::test(flavor = "multi_thread")]
async fn the_books_panel_on_cash_books() {
    let app = app_on("cash", None);
    let view = |n: &'static str| {
        let app = &app;
        async move {
            json_of(app, authed(&format!("/api/invoices/{n}/books")))
                .await
                .1
        }
    };
    let paid = view("201").await;
    assert_eq!(paid["payments"][0]["description"], "STRIPE PAYOUT CONTOSO");
    assert_eq!(paid["settlement"]["state"], "paid");
    assert_eq!(paid["settlement"]["paid_on"], "2026-11-03");
    let partial = view("202").await;
    assert_eq!(
        partial["settlement"],
        serde_json::json!({"state": "partial", "total": "$2,000.00", "paid": "$500.00",
            "outstanding": "$1,500.00", "paid_on": null, "due_date": "2026-12-01"})
    );
    assert_eq!(view("203").await["settlement"]["state"], "unpaid");
    assert_eq!(view("205").await["settlement"]["state"], "void");
    assert!(view("204").await["settlement"].is_null(), "a draft");
}

#[tokio::test(flavor = "multi_thread")]
async fn mbr_serves_an_invoice_page_through_its_template() {
    let app = app();
    let (status, headers, page) = send(
        &app.router,
        authed("/invoices/2026-11-10%20Customer%20X%20Invoice%20105/"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Bill to Customer X"), "{page}");
    assert_bill_table_105(&page);
    // MBR's own page keeps its chrome (search, breadcrumbs, previous/next)
    // around the sheet; a frame without scripts hides it (<noscript>).
    for chrome in [
        "<mbr-search>",
        r#"class="container breadcrumbs""#,
        "<mbr-nav>",
        "body.invoice-page > :not(main) { display: none !important; }",
    ] {
        assert!(page.contains(chrome), "missing {chrome:?}");
    }
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        super::mbr::SANDBOXED_CSP
    );
    // MBR's chrome and the base template run no inline script.
    let violations = mbr_core::csp::strict_script_violations(&page, &[]);
    assert!(violations.is_empty(), "{violations:#?}");
}

/// The company's logo (`logo:` on its note, snapshotted as `invoicer_logo`):
/// in the header of the base (the fixture's MyCo), Classic and Modern when
/// the invoice carries it (105), absent when it doesn't (101). MBR serves
/// the image itself, behind the gate, so the frame, the preview and the
/// PDF export (its export link sets the cookie) all load it.
#[tokio::test(flavor = "multi_thread")]
async fn the_company_logo_shows_when_the_invoice_carries_it() {
    let app = app();
    let img = |page: &str| -> Option<String> {
        let at = page.find("<img class=\"logo\"")?;
        Some(page[at..at + page[at..].find('>')? + 1].to_string())
    };
    for template in [
        None,
        Some("/invoices/tpl/classic.html"),
        Some("/invoices/tpl/modern.html"),
    ] {
        let with = |number: &str| match template {
            Some(t) => serde_json::json!({"number": number, "template": t}),
            None => serde_json::json!({"number": number}),
        };
        let (status, page, _) = preview(&app, with("105")).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let tag = img(&page).unwrap_or_else(|| panic!("{template:?}: no logo in {page}"));
        assert!(
            tag.contains("company&#x2F;logo.png\"") || tag.contains("/company/logo.png\""),
            "{template:?}: {tag}"
        );
        assert!(tag.contains("alt=\"My Company\""), "{template:?}: {tag}");
        let (status, page, _) = preview(&app, with("101")).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(img(&page), None, "{template:?}: 101 has no logo");
    }

    // A file name with a space is a URL that works.
    let saved = std::fs::read_to_string(app.books.vault().root().join(INVOICE_105)).unwrap();
    let source = saved.replace("/company/logo.png", "/company/My Logo.png");
    let (_, page, _) = preview(
        &app,
        serde_json::json!({"path": INVOICE_105, "source": source}),
    )
    .await;
    let tag = img(&page).expect("logo");
    assert!(tag.contains("My%20Logo.png"), "{tag}");

    // MBR's own page for 105 has it too.
    let (status, _, page) = send(
        &app.router,
        authed("/invoices/2026-11-10%20Customer%20X%20Invoice%20105/"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(img(&page).is_some(), "{page}");

    // The image, served from the books folder behind the gate.
    let response = app
        .router
        .clone()
        .oneshot(authed("/company/logo.png"))
        .await
        .expect("infallible");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.starts_with(b"\x89PNG"), "a PNG");
    let (status, _, _) = send(
        &app.router,
        get("/company/logo.png").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// PDF and the Send flow (PLAN 3c), with the mock native side: it "prints" a
// stub `%PDF-` file. The real print is `pdf_export_prints_a_real_pdf` in
// `desktop_tests` (ignored: it needs a webview).
// ---------------------------------------------------------------------------

use super::native::mock::MockNative;

const INVOICE_105: &str = "invoices/2026-11-10 Customer X Invoice 105.md";
const INVOICE_108: &str = "invoices/2026-11-17 Customer X Invoice 108.md";
const PDF_105: &str = "2026-11-10 Customer X Invoice 105.pdf";

fn mocked() -> (App, Arc<MockNative>) {
    let native = Arc::new(MockNative::everything());
    let app = app_with(Some(Arc::clone(&native) as Arc<dyn super::native::Native>));
    (app, native)
}

pub(super) fn books_root(app: &App) -> PathBuf {
    app.books.vault().root().to_path_buf()
}

pub(super) async fn json_of(app: &App, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let (status, _, body) = send(&app.router, request).await;
    let value = serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body));
    (status, value)
}

#[tokio::test(flavor = "multi_thread")]
async fn pdf_routes_say_when_this_process_cannot_print() {
    let app = app();
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("desktop"),
        "{body}"
    );
    let (status, body) = json_of(&app, authed("/api/invoices/105/pdf")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["capabilities"],
        serde_json::json!({"pdf": false, "share": false, "copy_file": false, "open_url": false, "open_file": false, "pick_file": false, "open_ledgeline": false})
    );
    // The token, as everywhere under /api.
    let anonymous = Request::post("/api/invoices/105/pdf")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app.router, anonymous).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sent_invoice_is_kept_in_pdf_dir_with_its_earlier_exports() {
    let (app, native) = mocked();
    let root = books_root(&app);
    let pdf_dir = root.join("invoices/pdf");

    let (status, body) = json_of(&app, authed("/api/invoices/105/pdf")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["destination"], "kept");
    assert_eq!(body["file"], PDF_105);
    assert_eq!(body["pdf_dir"], "invoices/pdf");
    assert_eq!(body["changed_since_sent"], serde_json::Value::Null);
    assert_eq!(body["exports"], serde_json::json!([]));
    assert_eq!(body["copy"]["reference"], "PO #PY75334, rev. 2");
    assert_eq!(body["copy"]["total"], "$8,350.00");
    assert_eq!(body["portal"]["name"], "Coupa");
    assert_eq!(body["portal"]["url"], "https://customerx.coupahost.com/");
    assert_eq!(body["portal"]["requires_reference"], true);

    // The first export: kept, and the mirror written.
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["destination"], "kept");
    assert_eq!(body["file"], PDF_105);
    assert_eq!(body["kept_at"], format!("invoices/pdf/{PDF_105}"));
    assert_eq!(body["sent_hash"], "written");
    assert!(
        body.get("path").is_none(),
        "the API never names full paths: {body}"
    );
    assert!(body.get("earlier").is_none(), "{body}");
    let pdf = std::fs::read(pdf_dir.join(PDF_105)).expect("kept");
    assert!(pdf.starts_with(b"%PDF-"));
    {
        let printed = native.printed.lock().unwrap();
        let job = printed.last().expect("printed");
        assert!(
            job.url
                .starts_with("http://127.0.0.1:5199/.ledgeline-invoicer/export/"),
            "{}",
            job.url
        );
        // The fixture's template has no export-ready partial: print on load.
        assert!(job.ready_on_load);
        assert!(job.output.starts_with(&pdf_dir));
        assert!(
            job.output
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with('.')
        );
    }
    let text = std::fs::read_to_string(root.join(INVOICE_105)).unwrap();
    let hash = invoicer_core::sent::hash(&text);
    assert!(text.contains(&format!("sent_hash: {hash}\n")), "{text}");

    // Again: the earlier one is kept under its date.
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sent_hash"], "unchanged");
    assert_eq!(body["changed_since_sent"], false);
    let earlier = body["earlier"].as_str().expect("earlier kept");
    assert!(
        earlier.starts_with("invoices/pdf/2026-11-10 Customer X Invoice 105 "),
        "{earlier}"
    );
    assert!(root.join(earlier).is_file());
    let (_, delivery) = json_of(&app, authed("/api/invoices/105/pdf")).await;
    let exports = delivery["exports"].as_array().unwrap();
    assert_eq!(exports.len(), 2, "{delivery}");
    assert_eq!(exports[0]["file"], PDF_105);
    assert_eq!(exports[0]["latest"], true);
    assert_eq!(exports[1]["latest"], false);
    assert_eq!(delivery["changed_since_sent"], false);

    // A paid mirror moving is not a change; an edit is.
    std::fs::write(
        root.join(INVOICE_105),
        text.replace("paid: no", "paid: 2026-11-20"),
    )
    .unwrap();
    app.books.apply_changes(&[root.join(INVOICE_105)]);
    let (_, delivery) = json_of(&app, authed("/api/invoices/105/pdf")).await;
    assert_eq!(delivery["changed_since_sent"], false, "{delivery}");
    std::fs::write(
        root.join(INVOICE_105),
        text.replace("Hi Fred,", "Hi Fred and team,"),
    )
    .unwrap();
    let (_, delivery) = json_of(&app, authed("/api/invoices/105/pdf")).await;
    assert_eq!(delivery["changed_since_sent"], true, "{delivery}");
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed_since_sent"], true);
    assert_eq!(body["sent_hash"], "written");
    assert!(
        body["warnings"]
            .to_string()
            .contains("differs from what was sent"),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_export_link_opens_the_page_once() {
    let (app, native) = mocked();
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The mock never loaded its link: it is still good, once.
    let url = native.printed.lock().unwrap().last().unwrap().url.clone();
    let path = url
        .strip_prefix("http://127.0.0.1:5199")
        .unwrap()
        .to_string();
    let (status, headers, _) = send(&app.router, get(&path).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = headers
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pass = location
        .strip_prefix(
            "/invoices/2026-11-10%20Customer%20X%20Invoice%20105/?ledgeline-invoicer-export=",
        )
        .unwrap_or_else(|| panic!("the page, with an export pass: {location}"));
    assert!(pass.len() >= 32, "{pass}");
    let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    let pair = cookie.split(';').next().unwrap().to_string();
    let page = || {
        get(&location)
            .header(header::COOKIE, &pair)
            .body(Body::empty())
            .unwrap()
    };
    // The export page runs MBR's own scripts (its export-ready signal):
    // the strict policy, without the sandbox.
    let (status, headers, body) = send(&app.router, page()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Bill to Customer X"), "{body}");
    assert!(body.contains("mbr-bootstrap.js"), "{body}");
    let violations = mbr_core::csp::strict_script_violations(&body, &[]);
    assert!(violations.is_empty(), "{violations:#?}");
    assert_eq!(csp_of(&headers), super::mbr::STRICT_CSP);
    // The pass is spent: the same page again is sandboxed.
    let (status, headers, _) = send(&app.router, page()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(csp_of(&headers), super::mbr::SANDBOXED_CSP);
    // Spent.
    let (status, headers, _) = send(&app.router, get(&path).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.get(header::SET_COOKIE).is_none());
    let (status, _, _) = send(
        &app.router,
        get("/.ledgeline-invoicer/export/made-up")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_goes_to_a_temporary_file_and_writes_nothing() {
    let (app, native) = mocked();
    let root = books_root(&app);
    let before = snapshot(&root);
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/108/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["destination"], "temp");
    assert_eq!(body["file"], "2026-11-17 Customer X Invoice 108.pdf");
    assert!(
        body.get("kept_at").is_none() && body.get("sent_hash").is_none(),
        "{body}"
    );
    assert_eq!(
        snapshot(&root),
        before,
        "a draft's export touches nothing in the books"
    );
    let temp = crate::pdf::temp_dir()
        .unwrap()
        .join("2026-11-17 Customer X Invoice 108.pdf");
    assert!(temp.is_file());
    // RA-12: each temporary export prints into its own staging file, then
    // is renamed into place.
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/108/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let outputs: Vec<_> = native
        .printed
        .lock()
        .unwrap()
        .iter()
        .map(|job| job.output.clone())
        .collect();
    assert_eq!(outputs.len(), 2);
    assert_ne!(outputs[0], outputs[1]);
    for output in &outputs {
        assert_ne!(output, &temp);
        assert_eq!(output.parent(), temp.parent());
        assert!(
            !output.exists(),
            "{} was renamed into place",
            output.display()
        );
    }
    assert!(temp.is_file());

    // Share… and Copy file name the PDF by destination and file name.
    let share = serde_json::json!({
        "destination": "temp",
        "file": "2026-11-17 Customer X Invoice 108.pdf",
        "anchor": {"x": 10.0, "y": 20.0, "width": 80.0, "height": 24.0}
    });
    let (status, body) = json_of(&app, post_json("/api/pdf/share", &share)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/pdf/copy",
            &serde_json::json!({"destination": "temp", "file": "2026-11-17 Customer X Invoice 108.pdf"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for (bad, want) in [
        (
            serde_json::json!({"destination": "temp", "file": "../../etc/passwd.pdf"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"destination": "temp", "file": "notes.md"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"destination": "temp", "file": ".hidden.pdf"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"destination": "copy", "file": "x.pdf"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"destination": "kept", "file": "never exported.pdf"}),
            StatusCode::NOT_FOUND,
        ),
    ] {
        for route in ["/api/pdf/share", "/api/pdf/open"] {
            let (status, body) = json_of(&app, post_json(route, &bad)).await;
            assert_eq!(status, want, "{route} {bad}: {body}");
        }
    }
    // Open: the same handles, never a path; the vetted file goes to the viewer.
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/pdf/open",
            &serde_json::json!({"destination": "temp", "file": "2026-11-17 Customer X Invoice 108.pdf"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        *native.opened_files.lock().unwrap(),
        std::slice::from_ref(&temp)
    );
    let (status, _) = json_of(
        &app,
        post_json(
            "/api/pdf/open",
            &serde_json::json!({"destination": "temp", "file": "x.pdf", "path": "/etc/hosts"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unknown fields are refused"
    );
    let _ = std::fs::remove_file(temp);
}

/// Without a window, Open answers 503 (the SPA shows the name as text).
#[tokio::test(flavor = "multi_thread")]
async fn opening_a_pdf_needs_the_window() {
    let app = app();
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/pdf/open",
            &serde_json::json!({"destination": "temp", "file": "x.pdf"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_marks_a_draft_sent_by_portal_and_the_portal_opens() {
    let (app, native) = mocked();
    let root = books_root(&app);

    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/108/portal", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({"url": "https://customerx.coupahost.com/", "opened": true})
    );
    assert_eq!(
        *native.opened.lock().unwrap(),
        ["https://customerx.coupahost.com/"]
    );
    let (status, _) = json_of(
        &app,
        post_json("/api/invoices/106/portal", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "Wingtip has no portal");

    // The portal needs a reference: refused without one, nothing written.
    let text = std::fs::read_to_string(root.join(INVOICE_108)).unwrap();
    std::fs::write(
        root.join(INVOICE_108),
        text.replace("reference: \"PO #PY75334\"\n", ""),
    )
    .unwrap();
    app.books.apply_changes(&[root.join(INVOICE_108)]);
    let journal_before = std::fs::read(root.join("books.journal")).unwrap();
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/108/send",
            &serde_json::json!({"via": "portal", "today": "2026-11-17"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("reference"),
        "{body}"
    );
    assert_eq!(
        std::fs::read(root.join("books.journal")).unwrap(),
        journal_before
    );

    std::fs::write(root.join(INVOICE_108), &text).unwrap();
    app.books.apply_changes(&[root.join(INVOICE_108)]);
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/108/send",
            &serde_json::json!({"via": "portal", "today": "2026-11-17"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = std::fs::read_to_string(root.join(INVOICE_108)).unwrap();
    assert!(after.contains("status: sent\n"), "{after}");
    assert!(after.contains("sent_via: portal\n"), "{after}");
    assert!(after.contains("sent_date: 2026-11-17\n"), "{after}");
    let (_, delivery) = json_of(&app, authed("/api/invoices/108/pdf")).await;
    assert_eq!(delivery["destination"], "kept");
    assert_eq!(delivery["sent_via"], "portal");

    let (status, _) = json_of(
        &app,
        post_json(
            "/api/invoices/108/send",
            &serde_json::json!({"via": "a\nb"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// RA-2: an export copies in the starter its invoice names when the books
/// don't have it, as opening the app does (the CLI's `pdf` opens no app).
#[tokio::test(flavor = "multi_thread")]
async fn an_export_copies_in_the_starter_its_invoice_names() {
    let (app, native) = mocked();
    let root = books_root(&app);
    let file = root.join(INVOICE_108);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace(
            "template: /invoices/tpl/myco-default.html",
            "template: /invoices/tpl/modern.html",
        ),
    )
    .unwrap();
    {
        let mut session = app.books.session();
        let (index, vault) = session.parts_mut();
        let note = vault.vault_path(&file).unwrap();
        index.refresh(vault, &[note]);
    }
    let starter = root.join("invoices/tpl/modern.html");
    assert!(!starter.exists());
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/108/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        starter.is_file(),
        "the starter was copied in before printing"
    );
    assert_eq!(native.printed.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_print_keeps_nothing() {
    let native = Arc::new(MockNative {
        fail: Some("the print system said no".to_string()),
        ..MockNative::everything()
    });
    let app = app_with(Some(Arc::clone(&native) as Arc<dyn super::native::Native>));
    let root = books_root(&app);
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("the print system said no"),
        "{body}"
    );
    let left: Vec<_> = std::fs::read_dir(root.join("invoices/pdf"))
        .unwrap()
        .collect();
    assert!(left.is_empty(), "no staging file left behind");
    let text = std::fs::read_to_string(root.join(INVOICE_105)).unwrap();
    assert!(!text.contains("sent_hash"));
}

/// Marking a draft sent books it, and the app's own session sees the booking
/// at once: the Books panel and the invoice's checks read the journal as it
/// is now, not as the session last read it (Patrick's "Mark as sent" error,
/// 2026-11-23).
#[tokio::test(flavor = "multi_thread")]
async fn send_books_the_invoice_and_the_session_sees_it_at_once() {
    for (fixture, number, booked) in [("accrual", "108", true), ("cash", "204", false)] {
        let app = app_on(fixture, None);
        let root = books_root(&app);
        // The panel is read first, as the page does before the click.
        let (status, before) =
            json_of(&app, authed(&format!("/api/invoices/{number}/books"))).await;
        assert_eq!(status, StatusCode::OK, "{fixture}: {before}");
        assert_eq!(before["status"], "draft", "{fixture}");

        let (status, body) = json_of(
            &app,
            post_json(
                &format!("/api/invoices/{number}/send"),
                &serde_json::json!({"via": "email", "today": "2026-11-17"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{fixture}: {body}");
        let journal = std::fs::read_to_string(root.join("books.journal")).unwrap();
        assert_eq!(
            journal.contains(&format!("invoice: {number}")),
            booked,
            "{fixture}: accrual books book at send, cash books at payment"
        );

        let (status, panel) = json_of(&app, authed(&format!("/api/invoices/{number}/books"))).await;
        assert_eq!(status, StatusCode::OK, "{fixture}: {panel}");
        assert_eq!(panel["status"], "sent", "{fixture}: {panel}");
        assert_eq!(
            !panel["bookings"].as_array().unwrap().is_empty(),
            booked,
            "{fixture}: {panel}"
        );
        assert_eq!(
            panel["problems"],
            serde_json::json!([]),
            "{fixture}: no problem with a freshly sent invoice"
        );
        let (status, detail) = json_of(&app, authed(&format!("/api/invoices/{number}"))).await;
        assert_eq!(status, StatusCode::OK, "{fixture}: {detail}");
        assert!(
            !detail.to_string().contains("no transaction"),
            "{fixture}: {detail}"
        );
    }
}

/// The session cookie's `name=value`, as the SPA's `POST /api/session` gets it.
async fn session_cookie(app: &App) -> String {
    let session = Request::post("/api/session")
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&app.router, session).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    cookie.split(';').next().unwrap().to_string()
}

fn csp_of(headers: &axum::http::HeaderMap) -> String {
    headers
        .get(header::CONTENT_SECURITY_POLICY)
        .expect("a CSP")
        .to_str()
        .unwrap()
        .to_string()
}

/// The frames' link hand-off, as a plain browser (`--server`) requests it:
/// the desktop window's navigation handler takes it first, so the server
/// only answers `204 No Content` (the frame stays put) and opens nothing.
/// Only the exact path; a near miss is MBR's, behind its gate.
#[tokio::test(flavor = "multi_thread")]
async fn the_link_handoff_is_an_empty_204_in_a_plain_browser() {
    let app = app();
    let handoff = format!(
        "{}?url=mailto%3Aap%40example.com%3Fsubject%3DInvoice%2520108",
        ledgeline_shell::security::HANDOFF_PATH
    );
    let (status, headers, body) =
        send(&app.router, get(&handoff).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty(), "{body}");
    assert!(headers.get(header::LOCATION).is_none(), "no redirect");

    let (status, _, _) = send(
        &app.router,
        get("/.ledgeline/open/x?url=mailto%3Aa%40b.c")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_ne!(status, StatusCode::NO_CONTENT, "only the exact path");
}

/// Patrick's blank starter preview (2026-11-23): the SPA showed a preview
/// as `srcdoc`, which inherits the SPA's CSP, whose `frame-ancestors 'none'`
/// WebKit applies to it. A preview is now a page of its own, framed by URL
/// like MBR's page for the invoice: behind the same gate, under MBR's CSP.
#[tokio::test(flavor = "multi_thread")]
async fn a_preview_is_a_gated_page_under_mbrs_policy() {
    let app = app();
    // Why not srcdoc: the SPA's own policy refuses every frame ancestor.
    let (_, headers, _) = send(
        &app.router,
        get("/app/invoices/108").body(Body::empty()).unwrap(),
    )
    .await;
    assert!(csp_of(&headers).contains("frame-ancestors 'none'"));

    let url = "/.ledgeline-invoicer/preview?number=108&template=%2Finvoices%2Ftpl%2Fclassic.html";
    let (status, _, _) = send(&app.router, get(url).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no cookie, no page");

    let cookie = session_cookie(&app).await;
    let root = books_root(&app);
    let before = snapshot(&root);
    for (template, marker) in [
        ("%2Finvoices%2Ftpl%2Fclassic.html", "No. 108"),
        ("%2Finvoices%2Ftpl%2Fmodern.html", "#108"),
    ] {
        let request = get(&format!(
            "/.ledgeline-invoicer/preview?number=108&template={template}"
        ))
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
        let (status, headers, page) = send(&app.router, request).await;
        assert_eq!(status, StatusCode::OK, "{template}: {page}");
        assert_eq!(csp_of(&headers), super::mbr::SANDBOXED_CSP, "{template}");
        assert!(super::mbr::SANDBOXED_CSP.contains("frame-ancestors 'self'"));
        let violations = mbr_core::csp::strict_script_violations(&page, &[]);
        assert!(violations.is_empty(), "{template}: {violations:#?}");
        assert!(page.contains(marker), "{template}: {page}");
        assert!(page.contains("Platform subscription, 1 year"), "{template}");
        assert!(
            page.contains("<base href=\"/invoices/2026-11-17%20Customer%20X%20Invoice%20108/\">"),
            "{template}: relative links resolve from the note's page"
        );
        assert!(headers.get("server-timing").is_some());
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            ledgeline_shell::security::no_store()
        );
    }
    // The fixture's own template, by path, with the bearer token.
    let mut request = authed(
        "/.ledgeline-invoicer/preview?path=invoices%2F2026-11-10%20Customer%20X%20Invoice%20105.md",
    );
    request.headers_mut().remove(header::COOKIE);
    let (status, _, page) = send(&app.router, request).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Bill to Customer X"), "{page}");

    // A failure is a page saying why, under the same policy.
    let request = get("/.ledgeline-invoicer/preview?number=108&template=%2Fnope.html")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let (status, headers, page) = send(&app.router, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{page}");
    assert_eq!(csp_of(&headers), super::mbr::SANDBOXED_CSP);
    assert!(page.contains("Couldn't preview it"), "{page}");
    let request = get("/.ledgeline-invoicer/preview?number=108&source=x")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app.router, request).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unsaved text is POST /api/preview's"
    );

    assert_eq!(snapshot(&root), before, "a preview writes nothing");
}

/// The next server-sent event's text from `body`, or `None` when nothing
/// came within `wait`.
pub(super) async fn next_event(body: &mut Body, wait: std::time::Duration) -> Option<String> {
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let frame = tokio::time::timeout_at(deadline, body.frame())
            .await
            .ok()??;
        let frame = frame.expect("a frame");
        if let Some(data) = frame.data_ref() {
            text.push_str(&String::from_utf8_lossy(data));
        }
        if text.contains("\n\n") {
            return Some(text);
        }
    }
}

/// Open `GET /api/events` and read its `ready` event.
pub(super) async fn events(app: &App) -> Body {
    let response = app
        .router
        .clone()
        .oneshot(authed("/api/events"))
        .await
        .expect("infallible");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    let mut body = response.into_body();
    let ready = next_event(&mut body, std::time::Duration::from_secs(5))
        .await
        .expect("ready");
    assert!(ready.contains("event: ready"), "{ready}");
    body
}

/// Patrick's "live reload doesn't work" (2026-11-23): nothing told the SPA
/// that a file changed. Every settled change, once applied, is an event on
/// `GET /api/events` naming the files (by the books folder, never a full
/// path); a PDF is no event; the API's answers already show the change.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_on_disk_is_an_event_after_the_books_are_reread() {
    let app = app();
    let anonymous = get("/api/events").body(Body::empty()).unwrap();
    let (status, _, _) = send(&app.router, anonymous).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let mut body = events(&app).await;
    let root = books_root(&app);
    let file = root.join(INVOICE_105);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace("Senior architect hours", "Principal architect hours"),
    )
    .unwrap();
    // A PDF beside it is no event.
    let pdf = root.join("invoices/pdf/x.pdf");
    std::fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    std::fs::write(&pdf, "%PDF-").unwrap();
    app.books.apply_changes(std::slice::from_ref(&pdf));
    app.books.apply_changes(&[file.clone(), pdf]);

    let event = next_event(&mut body, std::time::Duration::from_secs(5))
        .await
        .expect("an event");
    assert!(event.contains("event: changed"), "{event}");
    let data = event
        .lines()
        .find_map(|l| l.strip_prefix("data: "))
        .expect("data");
    let changed: serde_json::Value = serde_json::from_str(data).unwrap();
    assert_eq!(changed, serde_json::json!({"paths": [INVOICE_105]}));
    assert!(!event.contains(root.to_str().unwrap()), "no full path");

    // What the SPA asks next already shows the edit.
    let (_, detail) = json_of(&app, authed("/api/invoices/105")).await;
    assert!(
        detail.to_string().contains("Principal architect hours"),
        "{detail}"
    );

    // A journal change too.
    let journal = root.join("books.journal");
    let text = std::fs::read_to_string(&journal).unwrap();
    std::fs::write(
        &journal,
        format!("{text}\n2026-11-20 Coffee\n    expenses:office  $4\n    assets:checking\n"),
    )
    .unwrap();
    app.books.apply_changes(std::slice::from_ref(&journal));
    let event = next_event(&mut body, std::time::Duration::from_secs(5))
        .await
        .expect("an event");
    assert!(event.contains(r#"{"paths":["books.journal"]}"#), "{event}");
}

/// The watcher end to end: an edit on disk reaches `GET /api/events` within
/// a couple of seconds, with no call from the test (real file-system events).
#[tokio::test(flavor = "multi_thread")]
async fn the_watcher_sends_an_event_for_an_edit() {
    let app = app();
    let _watcher = super::watch::spawn(Arc::clone(&app.books)).expect("watches");
    let mut body = events(&app).await;
    // Let the watcher settle (macOS reports the copy's own writes late).
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let file = books_root(&app).join(INVOICE_108);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace("Platform subscription", "Platform licence"),
    )
    .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = next_event(&mut body, left)
            .await
            .expect("an event for the edit within 10 s");
        if event.contains(INVOICE_108) {
            break;
        }
    }
    let (_, detail) = json_of(&app, authed("/api/invoices/108")).await;
    assert!(detail.to_string().contains("Platform licence"), "{detail}");
}

// ---------------------------------------------------------------------------
// Phase 3 feedback, second pass: templates that stick, totals that follow
// the rows (PLAN F9–F10).
// ---------------------------------------------------------------------------

const PAGE_108: &str = "/invoices/2026-11-17%20Customer%20X%20Invoice%20108/";
const MODERN: &str = "/invoices/tpl/modern.html";

fn starter_text(file: &str) -> &'static str {
    crate::starters::STARTERS
        .iter()
        .find(|s| s.file == file)
        .expect("a starter")
        .source
}

/// Patrick's "editing `template:` to the Modern starter failed": the
/// starters are copied in only on first use (B1), so the file wasn't in the
/// books, and MBR's page for the invoice was its 500 "Page template error:
/// template `/invoices/tpl/modern.html` was not found" (and `show` warned
/// `unknown-template`). A hand edit naming a starter now copies it in once
/// the watcher sees it, so MBR's page renders through it.
#[tokio::test(flavor = "multi_thread")]
async fn a_hand_edited_template_naming_a_starter_copies_it_in() {
    let app = app();
    let root = books_root(&app);
    let file = root.join(INVOICE_108);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace("/invoices/tpl/myco-default.html", MODERN),
    )
    .unwrap();

    // What Patrick saw, before the watcher acts.
    let (status, _, page) = send(&app.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(page.contains("Page template error"), "{page}");
    assert!(page.contains("was not found"), "{page}");

    app.books.apply_changes(std::slice::from_ref(&file));
    let modern = root.join("invoices/tpl/modern.html");
    assert_eq!(
        std::fs::read_to_string(&modern).unwrap(),
        starter_text("modern.html")
    );
    let (status, _, page) = send(&app.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("#108"), "Modern's number mark: {page}");
    let (_, detail) = json_of(&app, authed("/api/invoices/108")).await;
    assert!(!detail.to_string().contains("unknown-template"), "{detail}");

    // The user's copy is theirs: never replaced, even when named again.
    std::fs::write(&modern, "{{ frontmatter.invoice_num }} mine").unwrap();
    app.books.apply_changes(&[file.clone(), modern.clone()]);
    assert_eq!(
        std::fs::read_to_string(&modern).unwrap(),
        "{{ frontmatter.invoice_num }} mine"
    );
}

/// Opening books whose invoice already names a starter not copied in (an
/// edit made while the app was closed) copies it in too.
#[tokio::test(flavor = "multi_thread")]
async fn opening_the_books_copies_in_a_starter_an_invoice_names() {
    let (dir, journal) = fixture();
    let file = dir.path().join("books").join(INVOICE_108);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace(
            "/invoices/tpl/myco-default.html",
            "/invoices/tpl/classic.html",
        ),
    )
    .unwrap();
    let books = OpenBooks::open(&journal, false).expect("opens");
    let copied = books.install_starters();
    assert_eq!(
        copied.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        ["invoices/tpl/classic.html"]
    );
    assert!(books.install_starters().is_empty(), "once");
}

/// "Use this template": the picker writes the invoice's `template:` (the
/// core's `set`) and copies a starter in on first use.
#[tokio::test(flavor = "multi_thread")]
async fn using_a_template_writes_it_and_copies_a_starter_in() {
    let (app, _native) = mocked();
    let root = books_root(&app);
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/108/template",
            &serde_json::json!({"template": MODERN}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["template"], MODERN);
    assert_eq!(body["changed"], true);
    assert_eq!(body["copied"], "invoices/tpl/modern.html");
    assert_eq!(body["status"], "draft");
    let text = std::fs::read_to_string(root.join(INVOICE_108)).unwrap();
    assert!(
        text.contains("template: /invoices/tpl/modern.html\n"),
        "{text}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("invoices/tpl/modern.html")).unwrap(),
        starter_text("modern.html")
    );
    // The page says which it uses now, and MBR renders it.
    let (_, detail) = json_of(&app, authed("/api/invoices/108")).await;
    assert_eq!(detail["template"], MODERN);
    let (status, _, page) = send(&app.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let (_, list) = json_of(&app, authed("/api/templates")).await;
    let modern = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"] == MODERN)
        .expect("listed");
    assert_eq!(modern["installed"], true);

    // Again: nothing to write, nothing copied.
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/108/template",
            &serde_json::json!({"template": "invoices/tpl/modern.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], false);
    assert_eq!(body["copied"], serde_json::Value::Null);

    // Only a template the books have, or a starter.
    for bad in ["/invoices/tpl/nope.html", "/books.journal", "/../x.html"] {
        let (status, body) = json_of(
            &app,
            post_json(
                "/api/invoices/108/template",
                &serde_json::json!({"template": bad}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
    }
    let (status, _) = json_of(
        &app,
        post_json(
            "/api/invoices/999/template",
            &serde_json::json!({"template": MODERN}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A sent invoice may change its styling; its PDF then differs from what
    // the customer has, and the Send panel says so (P4, P5).
    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/pdf", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let journal = std::fs::read_to_string(root.join("books.journal")).unwrap();
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/105/template",
            &serde_json::json!({"template": "/invoices/tpl/classic.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "sent");
    assert_eq!(body["copied"], "invoices/tpl/classic.html");
    let (_, delivery) = json_of(&app, authed("/api/invoices/105/pdf")).await;
    assert_eq!(delivery["changed_since_sent"], true, "{delivery}");
    assert_eq!(
        std::fs::read_to_string(root.join("books.journal")).unwrap(),
        journal,
        "a template is styling: the books never see it"
    );
}

/// The draft 108 with its architect hours changed from 2 to 3, leaving the
/// amounts as they were (what an editor's save looks like).
fn edit_hours_108(root: &Path) -> PathBuf {
    let file = root.join(INVOICE_108);
    let text = std::fs::read_to_string(&file).unwrap();
    let edited = text.replace("| 2   | h    | $800", "| 3   | h    | $800");
    assert_ne!(edited, text, "the fixture's row");
    std::fs::write(&file, edited).unwrap();
    file
}

/// Patrick's "totals aren't recalculated after an edit": an outside edit to
/// a draft's rows now rewrites its amounts once it settles (`recalc-on-save
/// = "drafts"`, the default), and the rewrite coming back is no loop.
#[tokio::test(flavor = "multi_thread")]
async fn an_edited_draft_is_recalculated_once_the_edit_settles() {
    let app = app();
    let root = books_root(&app);
    let file = edit_hours_108(&root);
    app.books.apply_changes(std::slice::from_ref(&file));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("$2,400.00"), "{text}");
    assert!(text.contains("**$14,400.00**"), "{text}");
    let (_, detail) = json_of(&app, authed("/api/invoices/108")).await;
    assert_eq!(detail["table"]["discrepancies"], serde_json::json!([]));
    // Its own write, reported by the watcher, changes nothing more.
    app.books.apply_changes(std::slice::from_ref(&file));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

/// The real watcher: the edit, the settle, the rewrite, then quiet.
#[tokio::test(flavor = "multi_thread")]
async fn the_watcher_recalculates_an_edited_draft_and_stops() {
    let app = app();
    let _watcher = super::watch::spawn(Arc::clone(&app.books)).expect("watches");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let file = edit_hours_108(&books_root(&app));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let text = loop {
        let text = std::fs::read_to_string(&file).unwrap();
        if text.contains("**$14,400.00**") {
            break text;
        }
        if tokio::time::Instant::now() >= deadline {
            // Which of recalc_on_save's checks said no (CI diagnosis).
            let session = app.books.session();
            let index = session.index();
            let doc = index.docs().find(|d| d.number() == Some("108"));
            panic!(
                "not recalculated within 10 s\nrecalc-on-save: {:?}\nindexed: {:#?}\nfile:\n{text}",
                index.settings().recalc_on_save,
                doc.map(|d| (&d.stamp, &d.header.status, &d.summary)),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    // Well past another debounce: nothing rewrote it again.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

/// With `recalc-on-save = "never"`, the page shows the mismatch (stored vs
/// computed, `show`'s `table.discrepancies`) and Recalculate fixes it.
#[tokio::test(flavor = "multi_thread")]
async fn recalculate_fixes_a_draft_when_saving_leaves_it_alone() {
    let app = app();
    let root = books_root(&app);
    let settings = root.join("ledgeline.toml");
    let text = std::fs::read_to_string(&settings).unwrap();
    std::fs::write(
        &settings,
        text.replace("[invoicer]\n", "[invoicer]\nrecalc-on-save = \"never\"\n"),
    )
    .unwrap();
    app.books.apply_changes(std::slice::from_ref(&settings));
    let file = edit_hours_108(&root);
    app.books.apply_changes(std::slice::from_ref(&file));
    let (_, detail) = json_of(&app, authed("/api/invoices/108")).await;
    let found = detail["table"]["discrepancies"].as_array().unwrap().clone();
    assert_eq!(found.len(), 2, "{detail}");
    assert_eq!(found[0]["written"], "$1,600.00");
    assert_eq!(found[0]["expected"], "$2,400.00");

    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/108/recalc", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recalculated"]["written"], true);
    assert_eq!(body["rebooked"], serde_json::Value::Null);
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("**$14,400.00**")
    );
    let (_, detail) = json_of(&app, authed("/api/invoices/108")).await;
    assert_eq!(detail["table"]["discrepancies"], serde_json::json!([]));
}

/// A sent invoice is never recalculated on save: its amount is booked. The
/// page shows the mismatch; Recalculate asks first (`409` without
/// `rebook`), then recalculates and re-books it, as `recalc` + `rebook` do.
#[tokio::test(flavor = "multi_thread")]
async fn a_sent_invoice_is_recalculated_only_when_asked_and_rebooked() {
    let app = app();
    let root = books_root(&app);
    let file = root.join(INVOICE_105);
    let text = std::fs::read_to_string(&file).unwrap();
    let edited = text.replace("| 50  | $200 ", "| 40  | $200 ");
    assert_ne!(edited, text);
    std::fs::write(&file, &edited).unwrap();
    app.books.apply_changes(std::slice::from_ref(&file));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        edited,
        "left alone"
    );
    let (_, detail) = json_of(&app, authed("/api/invoices/105")).await;
    assert!(
        !detail["table"]["discrepancies"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{detail}"
    );

    let (status, body) = json_of(
        &app,
        post_json("/api/invoices/105/recalc", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("re-books"),
        "{body}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);

    let journal = std::fs::read_to_string(root.join("books.journal")).unwrap();
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/105/recalc",
            &serde_json::json!({"rebook": true, "today": "2026-11-20"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recalculated"]["written"], true);
    assert_eq!(body["rebooked"]["changed"], true, "{body}");
    assert_ne!(
        std::fs::read_to_string(root.join("books.journal")).unwrap(),
        journal
    );
    let (_, panel) = json_of(&app, authed("/api/invoices/105/books")).await;
    assert_eq!(panel["in_step"], true, "{panel}");
}

/// Re-booking is refused on or before `closed-through` (I8), so
/// Recalculate refuses a sent invoice there before writing anything.
#[tokio::test(flavor = "multi_thread")]
async fn recalculating_a_sent_invoice_in_a_closed_period_writes_nothing() {
    let app = app();
    let root = books_root(&app);
    let file = root.join("invoices/2026-09-15 Fabrikam Invoice 102.md");
    let text = std::fs::read_to_string(&file).unwrap();
    let edited = text.replace("| 6   | $200 ", "| 7   | $200 ");
    assert_ne!(edited, text);
    std::fs::write(&file, &edited).unwrap();
    app.books.apply_changes(std::slice::from_ref(&file));
    let journal = std::fs::read_to_string(root.join("books.journal")).unwrap();
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/invoices/102/recalc",
            &serde_json::json!({"rebook": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("closed-through"),
        "{body}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);
    assert_eq!(
        std::fs::read_to_string(root.join("books.journal")).unwrap(),
        journal
    );
    // A void invoice is history.
    let (status, _) = json_of(
        &app,
        post_json("/api/invoices/103/recalc", &serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------
// The shipped base (PLAN Q1, option B): the starters and the fixture's
// template extend `_invoice_base.v1.html`.
// ---------------------------------------------------------------------------

const BASE_FILE: &str = "invoices/tpl/_invoice_base.v1.html";

/// Q1's first unknown: MBR reads a template's siblings from disk only, so a
/// starter not copied in, in books without the base either (books from
/// before the base: whole-page templates only), can't find what it
/// extends. The preview folds it into the shipped base, which gives the
/// page the real `extends` gives once the base is there.
#[tokio::test(flavor = "multi_thread")]
async fn a_starter_previews_in_books_without_the_base() {
    let app = app();
    let root = books_root(&app);
    std::fs::remove_file(root.join(BASE_FILE)).unwrap();
    // A child without its base beside it would break every template in the
    // folder (Tera checks the inheritance of all the siblings MBR
    // registers), so these books' own template is a whole page, as before.
    std::fs::write(
        root.join("invoices/tpl/myco-default.html"),
        "<p>Invoice {{ frontmatter.invoice_num }}</p>{{ markdown | safe }}",
    )
    .unwrap();
    let before = snapshot(&root);
    let mut folded = Vec::new();
    for template in ["/invoices/tpl/classic.html", "/invoices/tpl/modern.html"] {
        for number in ["104", "105", "108"] {
            let (status, page, _) = preview(
                &app,
                serde_json::json!({"number": number, "template": template}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{template} {number}: {page}");
            if number == "105" {
                assert_bill_table_105(&page);
            }
            folded.push(page);
        }
    }
    // A note naming a starter that isn't there (unsaved text) folds too.
    let source = std::fs::read_to_string(root.join(INVOICE_105))
        .unwrap()
        .replace(
            "/invoices/tpl/myco-default.html",
            "/invoices/tpl/classic.html",
        );
    let (status, page, _) = preview(
        &app,
        serde_json::json!({"path": INVOICE_105, "source": source}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("class=\"letterhead\""), "{page}");
    assert_eq!(snapshot(&root), before, "a preview writes nothing");

    // The same pages through the real `extends`, the base on disk.
    std::fs::write(root.join(BASE_FILE), crate::starters::BASE.source).unwrap();
    let mut extended = Vec::new();
    for template in ["/invoices/tpl/classic.html", "/invoices/tpl/modern.html"] {
        for number in ["104", "105", "108"] {
            let (status, page, _) = preview(
                &app,
                serde_json::json!({"number": number, "template": template}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{page}");
            extended.push(page);
        }
    }
    assert_eq!(folded, extended);

    // The user's base, when the books have one, is what a starter extends.
    std::fs::write(
        root.join(BASE_FILE),
        crate::starters::BASE.source.replace(
            "<main class=\"invoice-main\">",
            "<main class=\"invoice-main\" data-mine>",
        ),
    )
    .unwrap();
    let (_, page, _) = preview(
        &app,
        serde_json::json!({"number": "105", "template": "/invoices/tpl/modern.html"}),
    )
    .await;
    assert!(page.contains("data-mine"), "{page}");
}

/// Copying a starter in brings the base it extends: Use this template, a
/// hand edit (the watcher) and opening the books. Never over a base that
/// is there.
#[tokio::test(flavor = "multi_thread")]
async fn copying_a_starter_in_brings_the_base() {
    // Use this template.
    let (sending, _native) = mocked();
    let root = books_root(&sending);
    std::fs::remove_file(root.join(BASE_FILE)).unwrap();
    let (status, body) = json_of(
        &sending,
        post_json(
            "/api/invoices/108/template",
            &serde_json::json!({"template": MODERN}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["copied"], "invoices/tpl/modern.html");
    assert_eq!(
        std::fs::read_to_string(root.join(BASE_FILE)).unwrap(),
        crate::starters::BASE.source
    );
    let (status, _, page) = send(&sending.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("#108"), "{page}");
    // The base isn't a template: not listed, and nothing warns of it.
    // (Templates say they extend it, since 4d; none is it.)
    let (_, list) = json_of(&sending, authed("/api/templates")).await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|t| !t["path"].as_str().unwrap().contains("_invoice_base")),
        "{list}"
    );
    let (_, detail) = json_of(&sending, authed("/api/invoices/108")).await;
    assert!(!detail.to_string().contains("unknown-template"), "{detail}");

    // A hand edit, seen by the watcher.
    let app = app();
    let root = books_root(&app);
    std::fs::remove_file(root.join(BASE_FILE)).unwrap();
    let file = root.join(INVOICE_108);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace("/invoices/tpl/myco-default.html", MODERN),
    )
    .unwrap();
    app.books.apply_changes(std::slice::from_ref(&file));
    assert_eq!(
        std::fs::read_to_string(root.join(BASE_FILE)).unwrap(),
        crate::starters::BASE.source
    );
    let (status, _, page) = send(&app.router, authed(PAGE_108)).await;
    assert_eq!(status, StatusCode::OK, "{page}");

    // Opening the books.
    let (dir, journal) = fixture();
    let books_dir = dir.path().join("books");
    std::fs::remove_file(books_dir.join(BASE_FILE)).unwrap();
    let file = books_dir.join(INVOICE_108);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace(
            "/invoices/tpl/myco-default.html",
            "/invoices/tpl/classic.html",
        ),
    )
    .unwrap();
    let books = OpenBooks::open(&journal, false).expect("opens");
    let copied = books.install_starters();
    assert_eq!(
        copied.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        [BASE_FILE, "invoices/tpl/classic.html"]
    );
}

/// A whole-page template (no base), as every template was before Q1,
/// keeps working: listed, previewed, and served by MBR.
#[tokio::test(flavor = "multi_thread")]
async fn a_whole_page_template_keeps_working() {
    let app = app();
    let root = books_root(&app);
    let whole = "{# invoicer: name=\"Whole page\" #}\n<!doctype html>\n<html><head><title>{{ frontmatter.title }}</title></head>\n\
                 <body><h1 class=\"whole\">Invoice {{ frontmatter.invoice_num }}</h1>{{ markdown | safe }}</body></html>\n";
    std::fs::write(root.join("invoices/tpl/whole.html"), whole).unwrap();
    app.books
        .apply_changes(&[root.join("invoices/tpl/whole.html")]);
    let (_, list) = json_of(&app, authed("/api/templates")).await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|t| t["path"] == "/invoices/tpl/whole.html" && t["name"] == "Whole page"),
        "{list}"
    );
    let (status, page, _) = preview(
        &app,
        serde_json::json!({"number": "105", "template": "/invoices/tpl/whole.html"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("<h1 class=\"whole\">Invoice 105</h1>"),
        "{page}"
    );
    assert_bill_table_105(&page.replace(
        "</h1>",
        "</h1>table:has(thead th:last-child &gt; em:only-child)",
    ));

    let file = root.join(INVOICE_105);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace(
            "/invoices/tpl/myco-default.html",
            "/invoices/tpl/whole.html",
        ),
    )
    .unwrap();
    let (status, _, page) = send(
        &app.router,
        authed("/invoices/2026-11-10%20Customer%20X%20Invoice%20105/"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page.contains("<h1 class=\"whole\">Invoice 105</h1>"),
        "{page}"
    );
    assert!(
        !page.contains("invoice-main"),
        "nothing of the base: {page}"
    );
}

/// A child that only restyles (`theme`) is a template though it never
/// names `invoice_num`, and gets the base's layout: both parties with
/// address, email and phone, the dates, the bill. `settings` can turn
/// MBR's chrome off.
#[tokio::test(flavor = "multi_thread")]
async fn a_theme_only_child_gets_the_bases_layout() {
    let app = app();
    let root = books_root(&app);
    let child = "{# invoicer: name=\"Mine\" #}\n{% extends \"_invoice_base.v1.html\" %}\n\
                 {% block theme %}{{ super() }} .sheet { font-family: Georgia, serif; }{% endblock theme %}\n";
    std::fs::write(root.join("invoices/tpl/mine.html"), child).unwrap();
    let bare = "{# invoicer: name=\"Bare\" #}\n{% extends \"_invoice_base.v1.html\" %}\n\
                {% block settings %}{% set chrome = false %}{% endblock settings %}\n";
    std::fs::write(root.join("invoices/tpl/bare.html"), bare).unwrap();
    app.books.apply_changes(&[
        root.join("invoices/tpl/mine.html"),
        root.join("invoices/tpl/bare.html"),
    ]);
    let (_, list) = json_of(&app, authed("/api/templates")).await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Mine") && names.contains(&"Bare"),
        "{names:?}"
    );

    let (status, page, _) = preview(
        &app,
        serde_json::json!({"number": "105", "template": "/invoices/tpl/mine.html"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_bill_table_105(&page);
    for text in [
        "Georgia, serif",
        "--desk: #eef0f2",
        "<h1>Invoice 105</h1>",
        "From My Company",
        "1234 Background Blvd.",
        "finance@mycompany.com",
        "+1 (555) 010-2000",
        "Bill to Customer X",
        "1234 Main St. #555",
        "accounts_receivable@customerx.com",
        "+1 (303) 555-0142",
        "<b>Terms</b> Net 30",
        "mbr-export-ready",
    ] {
        assert!(page.contains(text), "missing {text:?}");
    }

    // MBR's own page: chrome through `mine`, none through `bare`.
    let file = root.join(INVOICE_105);
    let text = std::fs::read_to_string(&file).unwrap();
    for (template, chrome) in [("mine", true), ("bare", false)] {
        std::fs::write(
            &file,
            text.replace(
                "/invoices/tpl/myco-default.html",
                &format!("/invoices/tpl/{template}.html"),
            ),
        )
        .unwrap();
        let (status, _, page) = send(
            &app.router,
            authed("/invoices/2026-11-10%20Customer%20X%20Invoice%20105/"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert!(page.contains("Bill to Customer X"), "{page}");
        assert_eq!(page.contains("<mbr-search>"), chrome, "{template}: {page}");
    }
}

/// The invoice editor's routes (4a), with these helpers.
#[path = "editor_tests.rs"]
mod editor_tests;

#[tokio::test(flavor = "multi_thread")]
async fn the_invoice_list_filters_and_sorts_by_its_url() {
    let app = app();
    let numbers = |query: &'static str| {
        let router = app.router.clone();
        async move {
            let url = format!("/api/invoices?today=2026-11-17&{query}");
            let (status, _, body) = send(&router, authed(&url)).await;
            assert_eq!(status, StatusCode::OK, "{query}: {body}");
            let list: serde_json::Value = serde_json::from_str(&body).expect("json");
            list.as_array()
                .expect("array")
                .iter()
                .filter_map(|d| d["number"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(numbers("status=paid").await, ["104", "102", "101"]);
    assert_eq!(numbers("status=overdue").await, ["107", "106"]);
    assert_eq!(
        numbers("status=open&sort=due&dir=asc").await,
        ["106", "107", "105"]
    );
    assert_eq!(numbers("aged=61-90").await, ["106"]);
    assert_eq!(
        numbers("from=2026-10-01&to=2026-10-31").await,
        ["107", "104", "103"]
    );
    assert_eq!(numbers("customer=Contoso&q=107").await, ["107"]);
    assert_eq!(
        numbers("q=py75334&sort=number&dir=desc").await,
        ["108", "105", "104"]
    );
    assert_eq!(
        numbers("sort=number").await,
        ["101", "102", "103", "104", "105", "106", "107", "108"]
    );
    // Blank values are no filter: the SPA passes its URL through.
    assert_eq!(
        numbers("status=&q=&from=&to=&aged=&sort=&dir=").await.len(),
        8
    );
    for bad in [
        "from=last-week",
        "to=2026-13",
        "aged=old",
        "sort=amount",
        "dir=up",
        "status=unpaid",
    ] {
        let (status, _, body) = send(&app.router, authed(&format!("/api/invoices?{bad}"))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
}

#[path = "templates_tests.rs"]
mod templates_tests;
