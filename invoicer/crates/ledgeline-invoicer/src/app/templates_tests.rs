//! The template editor's routes (PLAN 4d) in-process, over a private
//! fixture copy.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};

use super::{
    App, BASE_FILE, HOST, TOKEN, app, authed, books_root, json_of, post_json, send, session_cookie,
};

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

fn read(app: &App, path: &str) -> String {
    std::fs::read_to_string(books_root(app).join(path)).expect("reads")
}

async fn list(app: &App) -> Vec<Value> {
    let (status, body) = json_of(app, authed("/api/templates")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().expect("a list").clone()
}

/// The preview page `url` names, as the frame loads it (the session cookie).
async fn frame(app: &App, url: &str) -> (StatusCode, String) {
    let cookie = session_cookie(app).await;
    let request = Request::get(url)
        .header(header::HOST, HOST)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    let (status, _, html) = send(&app.router, request).await;
    (status, html)
}

async fn preview(app: &App, template: &str, body: &str) -> Value {
    let (status, answer) = json_of(
        app,
        post_json(
            "/api/templates/preview",
            &json!({"number": "105", "template": template, "body": body}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_says_what_each_extends_and_who_uses_it() {
    let app = app();
    let all = list(&app).await;
    let paths: Vec<&str> = all.iter().filter_map(|t| t["path"].as_str()).collect();
    assert_eq!(
        paths,
        [
            "/invoices/tpl/myco-default.html",
            "/invoices/tpl/classic.html",
            "/invoices/tpl/modern.html"
        ]
    );
    let myco = &all[0];
    assert_eq!(myco["name"], "MyCo default");
    assert_eq!(myco["kinds"], json!(["invoice", "estimate"]));
    assert_eq!(myco["extends"], "_invoice_base.v1.html");
    assert_eq!(myco["base_version"], 1);
    assert_eq!(myco["overrides"], json!(["ready"]));
    assert!(myco["hash"].is_string());
    let used = myco["used_by"].as_array().expect("uses");
    let customers = used.iter().filter(|u| u["how"] == "customer").count();
    assert_eq!(customers, 5, "{used:?}");
    // A starter not copied in: no hash, the base it would extend, unused.
    let classic = &all[1];
    assert_eq!(classic["installed"], false);
    assert_eq!(classic["hash"], Value::Null);
    assert_eq!(classic["base_version"], 1);
    assert_eq!(classic["used_by"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_child_is_edited_previewed_saved_used_and_deleted() {
    let app = app();
    // New: a blank child of the base, named by the user.
    let (status, made) = json_of(
        &app,
        post_json("/api/templates", &json!({"name": "Ocean blue"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["path"], "invoices/tpl/ocean-blue.html");
    assert_eq!(made["template"], "/invoices/tpl/ocean-blue.html");
    assert_eq!(made["base"], Value::Null, "the fixture has the base");
    let text = read(&app, "invoices/tpl/ocean-blue.html");
    assert_eq!(
        text,
        "{# invoicer: name=\"Ocean blue\" kinds=\"invoice\" #}\n{% extends \"_invoice_base.v1.html\" %}\n"
    );
    // The same name again is refused.
    let (status, taken) = json_of(
        &app,
        post_json("/api/templates", &json!({"name": "ocean BLUE"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{taken}");
    assert_eq!(taken["code"], "name-taken");

    // Read for the editor: the body without the name line, and the base's
    // blocks with what each is for.
    let (status, one) = json_of(&app, authed("/api/templates/invoices/tpl/ocean-blue.html")).await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert_eq!(one["body"], "{% extends \"_invoice_base.v1.html\" %}\n");
    assert_eq!(one["read_only"], false);
    assert_eq!(one["name"], "Ocean blue");
    let blocks: Vec<&str> = one["base_blocks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["name"].as_str())
        .collect();
    assert_eq!(
        blocks,
        [
            "settings", "ready", "head", "theme", "header", "parties", "details", "body", "footer"
        ]
    );
    let theme = one["base_blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "theme")
        .unwrap();
    assert!(
        theme["about"]
            .as_str()
            .unwrap()
            .starts_with("the CSS for the sheet"),
        "{theme}"
    );
    let hash = one["hash"].as_str().unwrap().to_string();

    // Preview unsaved text: the override is on the page; nothing written.
    let body = "{% extends \"_invoice_base.v1.html\" %}\n{% block theme %}{{ super() }}\n.sheet h1 { color: rgb(0, 90, 160); }\n{% endblock theme %}\n";
    let answer = preview(&app, "/invoices/tpl/ocean-blue.html", body).await;
    assert_eq!(answer["problem"], Value::Null, "{answer}");
    let url = answer["url"].as_str().expect("a url");
    assert!(
        url.starts_with("/.ledgeline-invoicer/preview?draft="),
        "{url}"
    );
    let (status, html) = frame(&app, url).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains(".sheet h1 { color: rgb(0, 90, 160); }"),
        "the override"
    );
    assert!(html.contains("Senior architect hours"), "105's table");
    assert!(html.contains("<base href="), "links resolve from the note");
    assert_eq!(read(&app, "invoices/tpl/ocean-blue.html"), text, "unsaved");

    // A broken body: the problem in words, with its line; no page.
    let answer = preview(
        &app,
        "/invoices/tpl/ocean-blue.html",
        "{% extends \"_invoice_base.v1.html\" %}\n{% block footer %}\n{{ nope.nope }}\n{% endblock footer %}\n",
    )
    .await;
    assert_eq!(answer["url"], Value::Null);
    assert_eq!(answer["problem"]["line"], 3, "{answer}");
    assert!(
        answer["problem"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Nothing called nope.nope"),
        "{answer}"
    );
    let answer = preview(
        &app,
        "/invoices/tpl/ocean-blue.html",
        "{% extends \"_invoice_base.v1.html\" %}\n{% block theme %}\n{% if x %}\n{% endblock theme %}\n",
    )
    .await;
    assert_eq!(answer["problem"]["line"], 4, "{answer}");
    assert!(
        answer["problem"]["message"]
            .as_str()
            .unwrap()
            .starts_with("The template doesn't read here:"),
        "{answer}"
    );
    // No such invoice: a 404, not a problem with the template.
    let (status, _) = json_of(
        &app,
        post_json(
            "/api/templates/preview",
            &json!({"number": "9999", "template": "/invoices/tpl/ocean-blue.html", "body": body}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Save the body over the hash read; a rename rewrites the name line.
    let (status, saved) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/templates/invoices/tpl/ocean-blue.html",
            &json!({"hash": hash, "fields": {"body": body}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["changed"], json!(["body"]));
    assert_eq!(
        read(&app, "invoices/tpl/ocean-blue.html"),
        format!("{{# invoicer: name=\"Ocean blue\" kinds=\"invoice\" #}}\n{body}")
    );
    // The stale hash: 409 with the hash now, nothing written.
    let (status, stale) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/templates/invoices/tpl/ocean-blue.html",
            &json!({"hash": hash, "fields": {"name": "Sea"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "changed");
    assert_eq!(stale["revision"], saved["hash"]);
    let (status, renamed) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/templates/invoices/tpl/ocean-blue.html",
            &json!({"hash": saved["hash"], "fields": {"name": "Sea", "kinds": ["invoice", "estimate"]}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert!(
        read(&app, "invoices/tpl/ocean-blue.html")
            .starts_with("{# invoicer: name=\"Sea\" kinds=\"invoice,estimate\" #}\n")
    );

    // Use it on an invoice ("Use this template"): then it can't be deleted.
    let (status, used) = json_of(
        &app,
        post_json(
            "/api/invoices/105/template",
            &json!({"template": "/invoices/tpl/ocean-blue.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{used}");
    let hash = renamed["hash"].as_str().unwrap();
    let (status, refused) = json_of(
        &app,
        with_body(
            "DELETE",
            "/api/templates/invoices/tpl/ocean-blue.html",
            &json!({"hash": hash}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "in-use");
    assert_eq!(refused["invoices"], json!(["105"]));
    assert_eq!(refused["customers"], json!([]));
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .starts_with("Invoice 105 uses it"),
        "{refused}"
    );

    // Back to MyCo: deleted, and gone from the list.
    let (status, _) = json_of(
        &app,
        post_json(
            "/api/invoices/105/template",
            &json!({"template": "/invoices/tpl/myco-default.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, deleted) = json_of(
        &app,
        with_body(
            "DELETE",
            "/api/templates/invoices/tpl/ocean-blue.html",
            &json!({"hash": hash}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert!(
        !books_root(&app)
            .join("invoices/tpl/ocean-blue.html")
            .exists()
    );
    assert!(
        list(&app)
            .await
            .iter()
            .all(|t| t["path"] != "/invoices/tpl/ocean-blue.html")
    );
    assert!(books_root(&app).join(BASE_FILE).exists(), "the base stays");
}

#[tokio::test(flavor = "multi_thread")]
async fn starters_are_read_only_until_customized_and_can_be_duplicated() {
    let app = app();
    let (status, modern) = json_of(&app, authed("/api/templates/invoices/tpl/modern.html")).await;
    assert_eq!(status, StatusCode::OK, "{modern}");
    assert_eq!(modern["read_only"], true);
    assert_eq!(modern["installed"], false);
    assert_eq!(modern["name"], "Modern");
    assert!(
        modern["body"]
            .as_str()
            .unwrap()
            .starts_with("{% extends \"_invoice_base.v1.html\" %}"),
        "the body starts with extends: the name line is the form's"
    );
    assert!(!books_root(&app).join("invoices/tpl/modern.html").exists());

    // A save on a starter not copied in: no such file.
    let (status, _) = json_of(
        &app,
        with_body(
            "PUT",
            "/api/templates/invoices/tpl/modern.html",
            &json!({"hash": "x", "fields": {"name": "Mine"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Duplicate a starter: its body, a new name, its kinds.
    let (status, copy) = json_of(
        &app,
        post_json(
            "/api/templates",
            &json!({"name": "Modern, mine", "from": "/invoices/tpl/modern.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{copy}");
    let text = read(&app, "invoices/tpl/modern-mine.html");
    let shipped = crate::starters::STARTERS[1].source;
    assert_eq!(
        text.split_once('\n').unwrap().1,
        shipped.split_once('\n').unwrap().1
    );
    assert!(text.starts_with("{# invoicer: name=\"Modern, mine\" kinds=\""));

    // Customize: the starter copied in as it ships; then it is editable.
    let (status, done) = json_of(
        &app,
        post_json(
            "/api/templates/customize",
            &json!({"template": "/invoices/tpl/modern.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["path"], "/invoices/tpl/modern.html");
    assert_eq!(done["written"], true);
    assert_eq!(read(&app, "invoices/tpl/modern.html"), shipped);
    let (_, modern) = json_of(&app, authed("/api/templates/invoices/tpl/modern.html")).await;
    assert_eq!(modern["read_only"], false);
    assert_eq!(modern["starter"], true);
    assert!(modern["hash"].is_string());

    let (status, refused) = json_of(
        &app,
        post_json(
            "/api/templates/customize",
            &json!({"template": "/invoices/tpl/myco-default.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "not-a-starter");
}

/// Books without the base (nor a child of it): a new template brings it;
/// a preview before then folds the text into the shipped base, or says the base is missing
/// when it can't (`super()`).
#[tokio::test(flavor = "multi_thread")]
async fn without_the_base_a_preview_folds_and_a_new_template_brings_it() {
    let app = app();
    let root = books_root(&app);
    std::fs::remove_file(root.join(BASE_FILE)).unwrap();
    // MBR registers every sibling, and one extending a missing file fails
    // the folder's inheritance check (PLAN Q1): the fixture's child goes too.
    std::fs::remove_file(root.join("invoices/tpl/myco-default.html")).unwrap();
    let answer = preview(
        &app,
        "/invoices/tpl/classic.html",
        crate::starters::STARTERS[0].source,
    )
    .await;
    assert!(answer["url"].is_string(), "{answer}");
    let answer = preview(
        &app,
        "/invoices/tpl/x.html",
        "{% extends \"_invoice_base.v1.html\" %}\n{% block theme %}{{ super() }}{% endblock theme %}\n",
    )
    .await;
    assert_eq!(answer["url"], Value::Null);
    assert!(
        answer["problem"]["message"]
            .as_str()
            .unwrap()
            .contains("isn't in the templates folder yet"),
        "{answer}"
    );
    assert!(!root.join(BASE_FILE).exists(), "a preview writes nothing");

    let (status, made) =
        json_of(&app, post_json("/api/templates", &json!({"name": "Plain"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["base"], "invoices/tpl/_invoice_base.v1.html");
    assert_eq!(
        std::fs::read_to_string(root.join(BASE_FILE)).unwrap(),
        crate::starters::BASE.source
    );
}
