//! The invoice editor's routes (invoicer PLAN 4a), in process over a copy
//! of the accrual fixture (a child of the router tests, for their helpers).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};

use super::{
    App, HOST, TOKEN, app, app_on, authed, books_root, json_of, post_json, send, session_cookie,
};

fn put_json(path: &str, body: &Value) -> Request<Body> {
    Request::put(path)
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn state_of(app: &App, number: &str) -> Value {
    let (status, state) = json_of(app, authed(&format!("/api/invoices/{number}/form"))).await;
    assert_eq!(status, StatusCode::OK, "{state}");
    state
}

/// A grid row: `cells` by column, for the table `columns` (headers).
fn row(columns: &[Value], cells: &[(&str, &str)]) -> Value {
    Value::Array(
        columns
            .iter()
            .map(|c| {
                let role = c["role"].as_str().unwrap_or_default();
                let text = cells
                    .iter()
                    .find(|(r, _)| *r == role)
                    .map_or("", |(_, t)| *t);
                Value::String(text.to_string())
            })
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_options_offer_what_the_pickers_need() {
    let app = app();
    let (status, options) = json_of(&app, authed("/api/editor/options")).await;
    assert_eq!(status, StatusCode::OK, "{options}");
    let customer_x = options["customers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["title"] == "Customer X")
        .expect("Customer X");
    assert_eq!(customer_x["link"], "[[Customer X]]");
    assert!(
        !customer_x["open_pos"].as_array().unwrap().is_empty(),
        "{customer_x}"
    );
    assert!(
        options["catalog"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "ARCH" && i["rate"] == "$800"),
        "{options}"
    );
    let accounts = options["accounts"].as_array().unwrap();
    assert!(accounts.iter().any(|a| a == "revenue:consulting"));
    assert_eq!(options["deposit_account"], "liabilities:customer-deposits");
    assert!(
        options["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "support_tier"),
        "{options}"
    );
    assert_eq!(options["words"]["subtotal"], "Subtotal");
    assert_eq!(
        options["total_words"],
        json!(["Total due", "Total", "Balance due", "Amount due"])
    );
    assert_eq!(options["subtotal_words"], json!(["Subtotal"]));
    let links = options["link_notes"].as_array().unwrap();
    assert!(
        links.iter().any(|n| n["link"] == "[[Customer X]]"),
        "{options}"
    );
    assert!(
        !links.iter().any(|n| n["kind"] == "invoice"),
        "documents aren't offered: {options}"
    );
    assert_eq!(options["next_number"], "2005");
    assert_eq!(options["next_number_after"], "2004");
    assert_eq!(options["next_number_source"], "books");

    let (status, values) = json_of(&app, authed("/api/fields/support_tier/values")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(values, json!([{"value": "gold", "count": 1}]));
}

/// The 4a e2e path, in process: create a draft, build a table with a
/// discount, a subtotal and a tax row, save it, then send it.
#[tokio::test(flavor = "multi_thread")]
async fn create_edit_save_and_send_an_invoice() {
    let app = app();
    let root = books_root(&app);
    let (status, created) = json_of(
        &app,
        post_json(
            "/api/invoices",
            &json!({"customer": "[[Customer X]]", "date": "2026-11-17", "reference": "PO #PY75334"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["number"], "2005");
    let path = created["path"].as_str().unwrap().to_string();

    let state = state_of(&app, "2005").await;
    assert_eq!(state["status"], "draft");
    let mut form = state["form"].clone();
    assert_eq!(form["customer"], "[[Customer X]]");
    let columns = form["table"]["columns"].as_array().unwrap().clone();
    let rows = vec![
        row(
            &columns,
            &[
                ("description", "Senior architect hours"),
                ("quantity", "10"),
                ("rate", "$800"),
                ("books", "revenue:consulting"),
            ],
        ),
        row(
            &columns,
            &[("description", "Friends discount"), ("rate", "-10%")],
        ),
        row(&columns, &[("rate", "**Subtotal**")]),
        row(
            &columns,
            &[
                ("description", "Sales tax"),
                ("rate", "8.25%"),
                ("books", "liabilities:sales-tax"),
            ],
        ),
        row(&columns, &[("rate", "**Total due**")]),
    ];
    form["table"]["rows"] = Value::Array(rows);
    form["prose_before"] = json!("Thanks for the work this month.");

    let (status, computed) = json_of(
        &app,
        post_json(
            "/api/invoices/2005/compute",
            &json!({"form": form, "books": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{computed}");
    let amounts: Vec<&str> = computed["table"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["amount"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        amounts,
        ["$8,000.00", "-$800.00", "$7,200.00", "$594.00", "$7,794.00"]
    );
    assert_eq!(computed["table"]["rows"][1]["applies_to"], 1);
    assert_eq!(computed["table"]["rows"][3]["applies_to"], 3);
    assert_eq!(computed["total"], "$7,794.00");
    assert_eq!(computed["changed"], true);
    let would = computed["books"]["would_book"].as_str().unwrap_or_default();
    assert!(would.contains("invoice: 2005"), "{computed}");
    assert!(would.contains("liabilities:sales-tax"), "{would}");
    // Computing wrote nothing.
    let before = std::fs::read_to_string(root.join(&path)).unwrap();
    assert!(!before.contains("Sales tax"));

    let (status, saved) = json_of(
        &app,
        put_json(
            "/api/invoices/2005/form",
            &json!({"form": form, "revision": state["revision"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["written"], true);
    let text = std::fs::read_to_string(root.join(&path)).unwrap();
    assert!(text.contains("**$7,794.00**"), "{text}");
    assert!(text.contains("Thanks for the work this month."), "{text}");
    assert_eq!(state_of(&app, "2005").await["revision"], saved["revision"]);

    let (status, sent) = json_of(
        &app,
        post_json(
            "/api/invoices/2005/send",
            &json!({"via": "email", "today": "2026-11-17"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let journal = std::fs::read_to_string(root.join("books.journal")).unwrap();
    let booked = &journal[journal.find("invoice: 2005").expect("booked")..];
    for line in [
        "assets:accounts receivable",
        "$7794.00",
        "revenue:consulting",
        "liabilities:sales-tax",
        "$-594.00",
    ] {
        assert!(booked.contains(line), "{line} in {booked}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_outside_edit_is_never_clobbered() {
    let app = app();
    let root = books_root(&app);
    let file = root.join("invoices/2026-11-17 Customer X Invoice 108.md");
    let state = state_of(&app, "108").await;
    let mut form = state["form"].clone();
    form["reference"] = json!("PO #mine");

    let theirs = std::fs::read_to_string(&file)
        .unwrap()
        .replace("Pay by ACH", "Pay by wire");
    std::fs::write(&file, &theirs).unwrap();
    let (status, refused) = json_of(
        &app,
        put_json(
            "/api/invoices/108/form",
            &json!({"form": form, "revision": state["revision"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "changed");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), theirs);

    // "Keep mine": saved over their revision, deliberately.
    let (status, saved) = json_of(
        &app,
        put_json(
            "/api/invoices/108/form",
            &json!({"form": form, "revision": refused["revision"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("PO #mine"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sent_invoice_saves_only_with_its_rebook_and_never_in_a_closed_period() {
    let app = app();
    let root = books_root(&app);
    let journal = || std::fs::read_to_string(root.join("books.journal")).unwrap();
    let state = state_of(&app, "105").await;
    assert_eq!(state["status"], "sent");
    let mut form = state["form"].clone();
    form["reference"] = json!("PO #PY75334 rev. 3");
    let save = |form: &Value, revision: &Value, rebook: bool| {
        put_json(
            "/api/invoices/105/form",
            &json!({"form": form, "revision": revision, "rebook": rebook, "today": "2026-11-17"}),
        )
    };
    let (status, asked) = json_of(&app, save(&form, &state["revision"], false)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{asked}");
    assert_eq!(asked["code"], "confirm-rebook");

    let (status, saved) = json_of(&app, save(&form, &state["revision"], true)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(saved["rebooked"].is_object(), "{saved}");
    assert!(
        journal().contains("reference: PO #PY75334 rev. 3"),
        "{}",
        journal()
    );

    // 106 is dated in the closed period: refused before the file changes.
    let file = root.join("invoices/2026-08-01 Wingtip Invoice 106.md");
    let before = std::fs::read_to_string(&file).unwrap();
    let state = state_of(&app, "106").await;
    let mut form = state["form"].clone();
    form["reference"] = json!("late");
    let (status, refused) = json_of(
        &app,
        put_json(
            "/api/invoices/106/form",
            &json!({"form": form, "revision": state["revision"], "rebook": true}),
        ),
    )
    .await;
    assert!(status.is_client_error(), "{status} {refused}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

    // A void invoice isn't edited.
    let state = state_of(&app, "103").await;
    let (status, void) = json_of(
        &app,
        put_json(
            "/api/invoices/103/form",
            &json!({"form": state["form"], "revision": state["revision"], "rebook": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{void}");
    assert_eq!(void["code"], "void");
}

/// RA-6: an invoice sent outside the app (the CLI's `send`) before the
/// watcher refreshed the index is still saved as sent: re-booked, and
/// through the closed-period check.
#[tokio::test(flavor = "multi_thread")]
async fn a_save_reads_sent_from_the_file_not_a_stale_index() {
    let app = app();
    let root = books_root(&app);
    let journal = || std::fs::read_to_string(root.join("books.journal")).unwrap();
    {
        // Another process sends 108; this app's index still says draft.
        let mut cli = invoicer_core::session::Session::open_local(&root.join("books.journal"))
            .expect("opens");
        let books = cli.invoice_books();
        let (index, _) = cli.parts_mut();
        books
            .send(
                index,
                &invoicer_core::books::Send {
                    number: "108".to_string(),
                    today: "2026-11-17".to_string(),
                    sent_via: None,
                    expected: None,
                },
            )
            .expect("sent");
    }
    let state = state_of(&app, "108").await;
    assert_eq!(state["status"], "sent");
    let mut form = state["form"].clone();
    form["reference"] = json!("PO #PY75334 rev. 2");
    let (status, saved) = json_of(
        &app,
        put_json(
            "/api/invoices/108/form",
            &json!({"form": form, "revision": state["revision"], "rebook": true, "today": "2026-11-17"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(saved["rebooked"].is_object(), "{saved}");
    assert!(
        journal().contains("reference: PO #PY75334 rev. 2"),
        "{}",
        journal()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn void_rebook_deposits_and_a_new_customer() {
    let app = app();
    let root = books_root(&app);

    // Voiding a draft changes only its markdown.
    let journal_before = std::fs::read_to_string(root.join("books.journal")).unwrap();
    let (status, voided) = json_of(
        &app,
        post_json("/api/invoices/108/void", &json!({"today": "2026-11-17"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{voided}");
    assert_eq!(state_of(&app, "108").await["status"], "void");
    assert_eq!(
        std::fs::read_to_string(root.join("books.journal")).unwrap(),
        journal_before
    );

    // Re-booking an invoice in step changes nothing.
    let (status, rebooked) = json_of(
        &app,
        post_json("/api/invoices/105/rebook", &json!({"today": "2026-11-17"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rebooked}");
    assert_eq!(rebooked["changed"], false);

    let (status, deposits) = json_of(&app, authed("/api/invoices/105/deposits")).await;
    assert_eq!(status, StatusCode::OK, "{deposits}");
    assert_eq!(deposits["account"], "liabilities:customer-deposits");
    assert!(deposits["deposits"].is_array());

    // The editor's inline Add new is the customers API (E13, 4b's K3).
    let (status, added) = json_of(
        &app,
        post_json(
            "/api/customers",
            &json!({
                "title": "Acme Corp",
                "kind": "organization",
                "emails": [{"label": "billing", "value": "ap@acme.example"}],
                "terms": 45,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    assert_eq!(added["link"], "[[Acme Corp]]");
    assert!(root.join("customers/Acme Corp.md").exists());
    let (status, again) = json_of(
        &app,
        post_json(
            "/api/customers",
            &json!({"title": "Acme Corp", "kind": "person"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");
    // The 4a route is gone: one way to create a customer.
    let (status, _) = json_of(
        &app,
        post_json(
            "/api/editor/customer",
            &json!({"name": "Other Co", "kind": "organization"}),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert!(!root.join("customers/Other Co.md").exists());

    // The new customer can be invoiced at once, with its terms.
    let (status, created) = json_of(
        &app,
        post_json(
            "/api/invoices",
            &json!({"customer": "Acme Corp", "date": "2026-11-17"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let state = state_of(&app, created["number"].as_str().unwrap()).await;
    assert_eq!(state["form"]["terms"], 45);
    assert_eq!(state["form"]["due_date"], "2027-01-01");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsaved_preview_is_a_page_behind_the_gate() {
    let app = app_on("accrual", None);
    let state = state_of(&app, "108").await;
    let source = state["computed"]["text"]
        .as_str()
        .unwrap()
        .replace("Platform subscription", "Unsaved subscription");
    let (status, draft) = json_of(
        &app,
        post_json(
            "/api/preview/draft",
            &json!({"number": "108", "source": source}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    let url = draft["url"].as_str().unwrap();
    assert!(
        url.starts_with("/.ledgeline-invoicer/preview?draft="),
        "{url}"
    );

    let anonymous = Request::get(url)
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app.router, anonymous).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let cookie = session_cookie(&app).await;
    let page = |url: &str| {
        Request::get(url)
            .header(header::HOST, HOST)
            .header(header::COOKIE, cookie.clone())
            .body(Body::empty())
            .unwrap()
    };
    let (status, _, html) = send(&app.router, page(url)).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("Unsaved subscription"), "{html}");
    let (status, _, _) = send(&app.router, page("/.ledgeline-invoicer/preview?draft=nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// RA-7: a starter that can't be copied in doesn't fail the create: the
/// draft is written and numbered, so the answer says so, with a warning
/// (a 500 invited a retry, and a second draft).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_starter_not_copied_is_a_warning_on_create() {
    use std::os::unix::fs::PermissionsExt as _;
    let app = app();
    let tpl = books_root(&app).join("invoices/tpl");
    let mode = |m: u32| std::fs::set_permissions(&tpl, std::fs::Permissions::from_mode(m));
    mode(0o555).unwrap();
    let (status, created) = json_of(
        &app,
        post_json(
            "/api/invoices",
            &json!({"customer": "[[Customer X]]", "date": "2026-11-17", "template": "/invoices/tpl/classic.html"}),
        ),
    )
    .await;
    mode(0o755).unwrap();
    assert_eq!(status, StatusCode::OK, "{created}");
    assert!(created["number"].is_string(), "{created}");
    assert_eq!(created["copied"], Value::Null);
    let warnings = created["warnings"].as_array().expect("warnings");
    assert_eq!(warnings.len(), 1, "{created}");
    assert!(
        warnings[0]
            .as_str()
            .is_some_and(|w| w.contains("wasn't copied in")),
        "{created}"
    );
    let path = created["path"].as_str().expect("path");
    assert!(books_root(&app).join(path).is_file());
}

/// A starter the editor names is copied in with the base it extends (Q1 =
/// B): on `new` and on a save, never over a base that is there.
#[tokio::test(flavor = "multi_thread")]
async fn the_editor_copies_a_starter_with_its_base() {
    let app = app();
    let root = books_root(&app);
    std::fs::remove_file(root.join(super::BASE_FILE)).unwrap();
    let (status, created) = json_of(
        &app,
        post_json(
            "/api/invoices",
            &json!({"customer": "[[Customer X]]", "date": "2026-11-17", "template": "/invoices/tpl/classic.html"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["copied"], "invoices/tpl/classic.html");
    assert_eq!(
        std::fs::read_to_string(root.join(super::BASE_FILE)).unwrap(),
        crate::starters::BASE.source
    );
    let number = created["number"].as_str().unwrap().to_string();
    let (status, detail) = json_of(&app, authed(&format!("/api/invoices/{number}"))).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert!(!detail.to_string().contains("unknown-template"), "{detail}");

    // A save naming another starter: the user's base stays as it is.
    let mine = "{# the user's base #}\n";
    std::fs::write(root.join(super::BASE_FILE), mine).unwrap();
    let state = state_of(&app, "108").await;
    let mut form = state["form"].clone();
    form["template"] = json!("/invoices/tpl/modern.html");
    let (status, saved) = json_of(
        &app,
        put_json(
            "/api/invoices/108/form",
            &json!({"form": form, "revision": state["revision"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["copied"], "invoices/tpl/modern.html");
    assert_eq!(
        std::fs::read_to_string(root.join(super::BASE_FILE)).unwrap(),
        mine
    );
}

/// New → Deposit invoice: the same `new`, with one row booked to the
/// deposit account (so the editor shows its deposit fields), at zero for
/// the author to fill in; on the cash fixture, whose deposit account is set.
#[tokio::test(flavor = "multi_thread")]
async fn create_a_deposit_invoice() {
    let app = app_on("cash", None);
    let (status, created) = json_of(
        &app,
        post_json(
            "/api/invoices",
            &json!({"customer": "[[Customer X]]", "date": "2026-11-17", "deposit": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["warnings"], json!([]));
    let number = created["number"].as_str().unwrap().to_string();
    let state = state_of(&app, &number).await;
    assert_eq!(state["status"], "draft");
    let table = &state["form"]["table"];
    let columns = table["columns"].as_array().unwrap();
    let at = |role: &str| {
        columns
            .iter()
            .position(|c| c["role"] == role)
            .unwrap_or_else(|| panic!("{role}: {table}"))
    };
    let rows = table["rows"].as_array().unwrap();
    let deposit = rows
        .iter()
        .find(|r| r[at("books")] == "liabilities:customer-deposits")
        .unwrap_or_else(|| panic!("a deposit row: {table}"));
    assert_eq!(deposit[at("description")], "Deposit");
    assert_eq!(deposit[at("amount")], "$0.00");

    // Without `deposit`, no row books to the deposit account.
    let (_, plain) = json_of(
        &app,
        post_json(
            "/api/invoices",
            &json!({"customer": "[[Customer X]]", "date": "2026-11-17"}),
        ),
    )
    .await;
    let state = state_of(&app, plain["number"].as_str().unwrap()).await;
    assert!(
        !state["form"]["table"]
            .to_string()
            .contains("customer-deposits"),
        "{state}"
    );
}
