//! Apply payment's routes (cash books, I17): `GET` and `POST
//! /api/invoices/{num}/payments`, in-process over a private copy of the cash
//! fixture with two imported deposits added, as Ledgeline's importer writes
//! them. The CLI's `apply-payment` runs the same `InvoiceBooks` calls; its
//! own tests (tests/cli.rs) cover the undo.

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::tests::{App, app, app_on_prepared, authed, books_root, json_of, post_json};

const TODAY: &str = "2026-11-17";
const INV_207: &str = "invoices/2026-11-16 Customer X Invoice 207.md";

/// Two imports, as Ledgeline writes them: a part payment for 207, and one
/// larger than anything due (never a candidate).
const IMPORTS: &str = "
2026-11-17 * WIRE CUSTOMER X INV 207  ; id: wx-7
    assets:bank:checking   $3,500.00
    income:unknown        $-3,500.00

2026-11-17 * WIRE CUSTOMER X LARGE  ; id: wx-9
    assets:bank:checking   $9,000.00
    income:unknown        $-9,000.00
";

fn cash_with_imports() -> App {
    app_on_prepared("cash", None, |root| {
        let journal = root.join("books.journal");
        let text = std::fs::read_to_string(&journal).expect("reads");
        std::fs::write(&journal, format!("{text}{IMPORTS}")).expect("writes");
    })
}

fn read(app: &App, file: &str) -> String {
    std::fs::read_to_string(books_root(app).join(file)).expect("reads")
}

async fn options(app: &App, number: &str) -> (StatusCode, Value) {
    json_of(
        app,
        authed(&format!("/api/invoices/{number}/payments?today={TODAY}")),
    )
    .await
}

async fn post(app: &App, number: &str, body: &Value) -> (StatusCode, Value) {
    json_of(
        app,
        post_json(&format!("/api/invoices/{number}/payments"), body),
    )
    .await
}

/// List, preview (nothing written), apply (the deposit re-split, the mirror
/// and the undo record follow), and the applied deposit is no candidate any
/// more.
#[tokio::test(flavor = "multi_thread")]
async fn apply_a_payment_after_previewing_it() {
    let app = cash_with_imports();
    let journal = read(&app, "books.journal");

    let (status, listed) = options(&app, "207").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["remaining"], "$7,000.00");
    assert_eq!(listed["total"], "$7,000.00");
    let candidates = listed["candidates"].as_array().expect("candidates");
    let descriptions: Vec<&str> = candidates
        .iter()
        .filter_map(|c| c["description"].as_str())
        .collect();
    assert_eq!(descriptions, ["WIRE CUSTOMER X INV 207"], "{listed}");
    let wire = &candidates[0];
    assert_eq!(wire["received"], "$3,500.00");
    assert_eq!(
        (wire["exact"].clone(), wire["by_number"].clone()),
        (json!(false), json!(true))
    );
    assert!(
        wire.get("text").is_none(),
        "no journal text on the wire: {wire}"
    );
    let reference = wire["reference"].as_str().expect("reference").to_string();

    // The dry run: the split as the apply writes it, nothing written.
    let (status, preview) = post(
        &app,
        "207",
        &json!({"reference": reference, "dry_run": true, "today": TODAY}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(
        read(&app, "books.journal"),
        journal,
        "a preview writes nothing"
    );
    let split = json!([
        {"account": "revenue:consulting", "amount": "$1,500.00"},
        {"account": "revenue:consulting:delivery", "amount": "$2,000.00"},
    ]);
    assert_eq!(
        preview,
        json!({
            "written": false, "number": "207", "date": "2026-11-17",
            "description": "WIRE CUSTOMER X INV 207", "received": "$3,500.00",
            "applied": split, "partial": true, "remaining": "$3,500.00", "warnings": [],
        })
    );

    // Apply it: the same split, written; 207's `paid:` follows.
    let (status, applied) = post(
        &app,
        "207",
        &json!({"reference": reference, "today": TODAY}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["written"], true);
    assert_eq!(applied["applied"], split);
    assert_eq!(applied["remaining"], "$3,500.00");
    assert_eq!(applied["warnings"], json!([]));
    let undo_file = applied["undo_file"].as_str().expect("an undo record");
    assert!(books_root(&app).join(undo_file).exists(), "{undo_file}");
    let written = read(&app, "books.journal");
    assert!(
        written.contains(
            "2026-11-17 * WIRE CUSTOMER X INV 207  ; id: wx-7, invoice:207\n    \
             assets:bank:checking   $3,500.00\n"
        ),
        "{written}"
    );
    assert!(
        read(&app, INV_207).contains("paid: partial 2026-11-17 $3,500\n"),
        "the mirror follows"
    );
    let (_, panel) = json_of(&app, authed("/api/invoices/207/books")).await;
    assert_eq!(panel["settlement"]["state"], "partial", "{panel}");

    // Applied, it is no candidate: listed no more, and refused.
    let (_, listed) = options(&app, "207").await;
    assert_eq!(listed["remaining"], "$3,500.00");
    assert_eq!(listed["candidates"], json!([]));
    let (status, again) = post(
        &app,
        "207",
        &json!({"reference": reference, "today": TODAY}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{again}");
    assert_eq!(again["code"], "refused");
    assert_eq!(read(&app, "books.journal"), written);
}

/// A reference that names no candidate (unknown, or a deposit more than is
/// due) is refused, dry or not, and nothing is written.
#[tokio::test(flavor = "multi_thread")]
async fn a_deposit_that_is_not_a_candidate_is_refused() {
    let app = cash_with_imports();
    let journal = read(&app, "books.journal");
    for dry_run in [true, false] {
        let (status, refused) = post(
            &app,
            "207",
            &json!({"reference": "0123456789abcdef", "dry_run": dry_run, "today": TODAY}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["code"], "refused", "{refused}");
        assert!(
            refused["error"]
                .as_str()
                .expect("message")
                .contains("is not a deposit that can pay invoice 207"),
            "{refused}"
        );
    }
    assert_eq!(read(&app, "books.journal"), journal);

    // A draft takes no payment; a bad body is a 400.
    let (status, draft) = options(&app, "204").await;
    assert_eq!(draft["code"], "status", "{status} {draft}");
    let (status, bad) = post(&app, "207", &json!({"txn": "abc"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
}

/// Accrual books link payments in Ledgeline: no Apply payment.
#[tokio::test(flavor = "multi_thread")]
async fn accrual_books_refuse_apply_payment() {
    let app = app();
    let journal = read(&app, "books.journal");
    let (status, listed) = options(&app, "106").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{listed}");
    assert_eq!(listed["code"], "refused");
    assert!(
        listed["error"]
            .as_str()
            .expect("message")
            .contains("accrual books"),
        "{listed}"
    );
    for dry_run in [true, false] {
        let (status, refused) = post(
            &app,
            "106",
            &json!({"reference": "0123456789abcdef", "dry_run": dry_run, "today": TODAY}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["code"], "refused", "{refused}");
    }
    assert_eq!(read(&app, "books.journal"), journal);
}
