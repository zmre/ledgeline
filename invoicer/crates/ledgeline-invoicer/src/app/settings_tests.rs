//! The Settings page's API (`/api/settings`, settings plan §5) and the
//! shared payment-matching preferences (D2), over private copies of the
//! fixtures. The app's config folder is a temp dir: no real `ui-prefs.toml`
//! is read or written.
//!
//! ```sh
//! UPDATE_SNAPSHOTS=1 cargo test -p ledgeline-invoicer settings_tests   # re-record
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline_shell::AppDirs;
use ledgeline_shell::security::{AccessToken, Security};
use serde_json::{Value, json};

use super::books::OpenBooks;
use super::native::Native;
use super::native::mock::MockNative;
use super::tests::{
    App, HOST, TOKEN, app_on, app_on_prepared, authed, copy_tree, events, get, json_of, next_event,
    post_json, send,
};
use super::{AppState, Unopened, router};

const SETTINGS: &str = "ledgeline.toml";

/// The cash fixture's `ledgeline.toml`, as checked in.
const CASH_TOML: &str = include_str!("../../../../fixtures/cash/ledgeline.toml");

fn request(method: &str, path: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn put(body: &Value) -> Request<Body> {
    request("PUT", "/api/settings", body)
}

fn root(app: &App) -> PathBuf {
    app.books.vault().root().to_path_buf()
}

fn settings_text(app: &App) -> String {
    std::fs::read_to_string(root(app).join(SETTINGS)).expect("reads")
}

async fn page(app_router: &Router) -> Value {
    let (status, _, body) = send(app_router, authed("/api/settings")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str(&body).expect("json")
}

async fn revision(app: &App) -> Value {
    page(&app.router).await["revision"].clone()
}

/// Books a problem in their `ledgeline.toml` keeps shut, as the server
/// holds them: no books, and why.
struct Shut {
    _dir: tempfile::TempDir,
    router: Router,
    root: PathBuf,
}

fn shut(prepare: impl FnOnce(&Path), native: Option<Arc<dyn Native>>) -> Shut {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/cash");
    let root = dir.path().join("books");
    copy_tree(&src, &root);
    prepare(&root);
    let journal = root.join("books.journal");
    let failure = OpenBooks::open(&journal, false).expect_err("the books stay shut");
    let problem = failure.in_settings.expect("a problem in ledgeline.toml");
    let state = AppState::with_native(None, AppDirs::under(dir.path().join("config")), native);
    state.set_unopened(Unopened { journal, problem });
    let token = AccessToken::parse(TOKEN).expect("token");
    Shut {
        _dir: dir,
        router: router(state, Security::local(token.clone(), 5199), token, 5199),
        root,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_settings_routes_need_the_token() {
    let app = app_on("cash", None);
    for (method, path) in [
        ("GET", "/api/settings"),
        ("PUT", "/api/settings"),
        ("POST", "/api/settings/check"),
        ("POST", "/api/ui-prefs/pin"),
    ] {
        let anonymous = get(path).method(method).body(Body::empty()).unwrap();
        let (status, _, _) = send(&app.router, anonymous).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}

/// The page on the cash fixture: also the golden the SPA's tests can read.
#[tokio::test(flavor = "multi_thread")]
async fn the_page_reads_the_file_and_the_books() {
    let app = app_on("cash", None);
    let before = settings_text(&app);
    let page = page(&app.router).await;
    super::dashboard_tests::snapshot("cash-api-settings.json", &page);
    assert_eq!(page["file"], "present");
    assert_eq!(page["readOnly"], false);
    assert_eq!(page["problem"], Value::Null);
    assert_eq!(
        page["set"],
        json!([
            "company",
            "number-start",
            "revenue-account",
            "deposit-account"
        ])
    );
    assert_eq!(page["values"]["revenue-account"], "revenue:consulting");
    assert_eq!(page["values"]["default-terms"], Value::Null);
    assert_eq!(page["defaults"]["invoices-dir"], "invoices");
    assert_eq!(page["values"]["words"]["quantity"][0], "Qty");
    assert_eq!(page["books"]["basis"], "cash");
    assert_eq!(page["nextNumber"]["number"], "208");
    assert_eq!(page["booksError"], Value::Null);
    let templates = page["choices"]["templates"].as_array().expect("templates");
    assert!(
        templates
            .iter()
            .any(|t| t["value"] == "/invoices/tpl/myco-default.html" && t["starter"] == false),
        "{templates:?}"
    );
    assert!(
        templates
            .iter()
            .any(|t| t["value"] == "/invoices/tpl/classic.html" && t["starter"] == true),
        "{templates:?}"
    );
    assert!(
        page["choices"]["companies"]
            .as_array()
            .expect("companies")
            .iter()
            .any(|c| c["link"] == "[[My Company]]")
    );
    assert_eq!(page["coming"].as_array().map(Vec::len), Some(4));
    assert_eq!(page["affectsBookings"].as_array().map(Vec::len), Some(6));
    // Reading writes nothing.
    assert_eq!(settings_text(&app), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_file_is_created_only_when_asked() {
    let app = app_on_prepared("accrual", None, |books| {
        std::fs::remove_file(books.join(SETTINGS)).expect("removes");
    });
    let page = page(&app.router).await;
    assert_eq!(page["file"], "missing");
    assert_eq!(page["set"], json!([]));
    assert_eq!(page["readOnly"], false);
    let revision = page["revision"].clone();
    let change = json!({"revision": revision, "set": {"default-terms": 30}});
    let (status, refused) = json_of(&app, put(&change)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "no-file");
    assert!(!root(&app).join(SETTINGS).exists(), "nothing is created");
    let mut create = change.clone();
    create["createFile"] = json!(true);
    let (status, saved) = json_of(&app, put(&create)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["created"], true);
    let text = settings_text(&app);
    assert!(
        text.starts_with(invoicer_core::settings_edit::FILE_HEADER),
        "{text}"
    );
    assert!(text.ends_with("[invoicer]\ndefault-terms = 30\n"), "{text}");
    assert_eq!(app.books.session().settings().default_terms, Some(30));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unusable_file_is_read_only_and_never_written() {
    // A symbolic link: never followed, so `[books]` can't read it either.
    let link = shut(
        |books| {
            std::fs::rename(books.join(SETTINGS), books.join("real.toml")).expect("moves");
            std::os::unix::fs::symlink(books.join("real.toml"), books.join(SETTINGS))
                .expect("links");
        },
        None,
    );
    let page = page(&link.router).await;
    assert_eq!(page["file"], "unusable");
    assert_eq!(page["readOnly"], true);
    assert_eq!(page["revision"], Value::Null);
    assert!(
        page["problem"]
            .as_str()
            .is_some_and(|p| p.contains("symbolic link")),
        "{page}"
    );
    assert!(page["booksError"].is_string(), "{page}");
    // Not TOML.
    let garbled = shut(
        |books| std::fs::write(books.join(SETTINGS), "not = = toml").expect("writes"),
        None,
    );
    let page = self::page(&garbled.router).await;
    assert_eq!(page["file"], "unusable");
    assert!(
        page["problem"]
            .as_str()
            .is_some_and(|p| p.contains("not valid TOML")),
        "{page}"
    );
    assert_eq!(
        std::fs::read_to_string(garbled.root.join(SETTINGS)).expect("reads"),
        "not = = toml"
    );
}

/// Plan risk 2: a hand edit that leaves `[invoicer]` invalid keeps the books
/// shut, but the page still loads and says why, in plain words, and the
/// file can be opened from it.
#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_table_keeps_the_books_shut_but_not_the_page() {
    let native = Arc::new(MockNative::everything());
    let app = shut(
        |books| {
            std::fs::write(
                books.join(SETTINGS),
                CASH_TOML.replace(
                    "number-start = 201",
                    "number-start = 201\nfilename = \"{date}.md\"",
                ),
            )
            .expect("writes");
        },
        Some(Arc::clone(&native) as Arc<dyn Native>),
    );
    let page = page(&app.router).await;
    assert_eq!(page["file"], "present");
    assert_eq!(page["readOnly"], true);
    assert_eq!(page["problems"][0]["key"], "filename");
    for text in [
        page["problems"][0]["message"].as_str().expect("message"),
        page["booksError"].as_str().expect("why the books are shut"),
        page["problem"].as_str().expect("why it's read-only"),
    ] {
        assert!(!text.contains("filename") && !text.contains('{'), "{text}");
    }
    // Every other route says why there are no books.
    let (status, _, body) = send(&app.router, authed("/api/books")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let body: Value = serde_json::from_str(&body).expect("json");
    assert!(body["problem"].is_string(), "{body}");
    // Save needs the books.
    let (status, _, _) = send(
        &app.router,
        put(&json!({"revision": page["revision"], "unset": ["filename"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // "Open the settings file" works without the books (shown in Finder:
    // a .toml isn't a document), and nothing else does.
    let (status, _, body) = send(
        &app.router,
        post_json("/api/files/open", &json!({"path": "ledgeline.toml"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        native.revealed_files.lock().unwrap().as_slice(),
        [app.root.canonicalize().expect("root").join(SETTINGS)]
    );
    let (status, _, _) = send(
        &app.router,
        post_json("/api/files/open", &json!({"path": "books.journal"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
async fn save_writes_only_what_changed_and_the_session_follows() {
    let app = app_on("cash", None);
    let first = revision(&app).await;
    let (status, saved) = json_of(
        &app,
        put(&json!({
            "revision": first,
            "set": {"default-terms": 21, "words.tax": ["VAT"]},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        settings_text(&app),
        CASH_TOML.replace(
            "number-start = 201\n",
            "number-start = 201\ndefault-terms = 21\n\n[invoicer.words]\ntax = [\"VAT\"]\n"
        ),
        "comments, order and [books] are kept"
    );
    assert_ne!(saved["revision"], first);
    assert_eq!(saved["values"]["default-terms"], 21);
    assert_eq!(saved["created"], false);
    assert!(
        saved["set"]
            .as_array()
            .expect("set")
            .contains(&json!("words.tax"))
    );
    // The session has the new settings: the editor offers the default, and a
    // new invoice for a customer without terms gets them.
    assert_eq!(app.books.session().settings().default_terms, Some(21));
    let (_, options) = json_of(&app, authed("/api/editor/options")).await;
    assert_eq!(options["default_terms"], 21);
    // A stale revision is refused, with the current one.
    let (status, stale) = json_of(
        &app,
        put(&json!({"revision": first, "set": {"dso-days": 30}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "changed");
    assert_eq!(stale["revision"], saved["revision"]);
    // "Use the default" takes the key away again.
    let (status, back) = json_of(
        &app,
        put(&json!({"revision": saved["revision"], "unset": ["default-terms", "words.tax"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{back}");
    assert_eq!(settings_text(&app), CASH_TOML);
    assert_eq!(back["revision"], first);
    // A change to another table is no conflict.
    let text = settings_text(&app);
    std::fs::write(
        root(&app).join(SETTINGS),
        format!("{text}\n[ui]\ntheme = \"nord\"\n"),
    )
    .expect("writes");
    let (status, body) = json_of(
        &app,
        put(&json!({"revision": first, "set": {"dso-days": 30}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(settings_text(&app).ends_with("[ui]\ntheme = \"nord\"\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn booking_changes_are_confirmed_first() {
    let app = app_on("cash", None);
    let before = settings_text(&app);
    let change = json!({
        "revision": revision(&app).await,
        "set": {"payment-allocation": "pro-rata"},
    });
    let (status, checked) = json_of(&app, request("POST", "/api/settings/check", &change)).await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    assert_eq!(checked["effects"][0]["key"], "payment-allocation");
    assert_eq!(checked["problems"], json!([]));
    assert_eq!(checked["refused"], Value::Null);
    let (status, ask) = json_of(&app, put(&change)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{ask}");
    assert_eq!(ask["code"], "confirm");
    assert_eq!(ask["effects"], checked["effects"]);
    assert_eq!(settings_text(&app), before, "nothing written");
    let mut confirmed = change.clone();
    confirmed["confirmed"] = json!(["payment-allocation"]);
    let (status, saved) = json_of(&app, put(&confirmed)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        app.books.session().settings().payment_allocation,
        invoicer_core::settings::PaymentAllocation::ProRata
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_values_are_refused_by_setting() {
    let app = app_on("cash", None);
    let before = settings_text(&app);
    let rev = revision(&app).await;
    let change = json!({"revision": rev, "set": {"dso-days": 0, "filename": "{date}.md"}});
    let (status, checked) = json_of(&app, request("POST", "/api/settings/check", &change)).await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    let mut keys: Vec<&str> = checked["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .filter_map(|p| p["key"].as_str())
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["dso-days", "filename"]);
    let (status, invalid) = json_of(&app, put(&change)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{invalid}");
    assert_eq!(invalid["code"], "invalid");
    assert_eq!(invalid["problems"], checked["problems"]);
    for bad in [
        json!({"revision": rev, "set": {"invoice-dir": "x"}}),
        json!({"revision": rev, "set": {"company": null}}),
        json!({"revision": rev, "sets": {}}),
    ] {
        let (status, body) = json_of(&app, put(&bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        assert_eq!(body["code"], "bad-request");
    }
    assert_eq!(settings_text(&app), before);
}

/// D5: in cash books, 206's deposit wire went to revenue and 207, which
/// applies it, is unpaid: changing how deposit payments book is refused,
/// listing the deposit, whatever is confirmed.
#[tokio::test(flavor = "multi_thread")]
async fn deposit_payments_are_refused_while_a_deposit_waits() {
    let app = app_on("cash", None);
    let before = settings_text(&app);
    let change = json!({
        "revision": revision(&app).await,
        "set": {"cash-deposits": "liability-until-final"},
        "confirmed": ["cash-deposits"],
    });
    let (status, checked) = json_of(&app, request("POST", "/api/settings/check", &change)).await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    assert_eq!(checked["refused"]["deposits"][0]["number"], "206");
    let (status, refused) = json_of(&app, put(&change)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "refused");
    assert_eq!(refused["refused"]["key"], "cash-deposits");
    assert_eq!(
        refused["refused"]["deposits"][0]["bookedTo"],
        json!(["revenue:consulting"])
    );
    assert_eq!(settings_text(&app), before);
}

/// An outside edit reloads the books and reaches the SPA; our own Save's
/// echo, and a change to another table, don't reconcile again; an invalid
/// edit keeps the books as they were and the page shows the problem.
#[tokio::test(flavor = "multi_thread")]
async fn outside_edits_reload_and_echoes_do_not() {
    let app = app_on("cash", None);
    let file = root(&app).join(SETTINGS);
    let mut stream = events(&app).await;
    let wait = std::time::Duration::from_secs(5);

    let (status, _) = json_of(
        &app,
        put(&json!({"revision": revision(&app).await, "set": {"dso-days": 45}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        app.books
            .apply_changes(std::slice::from_ref(&file))
            .is_none(),
        "the echo of our own write is no change"
    );
    let event = next_event(&mut stream, wait).await.expect("an event");
    assert!(event.contains(r#"{"paths":["ledgeline.toml"]}"#), "{event}");

    let text = settings_text(&app);
    std::fs::write(&file, text.replace("dso-days = 45", "dso-days = 60")).expect("writes");
    assert!(
        app.books
            .apply_changes(std::slice::from_ref(&file))
            .is_some(),
        "an outside edit reconciles"
    );
    assert_eq!(app.books.session().settings().dso_days, 60);
    assert!(next_event(&mut stream, wait).await.is_some());

    let text = settings_text(&app);
    std::fs::write(&file, format!("{text}\n[ui]\ntheme = \"nord\"\n")).expect("writes");
    assert!(
        app.books
            .apply_changes(std::slice::from_ref(&file))
            .is_none()
    );
    assert!(next_event(&mut stream, wait).await.is_some());

    let text = settings_text(&app);
    std::fs::write(&file, text.replace("dso-days = 60", "dso-days = 0")).expect("writes");
    assert!(
        app.books
            .apply_changes(std::slice::from_ref(&file))
            .is_none()
    );
    assert_eq!(app.books.session().settings().dso_days, 60, "kept");
    let event = next_event(&mut stream, wait).await.expect("an event");
    assert!(event.contains("ledgeline.toml"), "{event}");
    let page = page(&app.router).await;
    assert_eq!(page["readOnly"], true);
    assert_eq!(page["problems"][0]["key"], "dso-days");
}

/// Settings that change what the index holds (RA-3's caches) take effect
/// at once: the session is opened afresh.
#[tokio::test(flavor = "multi_thread")]
async fn folders_and_the_catalog_take_effect_at_once() {
    let app = app_on("cash", None);
    let (status, saved) = json_of(
        &app,
        put(&json!({
            "revision": revision(&app).await,
            "set": {"templates-dir": "billing/tpl", "catalog": "billing/items.md"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, catalog) = json_of(&app, authed("/api/catalog")).await;
    assert_eq!(catalog["path"], "billing/items.md");
    let (_, templates) = json_of(&app, authed("/api/templates")).await;
    let text = templates.to_string();
    assert!(text.contains("/billing/tpl/classic.html"), "{text}");
    let page = page(&app.router).await;
    assert!(
        page["choices"]["templates"]
            .as_array()
            .expect("templates")
            .iter()
            .any(|t| t["value"] == "/billing/tpl/modern.html" && t["starter"] == true),
        "{page}"
    );
}

/// D2: the matcher's keys are shared with Ledgeline and pinnable here, and
/// Apply payment uses the effective value. 203 ($1,200, Wingtip) is paid by
/// an ACH dated 2026-11-16; 200 days later it is outside the default 180.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_lookback_is_what_apply_payment_uses() {
    let app = app_on_prepared("cash", None, |books| {
        let journal = books.join("books.journal");
        let text = std::fs::read_to_string(&journal).expect("reads");
        std::fs::write(
            &journal,
            format!(
                "{text}\n2026-11-16 * ACH WINGTIP PAYMENTS  ; id: ach-41\n    \
                 assets:bank:checking  $1,200.00\n    income:unknown\n"
            ),
        )
        .expect("writes");
    });
    let payments = "/api/invoices/203/payments?today=2027-06-04";
    let found = |body: &Value| {
        body["candidates"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c["description"] == "ACH WINGTIP PAYMENTS"))
    };
    let (status, body) = json_of(&app, authed(payments)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!found(&body), "{body}");

    let pin = |pinned: bool| {
        post_json(
            "/api/ui-prefs/pin",
            &json!({"key": "settlementLookbackDays", "pinned": pinned}),
        )
    };
    let (status, prefs) = json_of(&app, pin(true)).await;
    assert_eq!(status, StatusCode::OK, "{prefs}");
    assert!(
        settings_text(&app).contains("[ui]\nsettlementLookbackDays = 180\n"),
        "{}",
        settings_text(&app)
    );
    // Pinned, a change goes to these books' file, not the shared one.
    let (status, prefs) = json_of(
        &app,
        request(
            "PATCH",
            "/api/ui-prefs",
            &json!({"settlementLookbackDays": 365}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{prefs}");
    assert_eq!(prefs["prefs"]["settlementLookbackDays"], 365);
    assert!(settings_text(&app).contains("settlementLookbackDays = 365"));
    let (_, body) = json_of(&app, authed(payments)).await;
    assert!(found(&body), "{body}");
    // The pin wrote `[ui]` only: no reconcile, and Save still applies.
    let file = root(&app).join(SETTINGS);
    assert!(
        app.books
            .apply_changes(std::slice::from_ref(&file))
            .is_none()
    );

    let (status, _) = json_of(&app, pin(false)).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = json_of(&app, authed(payments)).await;
    assert!(!found(&body), "unpinned: the shared 180 again");
    // Only the shared keys pin.
    let (status, _) = json_of(
        &app,
        post_json(
            "/api/ui-prefs/pin",
            &json!({"key": "autoPrices", "pinned": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
