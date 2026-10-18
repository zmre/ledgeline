//! `LedgelineApp` driven the way the Swift shell drives it: requests through
//! [`LedgelineApp::handle`], awaited from an executor that is NOT tokio
//! (`futures::executor::block_on`, standing in for UniFFI's), over a
//! [`MemVault`] journal — and once over a real folder with a coordinator.

use std::sync::Arc;

use futures::executor::block_on;
use ledgeline::AppDirs;
use ledgeline_core::vault::{Availability, MemVault, Vault, VaultPath};
use serde_json::{Value, json};

use super::*;
use crate::coordinated_vault::tests::FakeCoordinator;

const JOURNAL: &str = "\
2026-01-01 opening balances
    assets:checking    $1000.00
    equity:opening

2026-01-05 coffee
    expenses:food    $4.50
    assets:checking
";

fn vp(s: &str) -> VaultPath {
    VaultPath::new(s).unwrap()
}

struct Fixture {
    mem: Arc<MemVault>,
    app: Arc<LedgelineApp>,
    _dirs: tempfile::TempDir,
}

fn fixture_with(mem: MemVault) -> Fixture {
    let dirs = tempfile::tempdir().unwrap();
    let mem = Arc::new(mem);
    let app = LedgelineApp::with_vault(
        Arc::clone(&mem) as Arc<dyn Vault>,
        vp("main.journal"),
        AppDirs::under(dirs.path()),
        false,
    )
    .unwrap();
    Fixture {
        mem,
        app,
        _dirs: dirs,
    }
}

fn fixture() -> Fixture {
    fixture_with(MemVault::new().with_file("main.journal", JOURNAL))
}

struct Reply {
    status: u16,
    headers: Vec<Header>,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }
}

/// One request, as the scheme handler would make it, with the token unless
/// `authorized` is false.
fn send(
    app: &LedgelineApp,
    method: &str,
    path: &str,
    body: Option<Value>,
    authorized: bool,
) -> Reply {
    let mut headers = vec![Header::new("Accept", "application/json")];
    if authorized {
        headers.push(Header::new(
            "Authorization",
            &format!("Bearer {}", app.access_token()),
        ));
    }
    let bytes = body.map_or_else(Vec::new, |b| {
        headers.push(Header::new("Content-Type", "application/json"));
        serde_json::to_vec(&b).unwrap()
    });
    block_on(async {
        let response = app
            .handle(
                method.to_string(),
                format!("{MOBILE_ORIGIN}{path}"),
                headers,
                bytes,
            )
            .await
            .unwrap();
        Reply {
            status: response.status(),
            headers: response.headers(),
            body: response.read_all().await.unwrap(),
        }
    })
}

fn get(app: &LedgelineApp, path: &str) -> Reply {
    send(app, "GET", path, None, true)
}

fn transaction_count(app: &LedgelineApp) -> usize {
    let reply = get(app, "/transactions");
    assert_eq!(reply.status, 200);
    reply.json().as_array().unwrap().len()
}

fn new_transaction() -> Value {
    json!({
        "date": "2026-02-01",
        "description": "groceries",
        "postings": [
            { "account": "expenses:food",
              "amount": { "commodity": "$", "quantity": { "mantissa": "2500", "places": 2 } } },
            { "account": "assets:checking" }
        ]
    })
}

#[test]
fn every_route_requires_the_token_in_process_too() {
    let f = fixture();
    let refused = send(&f.app, "GET", "/api/journal", None, false);
    assert_eq!(refused.status, 401);
    let journal = get(&f.app, "/api/journal");
    assert_eq!(journal.status, 200);
    assert!(journal.json().is_object());
    // The SPA shell itself is served without one, so it can bootstrap.
    let shell = send(&f.app, "GET", "/", None, false);
    assert_eq!(shell.status, 200);
    assert!(
        shell
            .header("content-type")
            .unwrap()
            .starts_with("text/html")
    );
}

#[test]
fn the_wire_routes_answer_from_the_parsed_journal() {
    let f = fixture();
    let reply = get(&f.app, "/transactions");
    assert_eq!(reply.status, 200);
    let transactions = reply.json();
    let descriptions: Vec<&str> = transactions
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["tdescription"].as_str().unwrap())
        .collect();
    assert_eq!(descriptions, vec!["opening balances", "coffee"]);
    // Never compressed: WebKit would hand the gzip bytes to the page as-is.
    assert_eq!(reply.header("content-encoding"), None);
}

