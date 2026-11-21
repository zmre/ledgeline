//! The invoicer-core bench gate (ARCHITECTURE §5.6, PLAN I14): every bench in
//! `benches/baseline.json` must have run, and none may be more than the
//! baseline's `threshold` (20%) slower than its stored median.
//!
//! ```text
//! just invoicer-bench-gate            # run the benches, then this
//! cargo run -p invoicer-core --example bench_gate             # compare the last run
//! cargo run -p invoicer-core --example bench_gate -- --write  # re-record the baseline
//! ```
//!
//! A bench over the threshold is re-run, alone, up to twice more, and its
//! best median counts: timings on a laptop shared with other work swing
//! by more than 20% between runs (measured: +70% on an unchanged tree), and
//! a gate that flaps is a gate people learn to ignore. A real regression is
//! slow on every run.
//!
//! It reads criterion's own output (`target/criterion/<id>/new/estimates.json`,
//! the median's point estimate in nanoseconds), so it gates exactly what
//! `cargo bench` printed. The baseline is a committed file because
//! criterion's `--save-baseline` lives under `target/`, which is never in git.
//! Timings are machine-specific: the baseline records the machine it was
//! taken on, and re-recording it on another machine is a deliberate act.

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The criterion groups invoicer-core's benches write.
const GROUPS: [&str; 3] = ["table", "invoicer", "ledgeline"];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn criterion_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| manifest_dir().join("../../../target"), PathBuf::from)
        .join("criterion")
}

/// Every `<id>/new/estimates.json` under `dir`, as `(id, median ns)`.
fn results(root: &Path, dir: &Path, out: &mut Vec<(String, f64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|n| n == "new") {
            let median = std::fs::read_to_string(path.join("estimates.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|v| v["median"]["point_estimate"].as_f64());
            if let (Some(median), Ok(id)) = (median, dir.strip_prefix(root)) {
                let id = id.to_string_lossy().replace('\\', "/");
                if GROUPS.iter().any(|g| id.starts_with(&format!("{g}/"))) {
                    out.push((id, median));
                }
            }
        } else if path
            .file_name()
            .is_some_and(|n| n != "base" && n != "change" && n != "report")
        {
            results(root, &path, out);
        }
    }
}

fn format_ns(ns: f64) -> String {
    match ns {
        n if n >= 1e9 => format!("{:.2} s", n / 1e9),
        n if n >= 1e6 => format!("{:.2} ms", n / 1e6),
        n if n >= 1e3 => format!("{:.2} µs", n / 1e3),
        n => format!("{n:.0} ns"),
    }
}

/// Re-run one bench alone (an anchored criterion filter) and return its new
/// median.
fn rerun(id: &str, root: &Path) -> Option<f64> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "bench",
            "-q",
            "-p",
            "invoicer-core",
            "--",
            &format!("^{id}$"),
        ])
        .stdout(std::process::Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let mut found = Vec::new();
    results(root, root, &mut found);
    found.into_iter().find(|(f, _)| f == id).map(|(_, ns)| ns)
}

const RETRIES: usize = 2;

fn main() -> ExitCode {
    let write = std::env::args().any(|a| a == "--write");
    let baseline_path = manifest_dir().join("benches/baseline.json");
    let root = criterion_dir();
    let mut found = Vec::new();
    results(&root, &root, &mut found);
    found.sort_by(|a, b| a.0.cmp(&b.0));
    if found.is_empty() {
        eprintln!(
            "bench gate: no criterion results under {}; run the benches first",
            root.display()
        );
        return ExitCode::FAILURE;
    }
    let baseline: Value = std::fs::read_to_string(&baseline_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({ "threshold": 0.2, "benches": {} }));

    if write {
        let benches: Map<String, Value> = found
            .iter()
            .map(|(id, ns)| (id.clone(), json!(ns.round())))
            .collect();
        let mut updated = baseline.clone();
        updated["benches"] = Value::Object(benches);
        let text = serde_json::to_string_pretty(&updated).unwrap_or_default();
        return match std::fs::write(&baseline_path, format!("{text}\n")) {
            Ok(()) => {
                println!(
                    "bench gate: recorded {} medians in {}",
                    found.len(),
                    baseline_path.display()
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("bench gate: can't write {}: {e}", baseline_path.display());
                ExitCode::FAILURE
            }
        };
    }

    let threshold = baseline["threshold"].as_f64().unwrap_or(0.2);
    let empty = Map::new();
    let benches = baseline["benches"].as_object().unwrap_or(&empty);
    let mut failed = false;
    println!(
        "{:<40} {:>12} {:>12} {:>8}",
        "bench", "baseline", "now", "change"
    );
    for (id, base) in benches {
        let Some(base) = base.as_f64() else { continue };
        match found.iter().find(|(f, _)| f == id) {
            None => {
                failed = true;
                println!(
                    "{id:<40} {:>12} {:>12} {:>8}  NOT RUN",
                    format_ns(base),
                    "-",
                    "-"
                );
            }
            Some((_, first)) => {
                let mut now = *first;
                let mut tries = 0;
                while now / base - 1.0 > threshold && tries < RETRIES {
                    tries += 1;
                    eprintln!(
                        "bench gate: {id} is {:+.0}%; re-running it alone ({tries}/{RETRIES})",
                        (now / base - 1.0) * 100.0
                    );
                    now = rerun(id, &root).map_or(now, |again| now.min(again));
                }
                let change = now / base - 1.0;
                let over = change > threshold;
                failed |= over;
                println!(
                    "{id:<40} {:>12} {:>12} {:>+7.1}%{}",
                    format_ns(base),
                    format_ns(now),
                    change * 100.0,
                    if over { "  REGRESSED" } else { "" }
                );
            }
        }
    }
    for (id, now) in found.iter().filter(|(id, _)| !benches.contains_key(id)) {
        println!(
            "{id:<40} {:>12} {:>12} {:>8}  new: not gated until recorded",
            "-",
            format_ns(*now),
            "-"
        );
    }
    if failed {
        eprintln!(
            "bench gate: FAILED (more than {:.0}% slower, or not run)",
            threshold * 100.0
        );
        ExitCode::FAILURE
    } else {
        println!("bench gate: ok (threshold {:.0}%)", threshold * 100.0);
        ExitCode::SUCCESS
    }
}
