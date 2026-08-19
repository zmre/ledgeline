//! Golden-byte tests for the NATIVE `/api/*` wire (CLEANUP.md DRY-3).
//!
//! # What this guards
//!
//! The native wire has no schema codegen. 28 `Wire*` structs in
//! `reports_api.rs` are mirrored by hand by 28 `Raw*` interfaces in
//! `web/src/lib/api/nativeDecode.ts`, kept in step by two prose comments
//! pointing at each other. Nothing mechanical connected them, so renaming one
//! Rust field — `inclusive` → `inclusive_total`, say — used to:
//!
//!   * pass `cargo build` (serde just emits the new key),
//!   * pass `cargo test` (no Rust test asserted the key names),
//!   * pass `tsc` and `svelte-check` (every `Raw*` field is optional),
//!   * pass `vitest` (the decoders' samples are hand-typed literals),
//!
//! and then render a balance sheet of `$0.00`, because `decodeMixed` treated an
//! absent money field as an empty `Map`. A whole class of silent-zero bugs with
//! four green gates in front of it.
//!
//! This file closes the Rust half: each endpoint below replays a PINNED request
//! and compares the response BYTE FOR BYTE against a body committed under
//! `fixtures/native/v1/`. `web/src/lib/api/nativeDecode.test.ts` decodes those
//! same files, so a rename now fails on both sides at once.
//!
//! It is the same shape as the hledger-wire guard (`server_endpoints.rs` over
//! `fixtures/api/v1.52/`), which the native wire had no equivalent of.
//!
//! # Byte equality, not semantic equality
//!
//! `fixtures/api/v1.52/` is compared with a key-ignoring semantic comparator
//! because hledger's JSON is somebody else's format and drifts between
//! releases. This wire is OURS: every byte of it is a deliberate choice, so any
//! change to any of them is a contract change that should be seen and reviewed.
//! Byte equality also catches things a `Value` comparison would wave through —
//! a `Dec` mantissa demoted from string to number, `null` vs. an omitted key,
//! a reordered field. When it does fail, [`describe_difference`] re-reads both
//! sides as JSON and names the differing paths, so the message is about a key,
//! not an offset.
//!
//! # Regenerating
//!
//! `just snapshot-native` — but only when the wire contract changed ON PURPOSE.
//! An unexplained diff here is the bug this file exists to catch.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{fixture_journal, fixtures_dir};
use http_body_util::BodyExt;
use ledgeline_server::app;
use serde_json::Value;
use tower::ServiceExt;

/// The committed request set, shared with `just snapshot-native` so a fixture
/// and the request that produced it cannot drift apart.
fn requests() -> Vec<(String, String)> {
    let path = fixtures_dir().join("native/v1/requests.tsv");
    let text = std::fs::read_to_string(&path).expect("native/v1/requests.tsv readable");
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let (name, uri) = line
                .split_once('\t')
                .unwrap_or_else(|| panic!("requests.tsv line is not `name<TAB>uri`: {line:?}"));
            (name.to_string(), uri.to_string())
        })
        .collect()
}

fn uri_for(name: &str) -> String {
    requests()
        .into_iter()
        .find(|(entry, _)| entry == name)
        .unwrap_or_else(|| panic!("requests.tsv has no entry named {name:?}"))
        .1
}

/// `GET uri` against a fresh app over `fixtures/sample.journal`, returning the
/// raw response body bytes (what the SPA's `fetch` actually receives).
async fn body_bytes(uri: &str) -> Vec<u8> {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    let response = app(&fixture_journal())
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK, "GET {uri} should be 200");
    response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec()
}

/// Append a human-readable account of how `actual` differs from `expected`,
/// keyed by JSON path. Renames show up as a paired absent/new key at the same
/// path, which is exactly the failure this suite is for.
fn describe_difference(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
    const MAX_FINDINGS: usize = 20;
    if out.len() >= MAX_FINDINGS {
        return;
    }
    match (expected, actual) {
        (Value::Object(want), Value::Object(got)) => {
            for (key, value) in want {
                match got.get(key) {
                    Some(live) => describe_difference(&format!("{path}.{key}"), value, live, out),
                    None => out.push(format!(
                        "{path}.{key}: in the fixture but ABSENT from the live response \
                         (renamed or dropped?)"
                    )),
                }
            }
            for key in got.keys().filter(|key| !want.contains_key(*key)) {
                out.push(format!("{path}.{key}: NEW in the live response"));
            }
        }
        (Value::Array(want), Value::Array(got)) if want.len() == got.len() => {
            for (i, (value, live)) in want.iter().zip(got).enumerate() {
                describe_difference(&format!("{path}[{i}]"), value, live, out);
            }
        }
        (Value::Array(want), Value::Array(got)) => out.push(format!(
            "{path}: fixture has {} elements, live response has {}",
            want.len(),
            got.len()
        )),
        _ if expected != actual => out.push(format!("{path}: expected {expected}, got {actual}")),
        _ => {}
    }
}

