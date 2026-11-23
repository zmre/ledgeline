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
    let dir = tempfile::tempdir().expect("tempdir");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/accrual");
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
    let (dir, journal) = fixture();
    let books = Arc::new(OpenBooks::open(&journal, false).expect("opens"));
    let app_dirs = AppDirs::under(dir.path().join("config"));
    let state = AppState::new(Some(Arc::clone(&books)), app_dirs);
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
