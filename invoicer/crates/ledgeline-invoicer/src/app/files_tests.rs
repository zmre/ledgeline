//! The file routes (contracts, Phase 4 feedback J5) and the sidebar's
//! counts in `GET /api/books` (J11), in-process over a private fixture copy.

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{Value, json};

use invoicer_core::attachment::Attachment;

use super::native::Native;
use super::native::mock::MockNative;
use super::tests::{App, app_on, authed, books_root, json_of, post_json};

fn mocked() -> (App, Arc<MockNative>) {
    let native = Arc::new(MockNative::everything());
    let app = app_on("accrual", Some(Arc::clone(&native) as Arc<dyn Native>));
    (app, native)
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_books_answer_every_list_empty() {
    // invoicer/fixtures/empty (H22): the e2e empty states' server side.
    let app = app_on("empty", None);
    let (status, books) = json_of(&app, authed("/api/books?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::OK, "{books}");
    assert_eq!(
        books["invoices"],
        json!({"all": 0, "drafts": 0, "open": 0, "overdue": 0})
    );
    assert_eq!(books["customers"], 0);
    assert_eq!(books["catalog"], 0);
    assert_eq!(books["snippets"], 0);
    // No templates of their own: the starters (4d's list).
    assert_eq!(books["templates"], crate::starters::STARTERS.len());
    let (status, templates) = json_of(&app, authed("/api/templates")).await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    assert!(
        templates
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["installed"] == false),
        "{templates}"
    );
    assert_eq!(books["nextNumber"], "1001");
    assert_eq!(books["nextNumberAfter"], Value::Null);
    assert_eq!(books["nextNumberSource"], "start");
    for (url, empty) in [
        ("/api/invoices", json!([])),
        ("/api/customers", json!([])),
        ("/api/snippets", json!([])),
    ] {
        let (status, body) = json_of(&app, authed(url)).await;
        assert_eq!((status, body), (StatusCode::OK, empty), "{url}");
    }
    let (status, catalog) = json_of(&app, authed("/api/catalog")).await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    assert_eq!(catalog["catalogs"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_books_carry_their_name_and_the_sidebars_counts() {
    let app = super::tests::app();
    let (status, books) = json_of(&app, authed("/api/books?today=2026-11-22")).await;
    assert_eq!(status, StatusCode::OK, "{books}");
    // No leading comment that reads as a title: the folder's name.
    assert_eq!(books["title"], "books");
    assert_eq!(books["journal"], "books.journal");
    assert_eq!(books["customers"], 6);
    let invoices = &books["invoices"];
    assert!(invoices["all"].as_u64().is_some_and(|n| n >= 8), "{books}");
    assert!(
        invoices["drafts"].as_u64().is_some_and(|n| n >= 1),
        "{books}"
    );
    assert!(
        invoices["overdue"].as_u64().is_some_and(|n| n >= 1),
        "{books}"
    );
    assert!(
        invoices["open"].as_u64() >= invoices["overdue"].as_u64(),
        "{books}"
    );
    assert!(books["catalog"].as_u64().is_some_and(|n| n > 0), "{books}");
    assert_eq!(books["snippets"], 2);
    // MyCo default, and the starters not copied in.
    assert_eq!(books["templates"], 1 + crate::starters::STARTERS.len());
    // The next number says what it follows (H19): 2004, booked only.
    assert_eq!(books["nextNumber"], "2005");
    assert_eq!(books["nextNumberAfter"], "2004");
    assert_eq!(books["nextNumberSource"], "books");
    let (status, _) = json_of(&app, authed("/api/books?today=soon")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_lists_notes_and_documents_a_contract_may_name() {
    let app = super::tests::app();
    let root = books_root(&app);
    std::fs::write(root.join("customers/contracts/SOW 4.pdf"), b"%PDF-1.7\n").expect("writes");
    let (status, body) = json_of(&app, authed("/api/files")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // No native side: the searchable list only.
    assert_eq!(body["pick"], false);
    assert_eq!(body["open"], false);
    let files = body["files"].as_array().expect("files");
    let find = |path: &str| files.iter().find(|f| f["path"] == path);
    let msa = find("customers/contracts/Customer X MSA 2026.md").expect("the MSA note");
    assert_eq!(msa["name"], "Customer X MSA 2026");
    assert_eq!(msa["entry"], "[[Customer X MSA 2026]]");
    let sow = find("customers/contracts/SOW 4.pdf").expect("the PDF");
    assert_eq!(sow["name"], "SOW 4.pdf");
    assert_eq!(sow["kind"], Value::Null);
    assert_eq!(sow["entry"], "/customers/contracts/SOW 4.pdf");
    // Invoices aren't contracts; the journal isn't a document.
    assert!(
        files
            .iter()
            .all(|f| f["kind"] != "invoice"
                && !f["path"].as_str().unwrap_or("").ends_with(".journal")),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_open_dialog_answers_a_file_in_the_books_and_refuses_one_outside() {
    let (app, native) = mocked();
    let root = books_root(&app);
    let (_, body) = json_of(&app, authed("/api/files?list=false")).await;
    assert_eq!(body["pick"], true);
    assert_eq!(body["open"], true);
    assert_eq!(body["files"], json!([]));

    // Cancelled.
    let (status, body) = json_of(&app, post_json("/api/files/pick", &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["cancelled"], true);
    let started = native.picked_from.lock().unwrap()[0].clone();
    assert_eq!(started, root.canonicalize().expect("canonical"));

    // A PDF in the books: its `/`-rooted path.
    let pdf = root.join("customers/contracts/SOW 5.pdf");
    std::fs::write(&pdf, b"%PDF-1.7\n").expect("writes");
    *native.pick.lock().unwrap() = Some(pdf);
    let (status, body) = json_of(&app, post_json("/api/files/pick", &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["file"]["entry"], "/customers/contracts/SOW 5.pdf");
    assert_eq!(body["file"]["name"], "SOW 5.pdf");

    // A note: its `[[Title]]`.
    *native.pick.lock().unwrap() = Some(root.join("customers/contracts/Customer X MSA 2026.md"));
    let (_, body) = json_of(&app, post_json("/api/files/pick", &json!({}))).await;
    assert_eq!(body["file"]["entry"], "[[Customer X MSA 2026]]");

    // Outside the books folder (a hidden folder is fine there): its
    // `~/` path or `file:///` URL, and where it is.
    let elsewhere = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(elsewhere.path().join(".private")).expect("dir");
    let outside = elsewhere.path().join(".private/Contract #1.pdf");
    std::fs::write(&outside, b"%PDF-1.7\n").expect("writes");
    *native.pick.lock().unwrap() = Some(outside.clone());
    let (status, body) = json_of(&app, post_json("/api/files/pick", &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["file"]["outside"], true, "{body}");
    assert_eq!(body["file"]["name"], "Contract #1.pdf");
    let entry = body["file"]["entry"].as_str().expect("entry");
    let Ok(Attachment::Outside(place)) = Attachment::parse(entry) else {
        panic!("{entry} reads as an outside file");
    };
    let home = invoicer_core::attachment::home_dir().map(|h| h.canonicalize().unwrap_or(h));
    assert_eq!(
        place.expand(home.as_deref()),
        Some(outside.canonicalize().expect("canonical")),
        "{entry}"
    );
    assert_eq!(body["file"]["path"], place.location());

    // A hidden folder in the books is still refused.
    std::fs::create_dir_all(root.join(".hidden")).expect("dir");
    std::fs::write(root.join(".hidden/x.pdf"), b"%PDF-").expect("writes");
    *native.pick.lock().unwrap() = Some(root.join(".hidden/x.pdf"));
    let (status, body) = json_of(&app, post_json("/api/files/pick", &json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

/// A `file:///` URL for `path`, as a pick writes it.
fn url_of(path: &std::path::Path) -> String {
    Attachment::outside(&path.canonicalize().expect("canonical"), None).entry()
}

#[tokio::test(flavor = "multi_thread")]
async fn open_shows_an_outside_file_only_when_a_customer_names_it() {
    let (app, native) = mocked();
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let dir = elsewhere.path().canonicalize().expect("canonical");
    for name in ["MSA 2026.pdf", "Unnamed.pdf", "PO 9.pdf", "Tool.command"] {
        std::fs::write(dir.join(name), b"%PDF-1.7\n").expect("writes");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("Tool.command"), dir.join("Looks like.pdf"))
        .expect("links");
    let (status, made) = json_of(
        &app,
        post_json(
            "/api/customers",
            &json!({
                "title": "Outside Co",
                "kind": "organization",
                "contracts": [url_of(&dir.join("MSA 2026.pdf")), url_of(&dir.join("Looks like.pdf"))],
                "open_pos": [{"reference": "PO 9", "attachment": url_of(&dir.join("PO 9.pdf"))}],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let open = |path: String| {
        let app = &app;
        async move { json_of(app, post_json("/api/files/open", &json!({ "path": path }))).await }
    };
    // Named by a contract, and by a PO's attachment: opened.
    let (status, body) = open(url_of(&dir.join("MSA 2026.pdf"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revealed"], false);
    assert_eq!(
        native.opened_files.lock().unwrap().last(),
        Some(&dir.join("MSA 2026.pdf"))
    );
    let (status, _) = open(url_of(&dir.join("PO 9.pdf"))).await;
    assert_eq!(status, StatusCode::OK);
    // A link whose target isn't a document: shown in Finder, never run.
    let (status, body) = open(url_of(&dir.join("Looks like.pdf"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revealed"], true, "{body}");
    // Not named by any customer: refused, though it is there.
    let opened = native.opened_files.lock().unwrap().len();
    let (status, body) = open(url_of(&dir.join("Unnamed.pdf"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // The file a named link leads to is the same file: named, so shown,
    // and still only in Finder.
    let (status, body) = open(url_of(&dir.join("Tool.command"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revealed"], true, "{body}");
    // Not there at all.
    let (status, _) = open("file:///nowhere/at/all.pdf".to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(native.opened_files.lock().unwrap().len(), opened);

    // The customer page: outside, found, where it is.
    let (status, detail) = json_of(&app, authed("/api/customers/Outside%20Co")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let msa = &detail["contracts"][0];
    assert_eq!(msa["kind"], "outside", "{detail}");
    assert_eq!(msa["name"], "MSA 2026.pdf");
    assert_eq!(
        msa["location"],
        dir.join("MSA 2026.pdf").display().to_string()
    );
    assert!(msa["found"].is_string(), "{detail}");
    assert_eq!(detail["po_usage"][0]["attachment"]["name"], "PO 9.pdf");
    assert_eq!(
        detail["form"]["open_pos"][0]["attachment"],
        url_of(&dir.join("PO 9.pdf"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_shows_a_books_file_in_its_viewer_and_nothing_else() {
    let (app, native) = mocked();
    let root = books_root(&app).canonicalize().expect("canonical");
    let (status, body) = json_of(
        &app,
        post_json(
            "/api/files/open",
            &json!({"path": "/customers/contracts/Customer X MSA 2026.md"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        native.opened_files.lock().unwrap()[0],
        root.join("customers/contracts/Customer X MSA 2026.md")
    );
    assert_eq!(body["revealed"], false);
    // SEC-5: a file that isn't a document is shown in Finder, never run.
    let contracts = books_root(&app).join("customers/contracts");
    for name in [
        "MSA.command",
        "Setup.terminal",
        "run.sh",
        "noextension",
        "page.html",
    ] {
        std::fs::write(contracts.join(name), "echo hi\n").expect("writes");
        let (status, body) = json_of(
            &app,
            post_json(
                "/api/files/open",
                &json!({"path": format!("customers/contracts/{name}")}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        assert_eq!(body["revealed"], true, "{name}");
        assert_eq!(
            native.revealed_files.lock().unwrap().last(),
            Some(&root.join("customers/contracts").join(name))
        );
    }
    std::fs::write(contracts.join("Scan.PDF"), "%PDF-1.7\n").expect("writes");
    let (_, body) = json_of(
        &app,
        post_json(
            "/api/files/open",
            &json!({"path": "customers/contracts/Scan.PDF"}),
        ),
    )
    .await;
    assert_eq!(body["revealed"], false, "{body}");
    assert_eq!(native.opened_files.lock().unwrap().len(), 2);
    assert_eq!(native.revealed_files.lock().unwrap().len(), 5);
    for (path, wanted) in [
        ("customers/contracts/nope.pdf", StatusCode::NOT_FOUND),
        ("../outside.pdf", StatusCode::BAD_REQUEST),
    ] {
        let (status, body) =
            json_of(&app, post_json("/api/files/open", &json!({"path": path}))).await;
        assert_eq!(status, wanted, "{path}: {body}");
    }
    // Without a window: refused.
    let plain = super::tests::app();
    let (status, _) = json_of(
        &plain,
        post_json("/api/files/open", &json!({"path": "customers/Contoso.md"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
