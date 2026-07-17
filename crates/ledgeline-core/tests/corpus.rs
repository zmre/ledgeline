//! Hermetic parse-corpus parity tests against real hledger `print -O json`.
//!
//! Each `fixtures/corpus/*.journal` is a focused snippet adapted (mostly
//! verbatim) from hledger's own parse tests under `hledger/test/journal/*.test`.
//! `scripts/gen-corpus.sh` captured `hledger -f FILE print -O json` into the
//! committed sibling `*.print.json` goldens, so this test runs without hledger
//! installed. For every fixture we `parse_journal` + serialize the transactions
//! and semantic-diff against the golden, reusing the `common` comparator (which
//! ignores `floatingPoint` and `sourceName` everywhere).
//!
//! `fixtures/corpus/errors/*.journal` are journals hledger REJECTS; we assert
//! our parser rejects them too.
//!
//! Fixtures our parser cannot yet reproduce are enumerated in [`GAPS`], each
//! with a `// GAP:` note naming the missing feature and the concrete hledger vs
//! ours discrepancy. GAP cases are NOT held to parity — but the test DOES assert
//! each still diverges, so if a later parser fix makes one match, the test goes
//! red to prompt removing the stale entry. This keeps `cargo test` green while
//! the gaps stay visible and honest. Per the task, the parser is NOT modified to
//! close these here.

mod common;

use common::compare;
use ledgeline_core::{parse_journal, wire};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Known parser gaps: fixture stem -> one-line reason. Each entry currently
/// diverges from hledger's golden (the test enforces that). Kept alphabetized.
const GAPS: &[(&str, &str)] = &[
    // GAP: balanced virtual postings `[a]` — the brackets are kept in the
    // account name, they are not balanced separately, and ptype stays
    // RegularPosting instead of BalancedVirtualPosting.
    (
        "balanced-virtual-postings",
        "[a] brackets kept in paccount, not balanced separately, ptype not BalancedVirtualPosting",
    ),
    // GAP: symbol-only `commodity $` directive is a parse error (we require a
    // number), so the whole journal fails; commodity tags are also not
    // propagated to postings.
    (
        "commodity-tags",
        "symbol-only `commodity $` directive rejected; commodity tags not propagated to ptags",
    ),
    // GAP: amounts with no commodity symbol (bare `1`) are a parse error — we
    // require a commodity token.
    (
        "commodityless-amounts",
        "amounts without a commodity symbol are rejected (empty-commodity unsupported)",
    ),
    // GAP: the `D` default-commodity directive is rejected outright.
    (
        "default-commodity",
        "D default-commodity directive is a fail-loud UnsupportedDirective",
    ),
    // GAP: no per-commodity canonical style — an inferred (elided) counter-leg
    // for a right-side, grouped commodity gets a bare left-side style instead of
    // the commodity's `1,000.00 EUR` shape (wrong ascommodityside/asdigitgroups).
    (
        "inferred-commodity-style",
        "inferred elided leg does not adopt the commodity's canonical style",
    ),
    // GAP: no per-commodity canonical style — a commodity that never appears
    // with a decimal mark (e.g. `-1712 D`) should have asdecimalmark null; we
    // always emit ".".
    (
        "precision",
        "asdecimalmark is '.' for a no-decimal commodity where hledger emits null",
    ),
    // GAP: no per-commodity canonical style — a space digit-group separator in a
    // `commodity` directive is not recognized (number truncated at the space),
    // so amounts keep their as-written "," groups instead of the canonical " ".
    (
        "numbers-space-groups",
        "space digit-group separator in commodity directive unsupported",
    ),
    // GAP: a `date:` posting tag is not lifted into pdate (stays null).
    ("posting-dates", "date: posting tag not applied to pdate"),
    // GAP: E-notation behind a left commodity (`$1.05e2`) is a Dec parse error.
    (
        "scientific-commodity",
        "scientific notation with a commodity is a parse error",
    ),
    // GAP: E-notation with no commodity (`1.05e2`) parses as number + bogus
    // commodity `e2` instead of the scientific value.
    (
        "scientific",
        "scientific notation parsed as number + commodity instead of its value",
    ),
    // GAP: hledger normalizes a `@@` total-cost amount (strips trailing zeros:
    // `$135.00` -> 135 / 0 places); we keep it as written (13500 / 2 places).
    (
        "total-cost-trailing-zeros",
        "@@ total-cost amount not normalized (trailing zeros kept)",
    ),
    // GAP: unbalanced virtual posting `(a)` — parens kept in the account name,
    // it wrongly participates in balancing, and ptype stays RegularPosting.
    (
        "virtual-postings",
        "(a) parens kept in paccount, participates in balancing, ptype not VirtualPosting",
    ),
];

