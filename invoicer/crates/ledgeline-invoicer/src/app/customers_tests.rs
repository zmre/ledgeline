//! The customer routes (PLAN 4b) in-process, over a private fixture copy.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};

use super::tests::{App, HOST, TOKEN, app, app_on, authed, books_root, json_of, post_json};

fn put_json(path: &str, body: &Value) -> Request<Body> {
    Request::put(path)
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// `/api/customers/{path}`, each segment percent-encoded as the SPA sends it.
fn customer_url(path: &str) -> String {
    let encoded: Vec<String> = path.split('/').map(|s| s.replace(' ', "%20")).collect();
    format!("/api/customers/{}", encoded.join("/"))
}

async fn detail(app: &App, path: &str) -> Value {
    let (status, body) = json_of(
        app,
        authed(&format!("{}?today=2026-11-22", customer_url(path))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_carries_each_customers_account() {
    let app = app();
    let (status, list) = json_of(&app, authed("/api/customers?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let list = list.as_array().expect("array");
    let titles: Vec<&str> = list.iter().filter_map(|c| c["title"].as_str()).collect();
    assert_eq!(
        titles,
        [
            "Contoso",
            "Customer X",
            "Fabrikam",
            "Fred Smith",
            "Tailspin",
            "Wingtip"
        ]
    );
    let contoso = &list[0];
    // The CLI's row, plus the account.
    assert_eq!(contoso["path"], "customers/Contoso.md");
    assert_eq!(contoso["kind"], "organization");
    assert_eq!(contoso["account"]["balance"][0]["amount"], "$2,000.00");
    assert_eq!(contoso["account"]["overdue"], 1);
    assert_eq!(contoso["account"]["last_payment"]["date"], "2026-11-05");
    let fabrikam = &list[2];
    assert_eq!(fabrikam["account"]["balance"], json!([]));
    let (status, _) = json_of(&app, authed("/api/customers?today=soon")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_customer_shows_its_invoices_account_form_and_backlinks() {
    let app = app();
    let x = detail(&app, "customers/Customer X.md").await;
    assert_eq!(x["title"], "Customer X");
    assert_eq!(x["ledger_customer"], "Customer X");
    let numbers: Vec<&str> = x["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .filter_map(|d| d["number"].as_str())
        .collect();
    assert_eq!(numbers, ["108", "105", "104"], "newest first");
    let first = &x["documents"][0];
    for key in ["status", "total", "due_date", "open", "overdue"] {
        assert!(first.get(key).is_some(), "{key}: {first}");
    }
    assert_eq!(x["account"]["balance"][0]["amount"], "$8,350.00");
    // The hash is the file's.
    let text = std::fs::read(books_root(&app).join("customers/Customer X.md")).expect("reads");
    assert_eq!(x["hash"], invoicer_core::customer_edit::content_hash(&text));
    // The form: as the note writes it.
    assert_eq!(x["form"]["title"], "Customer X");
    assert_eq!(x["form"]["kind"], "organization");
    assert_eq!(x["form"]["terms"], 30);
    assert_eq!(x["form"]["emails"][0]["label"], "billing");
    assert_eq!(x["form"]["billing_portal"]["requires_reference"], true);
    assert_eq!(x["form"]["company"], "");
    // MBR's backlinks (M1): its invoices link it from `customer:`.
    let backlinks = x["backlinks"].as_array().expect("backlinks");
    let from_105 = backlinks
        .iter()
        .find(|b| {
            b["path"]
                .as_str()
                .is_some_and(|p| p.ends_with("Customer X Invoice 105.md"))
        })
        .unwrap_or_else(|| panic!("105 links Customer X: {backlinks:?}"));
    assert_eq!(from_105["key"], "customer");
    assert!(
        from_105["url"]
            .as_str()
            .is_some_and(|u| u.starts_with("/invoices/")),
        "{from_105}"
    );

    // By name, by path without `.md`, and the refusals.
    let by_name = detail(&app, "Customer X").await;
    assert_eq!(by_name["path"], "customers/Customer X.md");
    let bare = detail(&app, "customers/Customer X").await;
    assert_eq!(bare["path"], "customers/Customer X.md");
    for missing in ["Nobody", "company/My Company.md", "invoices/catalog.md"] {
        let (status, body) = json_of(&app, authed(&customer_url(missing))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}: {body}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_a_customer_writes_its_note_and_lists_it() {
    let app = app();
    let (status, made) = json_of(
        &app,
        post_json(
            "/api/customers",
            &json!({
                "title": "Northwind Traders",
                "kind": "organization",
                "emails": [{"label": "billing", "value": "ap@northwind.example"}],
                "terms": 15
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["path"], "customers/Northwind Traders.md");
    assert_eq!(made["link"], "[[Northwind Traders]]");
    assert_eq!(made["ledger_customer"], "Northwind Traders");
    assert_eq!(made["kind"], "organization");
    assert_eq!(made["written"], true);
    let file = books_root(&app).join("customers/Northwind Traders.md");
    let text = std::fs::read_to_string(&file).expect("written");
    assert!(text.contains("tags:\n  - customer\n"), "{text}");
    assert_eq!(
        made["hash"],
        invoicer_core::customer_edit::content_hash(text.as_bytes())
    );
    // At once, without waiting for the watcher.
    let (_, list) = json_of(&app, authed("/api/customers")).await;
    assert!(
        list.as_array()
            .expect("array")
            .iter()
            .any(|c| c["title"] == "Northwind Traders"),
        "{list}"
    );
    let (_, books) = json_of(&app, authed("/api/books")).await;
    assert_eq!(books["customers"], 7);

    // Taken, malformed, unknown.
    let again = post_json(
        "/api/customers",
        &json!({"title": "northwind traders", "kind": "person"}),
    );
    let (status, body) = json_of(&app, again).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = json_of(
        &app,
        post_json("/api/customers", &json!({"title": "No Kind"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/customers",
            &json!({"title": "A", "kind": "person", "colour": "red"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!books_root(&app).join("customers/No Kind.md").exists());
    assert!(!books_root(&app).join("customers/A.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_a_customer_changes_only_what_the_form_changed() {
    let app = app();
    let path = "customers/Fabrikam.md";
    let file = books_root(&app).join(path);
    let before = std::fs::read_to_string(&file).expect("reads");
    let shown = detail(&app, path).await;

    // The form sent back as it came: nothing is written (the address map
    // stays a map).
    let (status, saved) = json_of(
        &app,
        put_json(
            &customer_url(path),
            &json!({"hash": shown["hash"], "fields": shown["form"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["written"], false);
    assert_eq!(std::fs::read_to_string(&file).expect("reads"), before);

    // One field: one line.
    let (status, saved) = json_of(
        &app,
        put_json(
            &customer_url(path),
            &json!({"hash": shown["hash"], "fields": {"terms": 45}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["changed"], json!(["terms"]));
    let after = std::fs::read_to_string(&file).expect("reads");
    assert_eq!(after, before.replace("terms: 30\n", "terms: 45\n"));
    assert_eq!(
        saved["hash"],
        invoicer_core::customer_edit::content_hash(after.as_bytes())
    );

    // The old hash is stale now: refused, and the file stays.
    let (status, body) = json_of(
        &app,
        put_json(
            &customer_url(path),
            &json!({"hash": shown["hash"], "fields": {"terms": 60}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(std::fs::read_to_string(&file).expect("reads"), after);

    // An outside edit since the page was shown is refused too.
    let current = detail(&app, path).await;
    let outside = format!("{after}\nPays by ACH.\n");
    std::fs::write(&file, &outside).expect("writes");
    let (status, _) = json_of(
        &app,
        put_json(
            &customer_url(path),
            &json!({"hash": current["hash"], "fields": {"terms": 60}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(std::fs::read_to_string(&file).expect("reads"), outside);
    // Reloaded, the page has the new hash and the edit goes through.
    let reloaded = detail(&app, path).await;
    assert_ne!(reloaded["hash"], current["hash"]);
    let (status, saved) = json_of(
        &app,
        put_json(
            &customer_url(path),
            &json!({"hash": reloaded["hash"], "fields": {"terms": 60}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let last = std::fs::read_to_string(&file).expect("reads");
    assert!(
        last.contains("terms: 60\n") && last.ends_with("Pays by ACH.\n"),
        "{last}"
    );

    // A body that doesn't read, a field that doesn't.
    let (status, _) = json_of(
        &app,
        put_json(&customer_url(path), &json!({"fields": {"terms": 1}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = json_of(
        &app,
        put_json(
            &customer_url(path),
            &json!({"hash": saved["hash"], "fields": {"terms": 99999}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_customer_can_be_edited_on_cash_books_too() {
    let app = app_on("cash", None);
    let (status, made) = json_of(
        &app,
        post_json(
            "/api/customers",
            &json!({"title": "Lee Park", "kind": "person", "company": "Customer X"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let shown = detail(&app, made["path"].as_str().expect("path")).await;
    assert_eq!(shown["hash"], made["hash"]);
    assert_eq!(shown["form"]["company"], "[[Customer X]]");
    // A person at a company bills to it.
    assert_eq!(shown["billing"]["name"], "Customer X");
    assert_eq!(shown["documents"], json!([]));
    assert_eq!(shown["account"]["balance"], json!([]));
    let (status, saved) = json_of(
        &app,
        put_json(
            &customer_url(made["path"].as_str().expect("path")),
            &json!({"hash": shown["hash"], "fields": {"emails": [{"label": "work", "value": "lee@customerx.com"}]}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["changed"], json!(["emails"]));
}