#[test]
fn an_edit_is_written_through_the_vault_and_served_back() {
    let f = fixture();
    let added = send(
        &f.app,
        "POST",
        "/api/transactions",
        Some(new_transaction()),
        true,
    );
    assert_eq!(
        added.status,
        201,
        "{}",
        String::from_utf8_lossy(&added.body)
    );
    assert_eq!(transaction_count(&f.app), 3);
    assert!(f.mem.text("main.journal").unwrap().contains("groceries"));
}

/// The external-change guarantee survives the seam: a file changed behind the
/// app's back is never overwritten by an edit — the edit is a 409.
#[test]
fn an_edit_over_an_external_change_is_a_409() {
    let f = fixture();
    let theirs = format!(
        "{JOURNAL}\n2026-03-01 synced from the desktop\n    expenses:x    $1\n    assets:checking\n"
    );
    f.mem.modify_externally("main.journal", &theirs);
    let refused = send(
        &f.app,
        "POST",
        "/api/transactions",
        Some(new_transaction()),
        true,
    );
    assert_eq!(refused.status, 409);
    assert_eq!(
        f.mem.text("main.journal").unwrap(),
        theirs,
        "theirs survives"
    );
}

/// A journal file evicted by the provider makes an edit a retryable 503 —
/// and asks for the download — rather than a 500.
#[test]
fn an_edit_over_a_not_downloaded_file_is_a_503_with_retry_after() {
    let f = fixture();
    f.mem
        .set_availability("main.journal", Availability::NotDownloaded);
    let reply = send(
        &f.app,
        "POST",
        "/api/transactions",
        Some(new_transaction()),
        true,
    );
    assert_eq!(
        reply.status,
        503,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    assert!(reply.header("retry-after").is_some());
    assert_eq!(f.mem.download_requests(), vec![vp("main.journal")]);
}

#[test]
fn refresh_reloads_only_what_changed_and_waits_out_an_eviction() {
    let f = fixture();
    let unchanged = block_on(f.app.refresh()).unwrap();
    assert_eq!(unchanged.outcome, RefreshOutcome::Unchanged);
    assert!(unchanged.editable);

    let theirs =
        format!("{JOURNAL}\n2026-03-01 synced\n    expenses:x    $1\n    assets:checking\n");
    f.mem.modify_externally("main.journal", &theirs);
    let reloaded = block_on(f.app.refresh()).unwrap();
    assert_eq!(reloaded.outcome, RefreshOutcome::Reloaded);
    assert_eq!(transaction_count(&f.app), 3);

    // Evicted: reported, downloaded, and the editor left alone.
    f.mem
        .set_availability("main.journal", Availability::NotDownloaded);
    let pending = block_on(f.app.refresh()).unwrap();
    assert_eq!(pending.outcome, RefreshOutcome::Pending);
    assert_eq!(pending.not_downloaded, vec!["main.journal".to_string()]);
    assert!(pending.editable, "an eviction must not cost the editor");
    assert!(f.mem.download_requests().contains(&vp("main.journal")));
    assert_eq!(transaction_count(&f.app), 3, "the last snapshot is served");

    f.mem.set_availability("main.journal", Availability::Local);
    let back = block_on(f.app.refresh()).unwrap();
    assert_eq!(back.outcome, RefreshOutcome::Unchanged);
}

/// A synced-in edit that does not parse keeps the last good snapshot, and once
/// it is fixed, editing comes back.
#[test]
fn a_broken_journal_fails_the_refresh_and_a_fixed_one_restores_editing() {
    let f = fixture();
    f.mem
        .modify_externally("main.journal", "2026-13-45 not a date\n    a  $1\n    b\n");
    let failed = block_on(f.app.refresh()).unwrap();
    assert_eq!(failed.outcome, RefreshOutcome::Failed);
    assert!(failed.message.is_some());
    assert_eq!(transaction_count(&f.app), 2, "the last good snapshot");

    let fixed = format!("{JOURNAL}\n2026-03-01 fixed\n    expenses:x    $1\n    assets:checking\n");
    f.mem.modify_externally("main.journal", &fixed);
    let restored = block_on(f.app.refresh()).unwrap();
    assert_eq!(restored.outcome, RefreshOutcome::Reloaded);
    assert!(restored.editable);
    assert!(f.app.is_editable());
    assert_eq!(transaction_count(&f.app), 3);
    let added = send(
        &f.app,
        "POST",
        "/api/transactions",
        Some(new_transaction()),
        true,
    );
    assert_eq!(added.status, 201);
}

#[test]
fn opening_over_a_not_downloaded_include_names_it_and_requests_it() {
    let mem = Arc::new(
        MemVault::new()
            .with_file("main.journal", "include 2025.journal\n")
            .with_file("2025.journal", JOURNAL),
    );
    mem.set_availability("2025.journal", Availability::NotDownloaded);
    let dirs = tempfile::tempdir().unwrap();
    let error = LedgelineApp::with_vault(
        Arc::clone(&mem) as Arc<dyn Vault>,
        vp("main.journal"),
        AppDirs::under(dirs.path()),
        false,
    )
    .err()
    .expect("the open fails");
    let MobileError::NotDownloaded { paths } = error else {
        panic!("expected NotDownloaded, got {error:?}");
    };
    assert_eq!(paths, vec!["2025.journal".to_string()]);
    assert_eq!(mem.download_requests(), vec![vp("2025.journal")]);
}

#[test]
fn a_single_file_with_includes_asks_for_its_folder() {
    let inner = MemVault::new()
        .with_file("main.journal", "include 2025.journal\n")
        .with_file("2025.journal", JOURNAL);
    let single = SingleFileVault::new(inner, "main.journal").unwrap();
    let dirs = tempfile::tempdir().unwrap();
    let error = LedgelineApp::with_vault(
        Arc::new(single),
        vp("main.journal"),
        AppDirs::under(dirs.path()),
        true,
    )
    .err()
    .expect("the open fails");
    assert!(
        matches!(error, MobileError::NeedsFolder { .. }),
        "{error:?}"
    );

    // Without includes it opens, and edits the one file.
    let lone = SingleFileVault::new(
        MemVault::new().with_file("main.journal", JOURNAL),
        "main.journal",
    )
    .unwrap();
    let app = LedgelineApp::with_vault(
        Arc::new(lone),
        vp("main.journal"),
        AppDirs::under(dirs.path()),
        true,
    )
    .unwrap();
    assert!(app.is_single_file());
    assert_eq!(transaction_count(&app), 2);
}

#[test]
fn the_token_script_is_guarded_by_the_mobile_origin() {
    let f = fixture();
    let script = f.app.token_init_script(None);
    assert!(script.contains("\"ledgeline://localhost\""), "{script}");
    assert!(script.contains(&format!("\"{}\"", f.app.access_token())));
    let custom = f.app.token_init_script(Some("app://books".into()));
    assert!(custom.contains("\"app://books\""));
}

/// End to end over a real folder: `open` builds a `CoordinatedVault`, and an
/// edit goes through the coordinator's conditional replace.
#[test]
fn open_over_a_folder_writes_through_the_coordinator() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("main.journal"), JOURNAL).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let coordinator = Arc::new(FakeCoordinator::default());
    let app = LedgelineApp::open(
        folder.path().to_string_lossy().into_owned(),
        "main.journal".into(),
        Some(cache.path().to_string_lossy().into_owned()),
        Some(Arc::clone(&coordinator) as Arc<dyn FileCoordinator>),
    )
    .unwrap();
    assert_eq!(app.main_file(), "main.journal");
    assert_eq!(transaction_count(&app), 2);

    let added = send(
        &app,
        "POST",
        "/api/transactions",
        Some(new_transaction()),
        true,
    );
    assert_eq!(
        added.status,
        201,
        "{}",
        String::from_utf8_lossy(&added.body)
    );
    let calls = coordinator.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].starts_with("replace ") && calls[0].ends_with("main.journal"));
    assert!(
        std::fs::read_to_string(folder.path().join("main.journal"))
            .unwrap()
            .contains("groceries")
    );

    // Another device's edit lands between the app's read and its write: the
    // coordinator's in-block check refuses, and the edit is a 409.
    *coordinator.sneak_in.lock().unwrap() = Some(b"; replaced on another device\n".to_vec());
    let refused = send(
        &app,
        "POST",
        "/api/transactions",
        Some(new_transaction()),
        true,
    );
    assert_eq!(refused.status, 409);
    assert_eq!(
        std::fs::read_to_string(folder.path().join("main.journal")).unwrap(),
        "; replaced on another device\n"
    );
}

#[test]
fn opening_something_that_is_not_there_is_an_open_error() {
    let missing = LedgelineApp::open(
        "/definitely/not/here".into(),
        "x.journal".into(),
        None,
        None,
    );
    assert!(matches!(missing, Err(MobileError::Open { .. })));
    let folder = tempfile::tempdir().unwrap();
    let no_journal = LedgelineApp::open(
        folder.path().to_string_lossy().into_owned(),
        "absent.journal".into(),
        None,
        None,
    );
    assert!(matches!(no_journal, Err(MobileError::Open { .. })));
}