/// The reason string if `stem` is a known gap.
fn gap_reason(stem: &str) -> Option<&'static str> {
    GAPS.iter()
        .find(|(name, _)| *name == stem)
        .map(|(_, reason)| *reason)
}

/// The `fixtures/corpus/` directory.
fn corpus_dir() -> PathBuf {
    common::fixtures_dir().join("corpus")
}

/// The `*.journal` files directly in `dir`, sorted by name.
fn journals_in(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "journal"))
        .collect();
    paths.sort();
    paths
}

/// A fixture's stem (`numbers.journal` -> `numbers`).
fn stem(path: &Path) -> String {
    path.file_stem()
        .expect("journal has a stem")
        .to_string_lossy()
        .into_owned()
}

/// Parse + serialize `journal_path` and semantic-diff against its
/// `*.print.json` golden. `Err` carries a parse error or the first JSON-path
/// mismatch — either way, "diverges from hledger".
fn print_case_result(journal_path: &Path) -> Result<(), String> {
    let golden_path = journal_path.with_extension("print.json");
    let text = std::fs::read_to_string(journal_path)
        .map_err(|e| format!("read {}: {e}", journal_path.display()))?;
    let source_name = journal_path.to_string_lossy().to_string();
    let journal = parse_journal(&text, &source_name).map_err(|e| format!("parse error: {e}"))?;
    let actual = wire::journal_to_value(&journal).map_err(|e| format!("serialize error: {e}"))?;
    let golden = std::fs::read_to_string(&golden_path)
        .map_err(|e| format!("read {}: {e}", golden_path.display()))?;
    let expected: Value =
        serde_json::from_str(&golden).map_err(|e| format!("golden parse error: {e}"))?;
    compare("$", &expected, &actual)
}

#[test]
fn corpus_matches_hledger_print() {
    let dir = corpus_dir();
    let print_cases = journals_in(&dir);
    assert!(
        !print_cases.is_empty(),
        "no corpus fixtures found in {} (run scripts/gen-corpus.sh)",
        dir.display()
    );

    let mut failures: Vec<String> = Vec::new();
    let mut stale_gaps: Vec<String> = Vec::new();
    let mut gap_notes: Vec<String> = Vec::new();
    let mut passed = 0usize;

    for journal_path in &print_cases {
        let stem = stem(journal_path);
        let result = print_case_result(journal_path);
        match (gap_reason(&stem), result) {
            // Expected-good fixture: must match hledger exactly.
            (None, Ok(())) => passed += 1,
            (None, Err(message)) => failures.push(format!("{stem}: {message}")),
            // Known gap: confirm it still diverges (parse error or mismatch), and
            // record the current discrepancy so the gap is self-documenting.
            (Some(reason), Err(message)) => {
                gap_notes.push(format!("{stem} [{reason}] -> {message}"));
            }
            (Some(reason), Ok(())) => stale_gaps.push(format!(
                "{stem}: now matches hledger — remove it from GAPS (was: {reason})"
            )),
        }
    }

    // Error fixtures: journals hledger rejects, which our parser must reject too.
    let errors_dir = dir.join("errors");
    let error_cases = journals_in(&errors_dir);
    assert!(
        !error_cases.is_empty(),
        "no error fixtures found in {}",
        errors_dir.display()
    );
    let mut errors_confirmed = 0usize;
    for journal_path in &error_cases {
        let stem = stem(journal_path);
        let text = std::fs::read_to_string(journal_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", journal_path.display()));
        let source_name = journal_path.to_string_lossy().to_string();
        match parse_journal(&text, &source_name) {
            Err(_) => errors_confirmed += 1,
            Ok(_) => failures.push(format!(
                "errors/{stem}: hledger rejects this journal but our parser accepted it"
            )),
        }
    }

    println!(
        "corpus: {passed} pass, {} gaps confirmed, {errors_confirmed} error fixtures reject",
        gap_notes.len()
    );
    for note in &gap_notes {
        println!("  GAP {note}");
    }

    let mut problems = failures;
    problems.extend(stale_gaps);
    assert!(
        problems.is_empty(),
        "corpus parity problems:\n  - {}",
        problems.join("\n  - ")
    );
}
