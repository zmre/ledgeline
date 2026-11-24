//! The catalog and snippet routes (PLAN 4c) in-process, over a private
//! fixture copy.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};

use super::tests::{App, HOST, TOKEN, app, authed, books_root, json_of, post_json};

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

fn url(base: &str, path: &str) -> String {
    let encoded: Vec<String> = path.split('/').map(|s| s.replace(' ', "%20")).collect();
    format!("{base}/{}", encoded.join("/"))
}

fn read(app: &App, path: &str) -> String {
    std::fs::read_to_string(books_root(app).join(path)).expect("reads")
}

async fn catalog(app: &App) -> Value {
    let (status, body) = json_of(app, authed("/api/catalog")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn the_catalog_is_edited_as_a_grid_and_offered_to_the_editor() {
    let app = app();
    let before = read(&app, "invoices/catalog.md");
    let body = catalog(&app).await;
    assert_eq!(body["path"], "invoices/catalog.md");
    let note = &body["catalogs"][0];
    assert_eq!(note["path"], "invoices/catalog.md");
    assert_eq!(note["problems"], json!([]));
    let roles: Vec<&str> = note["table"]["columns"]
        .as_array()
        .expect("columns")
        .iter()
        .filter_map(|c| c["role"].as_str())
        .collect();
    assert_eq!(roles, ["item", "description", "unit", "rate", "books"]);
    let hash = note["hash"].as_str().expect("hash").to_string();

    // An unchanged grid writes nothing.
    let (status, saved) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/catalog/invoices/catalog.md",
            &json!({"hash": hash, "table": note["table"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["written"], false);
    assert_eq!(read(&app, "invoices/catalog.md"), before);

    // Add an item; the `.md` is optional in the URL.
    let mut table = note["table"].clone();
    table["rows"].as_array_mut().expect("rows").push(json!([
        "AUDIT",
        "Security audit",
        "ea",
        "$5,000",
        "revenue:consulting"
    ]));
    let (status, saved) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/catalog/invoices/catalog",
            &json!({"hash": hash, "table": table}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["written"], true);
    assert_eq!(saved["items"], 5);
    let after = read(&app, "invoices/catalog.md");
    assert!(after.starts_with(&before), "only appended:\n{after}");
    assert!(
        after.ends_with(
            "| AUDIT  | Security audit                | ea   | $5,000  | revenue:consulting                                     |\n"
        ),
        "{after}"
    );
    let (_, options) = json_of(&app, authed("/api/editor/options")).await;
    assert!(
        options["catalog"]
            .as_array()
            .expect("catalog")
            .iter()
            .any(|i| i["code"] == "AUDIT" && i["description"] == "Security audit"),
        "{options}"
    );

    // The old hash is a conflict, with the current one.
    let (status, refused) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/catalog/invoices/catalog.md",
            &json!({"hash": hash, "table": table}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "changed");
    assert_eq!(refused["revision"], saved["hash"]);

    // Cells that don't read are named.
    table["rows"]
        .as_array_mut()
        .expect("rows")
        .push(json!(["arch", "Again", "h", "lots", ""]));
    let (status, refused) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/catalog/invoices/catalog.md",
            &json!({"hash": saved["hash"], "table": table}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "invalid");
    assert_eq!(refused["problems"][0]["row"], 5);
    assert_eq!(refused["problems"][0]["column"], 0);
    assert_eq!(refused["problems"][1]["column"], 3);

    // Not a catalog; not there; already there.
    let (status, _) = json_of(
        &app,
        with_body(
            "PUT",
            &url("/api/catalog", "customers/Customer X.md"),
            &json!({"hash": "x", "table": table}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/catalog/nope.md",
            &json!({"hash": "x", "table": table}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, refused) = json_of(&app, post_json("/api/catalog", &json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "exists");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_books_folder_without_a_catalog_can_start_one() {
    let app = app();
    std::fs::remove_file(books_root(&app).join("invoices/catalog.md")).expect("removes");
    {
        let mut session = app.books.session();
        let (index, vault) = session.parts_mut();
        let path = ledgeline_core::vault::VaultPath::new("invoices/catalog.md").expect("path");
        index.refresh(vault, std::slice::from_ref(&path));
    }
    assert_eq!(catalog(&app).await["catalogs"], json!([]));
    let (status, made) = json_of(&app, post_json("/api/catalog", &json!({}))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["path"], "invoices/catalog.md");
    let body = catalog(&app).await;
    assert_eq!(body["catalogs"][0]["table"]["rows"], json!([]));
    assert_eq!(body["catalogs"][0]["hash"], made["hash"]);
}

async fn snippets(app: &App) -> Vec<Value> {
    let (status, body) = json_of(app, authed("/api/snippets")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().expect("array").clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn snippets_are_listed_with_their_uses_created_edited_and_deleted() {
    let app = app();
    let list = snippets(&app).await;
    let titles: Vec<&str> = list.iter().filter_map(|s| s["title"].as_str()).collect();
    assert_eq!(titles, ["Net 30 ACH", "Thank you"]);
    let net = &list[0];
    assert_eq!(net["path"], "invoices/tpl/snippets/net-30.md");
    assert_eq!(net["kind"], "payment");
    assert!(
        net["body"]
            .as_str()
            .expect("body")
            .starts_with("## Payment\n")
    );
    let first = &net["used_by"][0];
    assert_eq!(first["path"], "customers/Customer X.md");
    assert_eq!(first["how"], "default");

    // Create one.
    let (status, made) = json_of(
        &app,
        post_json(
            "/api/snippets",
            &json!({"title": "Late fee", "kind": "terms", "body": "A late fee applies."}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["path"], "invoices/tpl/snippets/Late fee.md");
    assert_eq!(made["link"], "[[Late fee]]");
    assert_eq!(
        read(&app, "invoices/tpl/snippets/Late fee.md"),
        "---\ntitle: Late fee\ntype: snippet\nsnippet_kind: terms\n---\n\nA late fee applies.\n"
    );
    let (status, refused) = json_of(
        &app,
        post_json("/api/snippets", &json!({"title": "late fee"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "name-taken");
    let (status, _) = json_of(
        &app,
        post_json("/api/snippets", &json!({"title": "X", "password": "no"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Edit its body and kind.
    let late = url("/api/snippets", "invoices/tpl/snippets/Late fee.md");
    let (status, saved) = json_of(
        &app,
        with_body(
            "PUT",
            &late,
            &json!({"hash": made["hash"], "fields": {"kind": "note", "body": "A fee of **1.5%** a month."}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["changed"], json!(["snippet_kind", "body"]));
    assert_eq!(
        read(&app, "invoices/tpl/snippets/Late fee.md"),
        "---\ntitle: Late fee\ntype: snippet\nsnippet_kind: note\n---\n\nA fee of **1.5%** a month.\n"
    );
    // The old hash is a conflict.
    let (status, refused) = json_of(
        &app,
        with_body(
            "PUT",
            &late,
            &json!({"hash": made["hash"], "fields": {"body": "Mine"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "changed");
    assert_eq!(refused["revision"], saved["hash"]);

    // Delete: Net 30 ACH is Customer X's default, so only with force.
    let net_url = url("/api/snippets", "invoices/tpl/snippets/net-30.md");
    let (status, refused) = json_of(
        &app,
        with_body("DELETE", &net_url, &json!({"hash": net["hash"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "in-use");
    assert_eq!(refused["customers"], json!(["Customer X"]));
    let (status, gone) = json_of(
        &app,
        with_body(
            "DELETE",
            &net_url,
            &json!({"hash": net["hash"], "force": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gone}");
    assert!(
        !books_root(&app)
            .join("invoices/tpl/snippets/net-30.md")
            .exists()
    );
    let (status, gone) = json_of(
        &app,
        with_body("DELETE", &late, &json!({"hash": saved["hash"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gone}");
    let titles: Vec<String> = snippets(&app)
        .await
        .iter()
        .filter_map(|s| s["title"].as_str().map(str::to_string))
        .collect();
    assert_eq!(titles, ["Thank you"]);
    // A customer isn't a snippet.
    let (status, _) = json_of(
        &app,
        with_body(
            "DELETE",
            &url("/api/snippets", "customers/Customer X.md"),
            &json!({"hash": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
