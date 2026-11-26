//! The index's caches (review RA-3, RC-5): list, show, sidebar and customer
//! requests read templates, catalogs and each customer's documents from the
//! index, never the files. A hand edit shows on the next request once the
//! watcher reports it, and the app's own writes show at once.

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::tests::{App, app, authed, books_root, json_of, post_json};

const INVOICE_105: &str = "invoices/2026-11-10 Customer X Invoice 105.md";

async fn sidebar(app: &App) -> Value {
    let (status, body) = json_of(app, authed("/api/books?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn problems_of_105(app: &App) -> Vec<String> {
    let (status, body) = json_of(app, authed("/api/invoices/105?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .filter_map(|p| p["message"].as_str().map(str::to_string))
        .collect()
}

async fn invoices_of(app: &App, title: &str) -> u64 {
    let (status, list) = json_of(app, authed("/api/customers?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list.as_array()
        .expect("rows")
        .iter()
        .find(|c| c["title"] == title)
        .and_then(|c| c["invoices"].as_u64())
        .expect("the customer")
}

/// Write `text` at `rel` under the books folder, then report it as the
/// watcher would.
fn hand_write(app: &App, rel: &str, text: &str) {
    let file = books_root(app).join(rel);
    std::fs::write(&file, text).expect("writes");
    app.books.apply_changes(std::slice::from_ref(&file));
}

#[tokio::test(flavor = "multi_thread")]
async fn hand_edits_and_new_files_show_once_the_watcher_reports_them() {
    let app = app();
    let before = sidebar(&app).await;
    let templates = before["templates"].as_u64().expect("templates");
    let catalog = before["catalog"].as_u64().expect("catalog");
    assert!(
        !problems_of_105(&app)
            .await
            .iter()
            .any(|m| m.contains("isn't a template")),
    );

    // A new template file: counted.
    hand_write(
        &app,
        "invoices/tpl/mine.html",
        "{# invoicer: name=\"Mine\" #}\n<p>{{ frontmatter.invoice_num }}</p>\n",
    );
    assert_eq!(sidebar(&app).await["templates"], json!(templates + 1));

    // A hand edit that makes it no template any more: no longer counted.
    hand_write(&app, "invoices/tpl/mine.html", "<p>just a page</p>\n");
    assert_eq!(sidebar(&app).await["templates"], json!(templates));

    // An invoice naming a template that isn't there is a problem; writing
    // the template makes it go away.
    let path = books_root(&app).join(INVOICE_105);
    let text = std::fs::read_to_string(&path).expect("reads");
    hand_write(
        &app,
        INVOICE_105,
        &text.replace(
            "template: /invoices/tpl/myco-default.html",
            "template: /invoices/tpl/later.html",
        ),
    );
    assert!(
        problems_of_105(&app)
            .await
            .iter()
            .any(|m| m.contains("`template: /invoices/tpl/later.html` isn't a template")),
    );
    hand_write(
        &app,
        "invoices/tpl/later.html",
        "<p>{{ frontmatter.invoice_num }}</p>\n",
    );
    assert!(
        !problems_of_105(&app)
            .await
            .iter()
            .any(|m| m.contains("isn't a template")),
    );

    // A hand-added catalog row: counted.
    let catalog_path = books_root(&app).join("invoices/catalog.md");
    let catalog_text = std::fs::read_to_string(&catalog_path).expect("reads");
    hand_write(
        &app,
        "invoices/catalog.md",
        &format!("{catalog_text}| AUDIT | Security audit | ea | $5,000 | revenue:consulting |\n"),
    );
    assert_eq!(sidebar(&app).await["catalog"], json!(catalog + 1));

    // A new invoice file for a customer: on its row in the customer list.
    let wingtip = invoices_of(&app, "Wingtip").await;
    let new_one = text
        .replace("invoice_num: 105", "invoice_num: 9105")
        .replace("[[Customer X]]", "[[Wingtip]]")
        .replace("status: sent", "status: draft");
    hand_write(
        &app,
        "invoices/2026-11-20 Wingtip Invoice 9105.md",
        &new_one,
    );
    assert_eq!(invoices_of(&app, "Wingtip").await, wingtip + 1);
    let (status, found) = json_of(&app, authed("/api/invoices/9105?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::OK, "{found}");

    // Removed: gone from both.
    let gone = books_root(&app).join("invoices/2026-11-20 Wingtip Invoice 9105.md");
    std::fs::remove_file(&gone).expect("removes");
    app.books.apply_changes(std::slice::from_ref(&gone));
    assert_eq!(invoices_of(&app, "Wingtip").await, wingtip);
    let (status, _) = json_of(&app, authed("/api/invoices/9105?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_own_writes_show_at_once() {
    let app = app();
    let templates = sidebar(&app).await["templates"]
        .as_u64()
        .expect("templates");
    let (status, made) = json_of(
        &app,
        post_json("/api/templates", &json!({"name": "Ocean blue"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    // No watcher event: the write refreshed the index itself.
    assert_eq!(sidebar(&app).await["templates"], json!(templates + 1));

    let catalog = sidebar(&app).await["catalog"].as_u64().expect("catalog");
    let (status, body) = json_of(&app, authed("/api/catalog")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let note = &body["catalogs"][0];
    let mut table = note["table"].clone();
    table["rows"].as_array_mut().expect("rows").push(json!([
        "AUDIT",
        "Security audit",
        "ea",
        "$5,000",
        "revenue:consulting"
    ]));
    let request = axum::http::Request::builder()
        .method("PUT")
        .uri("/api/catalog/invoices/catalog.md")
        .header(axum::http::header::HOST, super::tests::HOST)
        .header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", super::tests::TOKEN),
        )
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(
            json!({"hash": note["hash"], "table": table}).to_string(),
        ))
        .expect("request");
    let (status, saved) = json_of(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(sidebar(&app).await["catalog"], json!(catalog + 1));
}
