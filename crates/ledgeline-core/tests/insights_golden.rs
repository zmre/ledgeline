//! Golden snapshot of the whole Insights report.
//!
//! Every box of `/api/insights` for two holdings-bearing fixtures over three
//! spans, pinned as the report's pretty `Debug` text: every `Dec` mantissa and
//! scale, and every `f64` in its round-trip form, so a change that moves any
//! figure at all shows up here. It exists so performance work on the dashboard
//! (plan 38 WP-1 shared the price database and the holdings replays) can prove
//! it changed nothing.
//!
//! An intended change to the report regenerates the snapshots with
//! `LEDGELINE_BLESS=1 cargo test -p ledgeline-core --test insights_golden`;
//! review the diff before committing it.

mod common;

use ledgeline_core::reports::{InsightsOpts, insights};
use ledgeline_core::{Dec, parse_journal};

/// `(fixture path under fixtures/, snapshot name)`.
const FIXTURES: [(&str, &str); 2] = [
    ("sample.journal", "sample"),
    ("parity/holdings.journal", "parity-holdings"),
];

/// `(variant, start, end)`: a long span, a short one, one opening before the
/// journal does, and a `tail` whose current half holds no transactions at all
/// (every trade lands in the previous window). Fixed dates, so the snapshots
/// never depend on today.
fn spans(name: &str) -> [(&'static str, &'static str, &'static str); 4] {
    match name {
        "sample" => [
            ("24m", "2024-07-01", "2026-07-16"),
            ("3m", "2026-04-01", "2026-07-16"),
            ("all", "2024-01-01", "2026-07-16"),
            ("tail", "2026-01-01", "2027-12-31"),
        ],
        _ => [
            ("12m", "2024-01-01", "2024-12-31"),
            ("3m", "2024-04-01", "2024-07-01"),
            ("all", "2023-01-01", "2024-07-01"),
            ("tail", "2023-07-01", "2025-12-31"),
        ],
    }
}

#[test]
fn insights_output_matches_the_committed_snapshots() {
    let dir = common::fixtures_dir();
    let bless = std::env::var_os("LEDGELINE_BLESS").is_some();
    let mut mismatched = Vec::new();
    for (path, name) in FIXTURES {
        let text = std::fs::read_to_string(dir.join(path)).expect("fixture readable");
        let journal = parse_journal(&text, path).expect("fixture parses");
        for (variant, start, end) in spans(name) {
            let report = insights(
                &journal,
                &InsightsOpts {
                    start,
                    end,
                    cost_exclude: &[],
                    change_min: Dec::zero(),
                },
            )
            .expect("insights succeeds");
            let actual = format!("{report:#?}\n");
            let snapshot = dir.join("insights").join(format!("{name}.{variant}.txt"));
            if bless {
                std::fs::create_dir_all(snapshot.parent().expect("has a parent"))
                    .expect("snapshot dir");
                std::fs::write(&snapshot, &actual).expect("snapshot written");
                continue;
            }
            let expected = std::fs::read_to_string(&snapshot)
                .unwrap_or_else(|e| panic!("{}: {e}", snapshot.display()));
            if expected != actual {
                mismatched.push(snapshot.display().to_string());
            }
        }
    }
    assert!(
        mismatched.is_empty(),
        "insights output changed for {mismatched:?}; if intended, re-run with LEDGELINE_BLESS=1 and review the diff"
    );
}
