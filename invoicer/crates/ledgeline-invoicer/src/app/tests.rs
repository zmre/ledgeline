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

const TOKEN: &str = "router-test-token-0123456789";
const PORT: u16 = 5199;
const HOST: &str = "127.0.0.1:5199";

fn copy_tree(src: &Path, dest: &Path) {
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

struct App {
    _dir: tempfile::TempDir,
    books: Arc<OpenBooks>,
    router: Router,
}

/// Must run inside a tokio runtime (MBR spawns on it).
fn app() -> App {
    app_with(None)
}

/// [`app`] with a native side (the PDF tests' mock).
fn app_with(native: Option<Arc<dyn super::native::Native>>) -> App {
    app_on("accrual", native)
}

/// [`app_with`] over the fixture `name` (`accrual`, `cash`).
fn app_on(name: &str, native: Option<Arc<dyn super::native::Native>>) -> App {
    let (dir, journal) = fixture_named(name);
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

async fn send(
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

fn get(path: &str) -> axum::http::request::Builder {
    Request::get(path).header(header::HOST, HOST)
}

fn authed(path: &str) -> Request<Body> {
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
    let (status, _, _) = send(&app.router, authed("/api/invoices/999")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(&app.router, authed("/api/invoices?status=nope")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

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
    assert_eq!(csp, super::mbr::MBR_CSP);

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

    // A settings change re-opens the session and reconciles everything.
    let settings = root.join("ledgeline.toml");
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

fn post_json(path: &str, body: &serde_json::Value) -> Request<Body> {
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
    assert_bill_table_105(&page);
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
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        super::mbr::MBR_CSP
    );
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

fn books_root(app: &App) -> PathBuf {
    app.books.vault().root().to_path_buf()
}

async fn json_of(app: &App, request: Request<Body>) -> (StatusCode, serde_json::Value) {
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
        serde_json::json!({"pdf": false, "share": false, "copy_file": false, "open_url": false})
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
    assert_eq!(
        headers.get(header::LOCATION).unwrap(),
        "/invoices/2026-11-10%20Customer%20X%20Invoice%20105/"
    );
    let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    let pair = cookie.split(';').next().unwrap().to_string();
    let page = get("/invoices/2026-11-10%20Customer%20X%20Invoice%20105/")
        .header(header::COOKIE, pair)
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app.router, page).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Bill to Customer X"), "{body}");
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
    let (app, _native) = mocked();
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
        let (status, body) = json_of(&app, post_json("/api/pdf/share", &bad)).await;
        assert_eq!(status, want, "{bad}: {body}");
    }
    let _ = std::fs::remove_file(temp);
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
