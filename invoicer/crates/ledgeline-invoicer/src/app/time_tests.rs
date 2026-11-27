//! Time tracking routes (plan 46), in process over a copy of
//! `invoicer/fixtures/time` (clock 2026-11-26), and setup over the accrual
//! fixture, which has none.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};

use super::tests::{App, HOST, TOKEN, app_on, authed, json_of, post_json};

const NOW: &str = "2026-11-26T14:05:12";

fn time_app() -> App {
    app_on("time", None)
}

fn with_body(method: &str, path: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn delete(path: &str) -> Request<Body> {
    Request::delete(path)
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("request")
}

fn root(app: &App) -> std::path::PathBuf {
    app.books.vault().root().to_path_buf()
}

fn read(app: &App, relative: &str) -> String {
    std::fs::read_to_string(root(app).join(relative)).expect("reads")
}

async fn ok(app: &App, request: Request<Body>) -> Value {
    let (status, body) = json_of(app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn the_routes_need_the_token() {
    let app = time_app();
    let request = Request::get("/api/time/state")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .expect("request");
    let (status, _) = json_of(&app, request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_on_books_with_no_time() {
    let app = app_on("accrual", None);
    let state = ok(&app, authed(&format!("/api/time/state?now={NOW}"))).await;
    assert_eq!(state["configured"], false);
    assert_eq!(state["setup"]["file"], "time.journal");
    let (status, body) = json_of(&app, authed("/api/time/entries")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "not-set-up");
    let dry = ok(
        &app,
        post_json(
            "/api/time/setup",
            &json!({"person": "pat", "dry_run": true, "today": "2026-11-26"}),
        ),
    )
    .await;
    assert_eq!(dry["written"], false);
    let paths: Vec<&str> = dry["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|f| f["path"].as_str().expect("path"))
        .collect();
    assert_eq!(
        paths,
        [
            "time.journal",
            "time-accounts.journal",
            "time/pat/2026-11.timedot"
        ]
    );
    assert!(!root(&app).join("time.journal").exists());
    let done = ok(
        &app,
        post_json(
            "/api/time/setup",
            &json!({"person": "pat", "today": "2026-11-26"}),
        ),
    )
    .await;
    assert_eq!(done["written"], true);
    assert!(read(&app, "time-accounts.journal").contains("account clients:contoso"));
    let state = ok(&app, authed(&format!("/api/time/state?now={NOW}"))).await;
    assert_eq!(state["configured"], true);
    assert_eq!(state["person"], "pat", "saved to ui-prefs");
    let (status, again) = json_of(
        &app,
        post_json("/api/time/setup", &json!({"today": "2026-11-26"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");
    assert_eq!(again["code"], "exists");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tracker_toggles_adjusts_notes_and_stops() {
    let app = time_app();
    // The person is the tracker's (P-person): save it first.
    let (status, _, body) = super::tests::send(
        &app.router,
        with_body(
            "PATCH",
            "/api/ui-prefs",
            &json!({"time": {"person": "pat"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let state = ok(&app, authed(&format!("/api/time/state?now={NOW}"))).await;
    assert_eq!(state["configured"], true);
    assert_eq!(state["slots"].as_array().map(Vec::len), Some(10));
    assert_eq!(state["slots"][1]["account"], "clients:acme:advisory");
    assert_eq!(state["today_total"]["text"], "1h 15m");
    // Slot 2 starts.
    let started = ok(
        &app,
        post_json("/api/time/toggle", &json!({"slot": 2, "now": NOW})),
    )
    .await;
    assert_eq!(started["done"]["started"], "clients:acme:advisory");
    assert_eq!(
        started["state"]["running"]["account"],
        "clients:acme:advisory"
    );
    assert_eq!(started["state"]["running"]["slot"], 2);
    assert!(
        read(&app, "time/pat/2026-11.timedot")
            .contains("running clients:acme:advisory  since 14:05:12")
    );
    // `+10` moves the running start back.
    let adjusted = ok(
        &app,
        post_json(
            "/api/time/adjust",
            &json!({"slot": 2, "minutes": 10, "now": "2026-11-26T14:06:00"}),
        ),
    )
    .await;
    assert_eq!(adjusted["done"]["since"], "2026-11-26T13:55:12");
    // A note with a tag is refused, naming it, with a rewrite.
    let (status, refused) = json_of(
        &app,
        post_json(
            "/api/time/note",
            &json!({"slot": 2, "note": "call re: K-1", "now": NOW}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused["code"], "note-tag");
    assert_eq!(refused["offending"], "re:");
    assert_eq!(refused["suggestion"], "call re K-1");
    ok(
        &app,
        post_json(
            "/api/time/note",
            &json!({"slot": 2, "note": "call about the K-1", "now": NOW}),
        ),
    )
    .await;
    // Other…: an account outside the slots starts and becomes slot 0.
    let other = ok(
        &app,
        post_json(
            "/api/time/toggle",
            &json!({"account": "internal:sales", "now": "2026-11-26T14:55:12"}),
        ),
    )
    .await;
    assert_eq!(other["done"]["stopped"]["days"][0][1], 60);
    assert_eq!(other["done"]["started"], "internal:sales");
    let zero = &other["state"]["slots"][9];
    assert_eq!(zero["key"], 0);
    assert_eq!(zero["account"], "internal:sales", "{other}");
    // Slot 0 is kept per ledger in ui-prefs.toml; the person for everyone.
    let prefs = std::fs::read_to_string(
        root(&app)
            .parent()
            .expect("parent")
            .join("config/config/ui-prefs.toml"),
    )
    .expect("ui-prefs.toml");
    assert!(prefs.contains("[ledgers."), "{prefs}");
    assert!(prefs.contains("other = \"internal:sales\""), "{prefs}");
    assert!(prefs.contains("[time]\nperson = \"pat\""), "{prefs}");
    let text = read(&app, "time/pat/2026-11.timedot");
    assert!(
        text.contains("clients:acme:advisory             2h  ; call about the K-1"),
        "{text}"
    );
    // Stop; the resume hint is what stopped.
    let stopped = ok(
        &app,
        post_json("/api/time/stop", &json!({"now": "2026-11-26T15:25:12"})),
    )
    .await;
    assert_eq!(stopped["done"]["stopped"]["days"][0][1], 30);
    assert_eq!(stopped["state"]["running"], Value::Null);
    assert_eq!(stopped["state"]["last"]["account"], "internal:sales");
    assert_eq!(stopped["state"]["last"]["slot"], 0);
    // Started late: snap back to this app's last stop today.
    ok(
        &app,
        post_json(
            "/api/time/toggle",
            &json!({"slot": 2, "now": "2026-11-26T15:40:00"}),
        ),
    )
    .await;
    let snapped = ok(
        &app,
        post_json("/api/time/snap", &json!({"now": "2026-11-26T15:41:00"})),
    )
    .await;
    assert_eq!(snapped["done"]["since"], "2026-11-26T15:25:12");
    let stopped = ok(
        &app,
        post_json("/api/time/stop", &json!({"now": "2026-11-26T16:00:00"})),
    )
    .await;
    assert_eq!(stopped["done"]["stopped"]["days"][0][1], 35);
    let (status, nothing) = json_of(&app, post_json("/api/time/stop", &json!({"now": NOW}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(nothing["code"], "conflict");
    // An empty slot says so.
    let (status, empty) = json_of(
        &app,
        post_json("/api/time/toggle", &json!({"slot": 9, "now": NOW})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{empty}");
}

#[tokio::test(flavor = "multi_thread")]
async fn entries_list_edit_and_refuse_locked_lines() {
    let app = time_app();
    let list = ok(
        &app,
        authed("/api/time/entries?customer=Acme%20Corp&billed=invoiced"),
    )
    .await;
    let entries = list["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2);
    assert_eq!(list["total"]["text"], "5h");
    let locked = entries[0]["id"].as_str().expect("id").to_string();
    assert!(
        entries[0]["locked"]
            .as_str()
            .is_some_and(|r| r.contains("101"))
    );
    let (status, refused) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/time/entries",
            &json!({"id": locked, "duration": "4h"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(refused["code"], "locked");
    let added = ok(
        &app,
        post_json(
            "/api/time/entries",
            &json!({"date": "2026-11-27", "account": "internal:admin", "duration": "1h30", "person": "pat"}),
        ),
    )
    .await;
    assert_eq!(added["done"], "time/pat/2026-11.timedot");
    let list = ok(
        &app,
        authed("/api/time/entries?from=2026-11-27&to=2026-11-27"),
    )
    .await;
    let id = list["entries"][0]["id"].as_str().expect("id").to_string();
    ok(
        &app,
        with_body(
            "PUT",
            "/api/time/entries",
            &json!({"id": id, "note": "expense reports"}),
        ),
    )
    .await;
    assert!(read(&app, "time/pat/2026-11.timedot").contains("90m  ; expense reports"));
    let bulk = ok(
        &app,
        post_json(
            "/api/time/entries/bulk",
            &json!({"ids": [id], "op": "person", "person": "sam"}),
        ),
    )
    .await;
    assert_eq!(bulk["done"], 1);
    ok(
        &app,
        delete(&format!("/api/time/entries?id={}", id.replace('@', "%40"))),
    )
    .await;
    assert!(!read(&app, "time/pat/2026-11.timedot").contains("expense reports"));
    let (status, bad) = json_of(
        &app,
        post_json(
            "/api/time/entries",
            &json!({"date": "2026-11-27", "account": "internal:admin", "duration": "soon"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
}

#[tokio::test(flavor = "multi_thread")]
async fn accounts_summary_and_unbilled() {
    let app = time_app();
    let view = ok(&app, authed("/api/time/accounts?month=2026-11")).await;
    let tax = view["accounts"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["account"] == "clients:acme:tax-return")
        .expect("row")
        .clone();
    assert_eq!(tax["customer"]["from"], "clients:acme");
    assert_eq!(tax["customer"]["inherited"], true);
    assert_eq!(view["undeclared"], json!(["internal:training"]));
    ok(
        &app,
        post_json(
            "/api/time/accounts/set-tag",
            &json!({"account": "clients:initech", "tag": "item", "value": "ARCH"}),
        ),
    )
    .await;
    assert!(read(&app, "time-accounts.journal").contains("customer: Initech Inc, item: ARCH"));
    let preview = ok(
        &app,
        post_json(
            "/api/time/accounts/rename",
            &json!({"from": "internal:admin", "to": "internal:office"}),
        ),
    )
    .await;
    assert_eq!(preview["done"]["applied"], false);
    assert!(read(&app, "time/pat/2026-11.timedot").contains("internal:admin"));
    ok(
        &app,
        post_json(
            "/api/time/accounts/archive",
            &json!({"account": "internal:training"}),
        ),
    )
    .await;
    assert!(read(&app, "time-accounts.journal").contains("account internal:training"));
    let (status, bad) = json_of(
        &app,
        post_json(
            "/api/time/accounts/set-tag",
            &json!({"account": "internal", "tag": "color", "value": "red"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");

    let summary = ok(
        &app,
        authed(
            "/api/time/summary?group=customer,billable&from=2026-11-01&to=2026-11-30&series=week",
        ),
    )
    .await;
    assert!(summary["rows"].as_array().is_some_and(|r| !r.is_empty()));
    assert_eq!(summary["buckets"][0], "2026-W45");
    assert_eq!(summary["file"], "time.journal");
    let notes = ok(
        &app,
        authed("/api/time/summary?group=note&from=2026-11-01&to=2026-11-30"),
    )
    .await;
    assert!(
        notes["rows"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|r| r["key"][0] == "email"))
    );
    let tree = ok(&app, authed("/api/time/summary?view=tree")).await;
    assert_eq!(tree["tree"][0]["account"], "clients");
    let (status, _) = json_of(&app, authed("/api/time/summary?group=colour")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let unbilled = ok(&app, authed("/api/time/unbilled?today=2026-11-26")).await;
    let acme = unbilled["customers"]
        .as_array()
        .expect("customers")
        .iter()
        .find(|c| c["customer"] == "Acme Corp")
        .expect("acme")
        .clone();
    assert_eq!(acme["periods"][0]["from"], "2026-10-01");
    assert_eq!(unbilled["needs_customer"][0]["account"], "clients:orphan");
    let unpaid = ok(&app, authed("/api/time/unpaid?today=2026-11-26")).await;
    assert_eq!(unpaid.as_array().map(Vec::len), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn bill_preview_create_then_delete_the_draft_releases_its_time() {
    let app = time_app();
    let request = json!({
        "customer": "Acme Corp",
        "from": "2026-11-01",
        "to": "2026-11-30",
        "group": "account-person",
        "today": "2026-11-26",
    });
    let mut preview_request = request.clone();
    preview_request["preview"] = json!(true);
    let preview = ok(&app, post_json("/api/time/bill", &preview_request)).await;
    assert_eq!(preview["ready"], true, "{preview}");
    assert_eq!(preview["amount"], "$1,390.00");
    assert_eq!(preview["customer"]["rounding"]["increment"], 6);
    assert!(
        !root(&app)
            .join("invoices/2026-11-26 Acme Corp Invoice 102.md")
            .exists()
    );
    let created = ok(&app, post_json("/api/time/bill", &request)).await;
    assert_eq!(created["number"], "102");
    assert_eq!(created["tagged"].as_array().map(Vec::len), Some(6));
    let invoice = ok(&app, authed("/api/invoices/102")).await;
    assert_eq!(invoice["status"], "draft");
    assert!(read(&app, "time/pat/2026-11.timedot").contains("invoice: 102"));
    // Again: nothing unbilled is left for the period.
    let (status, again) = json_of(&app, post_json("/api/time/bill", &request)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");
    // Delete the draft: its time is released.
    let deleted = ok(&app, delete("/api/invoices/102")).await;
    assert_eq!(deleted["time"]["released"], 6);
    assert!(!read(&app, "time/pat/2026-11.timedot").contains("invoice: 102"));
    let (status, sent) = json_of(&app, delete("/api/invoices/101")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{sent}");
    assert_eq!(sent["code"], "status");
}

#[tokio::test(flavor = "multi_thread")]
async fn void_can_write_the_time_off() {
    let app = time_app();
    let voided = ok(
        &app,
        post_json(
            "/api/invoices/101/void",
            &json!({"date": "2026-11-26", "today": "2026-11-26", "time": "nobill"}),
        ),
    )
    .await;
    assert_eq!(voided["time"]["released"], 2, "{voided}");
    let text = read(&app, "time/pat/2026-10.timedot");
    assert!(
        text.contains("first pass, nobill: invoice 101 void"),
        "{text}"
    );
    assert!(!text.contains("invoice: 101"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pay_lists_what_it_needs_then_books_the_bill() {
    let app = time_app();
    let needs = ok(
        &app,
        post_json(
            "/api/time/pay",
            &json!({"person": "riley", "today": "2026-11-26"}),
        ),
    )
    .await;
    assert_eq!(needs["ready"], false);
    let fields: Vec<&str> = needs["needs"]
        .as_array()
        .expect("needs")
        .iter()
        .map(|n| n["field"].as_str().expect("field"))
        .collect();
    assert_eq!(fields, ["rate", "expense_account"]);
    let booked = ok(
        &app,
        post_json(
            "/api/time/pay",
            &json!({
                "person": "riley",
                "rate": "$60",
                "expense_account": "expenses:contractors",
                "today": "2026-11-26",
            }),
        ),
    )
    .await;
    assert_eq!(booked["amount"], "$180.00");
    assert_eq!(
        booked["saved_to_note"],
        json!(["pay_rate", "expense_account"])
    );
    assert!(read(&app, "books.journal").contains("Riley Chen | riley timesheet 2026-11"));
    assert!(read(&app, "time/riley/2026-11.timedot").contains("bill: riley timesheet 2026-11"));
    let entries = ok(&app, authed("/api/time/entries?person=riley")).await;
    assert!(
        entries["entries"][0]["locked"].is_string(),
        "a booked bill locks it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_outside_edit_reaches_the_state_and_the_change_events() {
    let app = time_app();
    let before = ok(&app, authed(&format!("/api/time/state?now={NOW}"))).await;
    assert_eq!(before["person"], Value::Null);
    let mut events = app.books.subscribe();
    let file = root(&app).join("time/pat/2026-11.timedot");
    let text = std::fs::read_to_string(&file).expect("reads").replace(
        "internal:admin                   15m\n",
        "internal:admin                   45m\n",
    );
    std::fs::write(&file, text).expect("write");
    app.books.apply_changes(&[file]);
    let changed = events.try_recv().expect("an event");
    assert_eq!(changed.paths, ["time/pat/2026-11.timedot"]);
    let entries = ok(
        &app,
        authed("/api/time/entries?from=2026-11-26&to=2026-11-26&person=pat"),
    )
    .await;
    assert_eq!(entries["total"]["text"], "1h 45m", "the cache was dropped");
}
