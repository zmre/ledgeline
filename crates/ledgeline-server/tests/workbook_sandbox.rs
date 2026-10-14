//! The workbook worker: spreadsheet parsing in a resource-limited child process.
//!
//! Two halves. The first drives the real binary as a worker directly — the
//! hidden argument, the upload on stdin, JSON on stdout — so the child's own
//! limits are tested with nothing in between, including the memory budget that
//! has to stand in for `RLIMIT_AS` on macOS. The second drives the stage route
//! in-process and checks the *parent's* half: a worker that dies is a `400`, and
//! the server is still there for the next upload.
//!
//! `import_cli.rs` has the third door, `ledgeline import`, where the binary is
//! both parent and worker.

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::{fixtures_dir, shared_strings_bomb_xlsx, sparse_bomb_xlsx};
use http_body_util::BodyExt;
use ledgeline::{AppState, router_with_state};
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tower::ServiceExt;

/// The worker's hidden argument. Spelled out rather than imported: it is not
/// public API, and this is the test that it stays out of `--help`.
const WORKER_ARG: &str = "--ledgeline-internal-workbook-worker";

const FILENAME: &str = "x-ledgeline-filename";

fn spreadsheet_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join("import/spreadsheet").join(name))
        .unwrap_or_else(|error| panic!("fixture {name} should read: {error}"))
}

/// Run the built binary as a worker for `job`, with `input` on its stdin.
fn worker(job: &str, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ledgeline"))
        .args([WORKER_ARG, job])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the ledgeline binary starts");
    // A worker that dies on open stops reading, so the write may fail; its
    // exit status is the thing under test.
    let _ = child.stdin.take().expect("stdin is piped").write_all(input);
    child.wait_with_output().expect("the worker is reaped")
}

fn answer(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "the worker failed: {:?}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("the worker answers in JSON")
}

// ---------------------------------------------------------------------------
// The worker itself
// ---------------------------------------------------------------------------

#[test]
fn the_worker_argument_is_not_in_the_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_ledgeline"))
        .arg("--help")
        .output()
        .expect("the binary runs");
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("workbook-worker"));
}

#[test]
fn a_worker_reads_a_real_workbook_and_answers_on_stdout() {
    let output = worker("convert:xlsx", &spreadsheet_fixture("simple.xlsx"));
    let answer = answer(&output);
    assert_eq!(
        answer["Ok"]["header"],
        serde_json::json!(["Date", "Description", "Amount", "Balance"]),
        "{answer}"
    );
    assert_eq!(answer["Ok"]["rows"][0][0], "2026-01-05", "{answer}");
}

#[test]
fn a_worker_with_an_unknown_job_refuses_it() {
    let output = worker("convert:docx", b"anything");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn the_sparse_bomb_is_skipped_by_the_capped_reader_inside_the_worker() {
    // The caps refuse the sheet before anything dense exists, so this is not
    // even a crash: the worker answers, and the answer is that there is no
    // statement in the file.
    let started = Instant::now();
    let output = worker("convert:xlsx", &sparse_bomb_xlsx());
    assert_eq!(answer(&output), serde_json::json!({ "Err": "NoTable" }));
    assert!(started.elapsed() < Duration::from_secs(30));
}

#[test]
fn a_worker_over_its_memory_budget_dies_rather_than_allocating() {
    // ~240 GB asked for on open. This is the test of the allocator budget: on
    // macOS `setrlimit(RLIMIT_AS)` cannot be lowered to anything useful, so
    // without the budget this request would be handed to the system allocator.
    let started = Instant::now();
    let output = worker("convert:xlsx", &shared_strings_bomb_xlsx());
    assert!(
        !output.status.success(),
        "the worker must not have survived: {:?}",
        output.status
    );
    assert!(output.stdout.is_empty(), "and must not have answered");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "it dies at the oversized request, not after a struggle"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            output.status.signal(),
            Some(6),
            "a refused allocation is an abort (SIGABRT)"
        );
    }
}

// ---------------------------------------------------------------------------
// The parent: the stage route
// ---------------------------------------------------------------------------

const OPENING: &str = "2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

fn state() -> (TempDir, AppState) {
    common::use_built_workbook_worker();
    let dir = TempDir::new().expect("temp dir");
    std::fs::write(dir.path().join("main.journal"), OPENING).expect("write journal");
    let state = AppState::from_journal_path(dir.path().join("main.journal"))
        .expect("the scratch journal opens");
    (dir, state)
}

async fn upload(state: &AppState, name: &str, bytes: Vec<u8>) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri("/api/import/stage")
        .header(HeaderName::from_static(FILENAME), name)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("request builds");
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn bomb_uploads_are_refused_and_the_server_carries_on() {
    let (_dir, state) = state();

    let (status, body) = upload(&state, "bomb.xlsx", sparse_bomb_xlsx()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains("no worksheet contained tabular data"),
        "{body}"
    );

    // QuickBooks detection runs first on every upload and opens the workbook
    // too, so this crosses the worker twice and must be refused by both.
    let (status, body) = upload(&state, "bomb.xlsx", shared_strings_bomb_xlsx()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains("malformed xlsx file")
            && body.contains("could not be read within the memory and time"),
        "{body}"
    );

    // Still here, and still reading workbooks.
    let (status, body) = upload(&state, "simple.xlsx", spreadsheet_fixture("simple.xlsx")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("GROCERY STORE"), "{body}");
}

#[tokio::test]
async fn every_spreadsheet_format_still_converts_through_the_worker() {
    let (_dir, state) = state();
    for name in ["simple.xlsx", "legacy.xls", "sheet.ods", "multi-sheet.xlsx"] {
        let (status, body) = upload(&state, name, spreadsheet_fixture(name)).await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
    }
}