/// Replay one pinned request and require the response bytes to equal the
/// committed golden exactly.
async fn assert_matches_golden(name: &str) {
    let path = fixtures_dir()
        .join("native/v1")
        .join(format!("{name}.json"));
    let expected = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{name}.json readable ({e}) — run `just snapshot-native`"));
    let actual = body_bytes(&uri_for(name)).await;
    if actual == expected {
        return;
    }

    // Bytes differ. Say WHY in terms of keys before falling back to offsets.
    let mut findings = Vec::new();
    match (
        serde_json::from_slice::<Value>(&expected),
        serde_json::from_slice::<Value>(&actual),
    ) {
        (Ok(want), Ok(got)) => describe_difference("$", &want, &got, &mut findings),
        _ => findings.push("one side is not valid JSON".to_string()),
    }
    if findings.is_empty() {
        findings.push(
            "the two bodies are semantically equal but not byte-equal (key order, number \
             formatting, or an omitted-vs-null key changed)"
                .to_string(),
        );
    }
    panic!(
        "the native wire for `{name}` no longer matches fixtures/native/v1/{name}.json:\n  {}\n\n\
         If this change was deliberate, update BOTH sides — regenerate with \
         `just snapshot-native` AND fix the matching `Raw*` interface + decoder in \
         web/src/lib/api/nativeDecode.ts — then re-run vitest. If it was not \
         deliberate, this is the silent-zero bug DRY-3 is about: the SPA would have \
         rendered $0.00 for the renamed field without erroring.",
        findings.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// One test per endpoint, so a failure names the endpoint that broke.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn balancesheet_matches_the_native_golden() {
    assert_matches_golden("balancesheet").await;
}

#[tokio::test]
async fn balancesheet_grouped_matches_the_native_golden() {
    assert_matches_golden("balancesheet-grouped").await;
}

#[tokio::test]
async fn incomestatement_matches_the_native_golden() {
    assert_matches_golden("incomestatement").await;
}

#[tokio::test]
async fn incomestatement_grouped_matches_the_native_golden() {
    assert_matches_golden("incomestatement-grouped").await;
}

#[tokio::test]
async fn cashflow_matches_the_native_golden() {
    assert_matches_golden("cashflow").await;
}

#[tokio::test]
async fn networth_matches_the_native_golden() {
    assert_matches_golden("networth").await;
}

#[tokio::test]
async fn budget_matches_the_native_golden() {
    assert_matches_golden("budget").await;
}

#[tokio::test]
async fn insights_matches_the_native_golden() {
    assert_matches_golden("insights").await;
}

#[tokio::test]
async fn subscriptions_matches_the_native_golden() {
    assert_matches_golden("subscriptions").await;
}

#[tokio::test]
async fn holdings_matches_the_native_golden() {
    assert_matches_golden("holdings").await;
}

#[tokio::test]
async fn holdings_series_matches_the_native_golden() {
    assert_matches_golden("holdings-series").await;
}

// ---------------------------------------------------------------------------
// Meta-tests on the fixture set itself
// ---------------------------------------------------------------------------

/// Every manifest entry has a committed body and a named test above, and no
/// stray `.json` is left behind. Without this, adding a native endpoint to
/// `requests.tsv` and forgetting the test would leave the new wire unguarded —
/// the very gap this suite closes.
#[test]
fn every_manifest_entry_is_covered_by_a_committed_body() {
    let entries = requests();
    assert_eq!(
        entries.len(),
        11,
        "the manifest gained or lost an endpoint; add/remove the matching \
         #[tokio::test] above and update this count"
    );

    let dir = fixtures_dir().join("native/v1");
    let mut committed: Vec<String> = std::fs::read_dir(&dir)
        .expect("fixtures/native/v1 readable")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(str::to_string)
        })
        .collect();
    committed.sort();

    let mut named: Vec<String> = entries.iter().map(|(name, _)| name.clone()).collect();
    named.sort();
    assert_eq!(
        named, committed,
        "fixtures/native/v1/*.json and requests.tsv disagree — run `just snapshot-native`"
    );
}

/// Nothing in the pinned request set may depend on the system clock. Every
/// handler defaults its date param to `today_utc()`, so a request that omitted
/// one would produce a body that changed at midnight and a suite that failed
/// for no reason. Each URI must carry its own as-of.
#[test]
fn every_pinned_request_fixes_its_own_dates() {
    for (name, uri) in requests() {
        assert!(
            uri.contains("asOf=") || uri.contains("end=") || uri.contains("to="),
            "{name}: {uri} has no pinned end date, so its golden would drift with the clock"
        );
    }
}
