//! The format-preserving journal date sort, over `fixtures/import/sort/`.
//!
//! Two things are being proved and it matters that they are separate:
//!
//! | Check | What it proves |
//! | --- | --- |
//! | `cargo test -p ledgeline-core --test sort` | The sort moves transactions and damages nothing else |
//! | `LEDGELINE_HLEDGER_SORT_CHECK=1 cargo test …` | The sorted files are journals **real hledger accepts**, in date order |
//!
//! The first alone is not enough: a sort could shuffle bytes into something that
//! round-trips through our own reader perfectly and that hledger rejects. Only
//! the binary can answer that, so the second exists — opt-in, so `cargo test`
//! stays hermetic and needs no hledger.
//!
//! Every transaction-shaped assertion here reads the fixtures through a
//! **deliberately independent** grammar reader ([`transactions`]) rather than
//! through `sort`'s own item model. A permutation check that asks the code under
//! test what a transaction is proves nothing.

mod common;

use ledgeline_core::sort::{Move, SortError, SortPlan, apply, plan};
use proptest::prelude::*;
use std::collections::BTreeMap;

/// Every fixture under `fixtures/import/sort/`, as `(name, text)`.
fn fixtures() -> Vec<(String, String)> {
    let dir = common::fixtures_dir().join("import/sort");
    let mut found: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("fixtures/import/sort exists")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "journal").then_some(())?;
            Some((
                path.file_name()?.to_string_lossy().into_owned(),
                std::fs::read_to_string(&path).ok()?,
            ))
        })
        .collect();
    found.sort();
    assert!(found.len() >= 6, "the fixture corpus should be present");
    found
}

fn fixture(name: &str) -> String {
    let path = common::fixtures_dir().join("import/sort").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} readable: {e}"))
}

/// Read `text`, apply its own sort, and hand back both.
fn sorted(name: &str) -> (String, SortPlan, String) {
    let text = fixture(name);
    let plan = plan(&text).unwrap_or_else(|e| panic!("{name} plans: {e}"));
    let out = apply(&text, &plan).unwrap_or_else(|e| panic!("{name} applies: {e}"));
    (text, plan, out)
}

// ---------------------------------------------------------------------------
// An independent reading of the journal grammar
// ---------------------------------------------------------------------------

/// `text`'s transactions, each as its terminator-stripped lines.
///
/// A second, deliberately naive implementation of hledger's rule: a column-1
/// line starting with a digit, plus every following line that is indented and
/// not whitespace-only. It does not know about `comment` blocks, which is
/// harmless for the multiset comparisons it feeds — a block never moves, so its
/// contents contribute the same entries on both sides.
fn transactions(text: &str) -> Vec<Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with(|c: char| c.is_ascii_digit()))
        .map(|(at, header)| {
            std::iter::once(*header)
                .chain(
                    lines[at + 1..]
                        .iter()
                        .take_while(|line| line.starts_with([' ', '\t']) && !line.trim().is_empty())
                        .copied(),
                )
                .map(str::to_string)
                .collect()
        })
        .collect()
}

/// The date on each transaction header, in file order.
fn dates(text: &str) -> Vec<String> {
    transactions(text)
        .iter()
        .filter_map(|txn| txn.first()?.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Every 1-based line number at which `text` holds exactly `line`.
fn lines_at(text: &str, line: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, seen)| *seen == line)
        .map(|(at, _)| at + 1)
        .collect()
}

/// How many times each distinct entry occurs — a multiset, for permutation
/// checks that must not care about order.
fn multiset<T: Ord>(items: Vec<T>) -> BTreeMap<T, usize> {
    items.into_iter().fold(BTreeMap::new(), |mut counts, item| {
        *counts.entry(item).or_insert(0) += 1;
        counts
    })
}

// ---------------------------------------------------------------------------
// Every fixture
// ---------------------------------------------------------------------------

#[test]
fn every_fixture_plans_applies_and_proves_itself() {
    for (name, text) in fixtures() {
        let plan = plan(&text).unwrap_or_else(|e| panic!("{name} plans: {e}"));
        let out = apply(&text, &plan).unwrap_or_else(|e| panic!("{name} applies: {e}"));
        assert_eq!(
            multiset(transactions(&text)),
            multiset(transactions(&out)),
            "{name}: every transaction must survive byte for byte"
        );
        assert_eq!(
            plan.unchanged,
            out == text,
            "{name}: `unchanged` must mean the bytes are identical"
        );
    }
}

#[test]
fn every_fixture_sorts_to_a_fixed_point() {
    for (name, text) in fixtures() {
        let once = apply(&text, &plan(&text).expect("plans")).expect("applies");
        let again = plan(&once).unwrap_or_else(|e| panic!("{name} re-plans: {e}"));
        assert!(
            again.unchanged,
            "{name}: sorting twice must equal sorting once"
        );
        assert_eq!(
            apply(&once, &again).as_deref(),
            Ok(once.as_str()),
            "{name}: a sorted file is its own fixed point"
        );
    }
}

#[test]
fn an_already_sorted_file_is_the_exact_identity() {
    let (text, plan, out) = sorted("already-sorted.journal");
    assert!(plan.unchanged);
    assert!(plan.moves.is_empty());
    // Byte-for-byte, not merely equivalent: no rewrite at all.
    assert_eq!(out, text);
}

// ---------------------------------------------------------------------------
// interleaved.journal — directives, an include and a P must not move
// ---------------------------------------------------------------------------

/// Lines of `interleaved.journal` that must come back at the same line number.
/// Every one of them would change the journal's meaning if it moved.
const PINNED: &[&str] = &[
    "; fixtures/import/sort/interleaved.journal",
    "account assets:bank:checking",
    "account income:salary",
    "commodity $1,000.00",
    "commodity 1,000.00 AAPL",
    "include interleaved-include.journal",
    "; A price directive sitting between transactions. It must not move, and the",
    "P 2026-02-01 AAPL $190.00",
];

#[test]
fn interleaved_puts_its_transactions_in_date_order() {
    let (text, _, out) = sorted("interleaved.journal");
    assert_eq!(
        dates(&text),
        ["2026-02-10", "2026-02-03", "2026-01-28", "2026-02-07"]
    );
    assert_eq!(
        dates(&out),
        ["2026-01-28", "2026-02-03", "2026-02-07", "2026-02-10"]
    );
}

#[test]
fn interleaved_leaves_every_directive_include_and_comment_exactly_where_it_was() {
    let (text, _, out) = sorted("interleaved.journal");
    for line in PINNED {
        let before = lines_at(&text, line);
        assert!(!before.is_empty(), "fixture should contain {line:?}");
        assert_eq!(
            before,
            lines_at(&out, line),
            "{line:?} must not move; moving it changes what the journal means"
        );
    }
    assert_eq!(
        text.lines().count(),
        out.lines().count(),
        "a sort adds and removes no lines"
    );
}

#[test]
fn interleaved_reports_the_moves_a_user_would_confirm() {
    let (_, plan, _) = sorted("interleaved.journal");
    assert!(!plan.unchanged);
    assert_eq!(
        plan.moves,
        vec![
            Move {
                date: "2026-01-28".to_string(),
                description: "Paycheck".to_string(),
                from_line: 30,
                to_line: 18,
            },
            Move {
                date: "2026-02-07".to_string(),
                description: "Grocery store".to_string(),
                from_line: 34,
                to_line: 30,
            },
            Move {
                date: "2026-02-10".to_string(),
                description: "Grocery store".to_string(),
                from_line: 18,
                to_line: 34,
            },
        ],
        "moves are reported in output order, with the line each row came from"
    );
}

#[test]
fn a_transaction_that_kept_its_place_is_not_reported_as_moved() {
    // 2026-02-03 sorts into the slot it already occupied. Listing it would fill
    // the confirmation diff with rows that are not the point.
    let (_, plan, _) = sorted("interleaved.journal");
    assert!(
        !plan.moves.iter().any(|m| m.date == "2026-02-03"),
        "an unmoved transaction must not appear in the diff"
    );
}

// ---------------------------------------------------------------------------
// comments.journal — lead comments travel, a header comment does not
// ---------------------------------------------------------------------------

#[test]
fn a_lead_comment_travels_with_its_transaction() {
    let (_, _, out) = sorted("comments.journal");
    let lines: Vec<&str> = out.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with("2026-02-14"))
        .expect("the cafe transaction is present");
    assert_eq!(
        lines.get(at - 2..at),
        Some(
            [
                "; Filed late: an import appended this row after the rent above.",
                "; Both of these comment lines must travel with it.",
            ]
            .as_slice()
        ),
        "both comment lines must arrive with the transaction they explain"
    );

    let rent = lines
        .iter()
        .position(|line| line.starts_with("2026-03-01"))
        .expect("the rent transaction is present");
    assert_eq!(
        lines.get(rent - 1),
        Some(&"; Rent goes out on the first of the month."),
        "the other transaction keeps its own comment too"
    );
    assert!(at < rent, "the sort must have swapped the two");
}

#[test]
fn a_file_header_comment_separated_by_a_blank_line_never_moves() {
    let (text, _, out) = sorted("comments.journal");
    for line in [
        "; fixtures/import/sort/comments.journal",
        "; A file-header comment, separated from everything below by a blank line. It",
        "; attaches to no transaction and must never move.",
    ] {
        assert_eq!(lines_at(&text, line), lines_at(&out, line), "{line:?}");
    }
}

// ---------------------------------------------------------------------------
// Terminator edge cases
// ---------------------------------------------------------------------------

#[test]
fn crlf_survives_the_sort() {
    let (text, plan, out) = sorted("crlf.journal");
    assert!(!plan.unchanged, "the fixture is out of order");
    assert_eq!(
        out.matches("\r\n").count(),
        text.matches("\r\n").count(),
        "every terminator must still be CRLF"
    );
    assert!(
        !out.replace("\r\n", "").contains('\n'),
        "no terminator may have been rewritten as a bare LF"
    );
    assert_eq!(dates(&out), ["2026-02-03", "2026-02-10"]);
}

#[test]
fn a_missing_final_newline_survives_when_its_transaction_stays_last() {
    let (text, plan, out) = sorted("no-final-newline.journal");
    assert!(!text.ends_with('\n'), "the fixture must lack a terminator");
    assert!(!plan.unchanged, "the fixture is out of order");
    assert!(
        !out.ends_with('\n'),
        "the sorted file must still end without a terminator"
    );
    assert_eq!(dates(&out), ["2026-01-01", "2026-01-05", "2026-01-10"]);
}

#[test]
fn a_terminatorless_last_transaction_is_given_one_when_it_stops_being_last() {
    // The only case where the sort adds a byte, and it must: without the
    // terminator the moved transaction would be glued onto its new successor.
    let text =
        "2026-02-01 later\n    a:b  $1.00\n    c:d\n\n2026-01-01 earlier\n    a:b  $1.00\n    c:d";
    let out = apply(text, &plan(text).expect("plans")).expect("applies");
    assert_eq!(
        out,
        "2026-01-01 earlier\n    a:b  $1.00\n    c:d\n\n2026-02-01 later\n    a:b  $1.00\n    c:d\n"
    );
    assert_eq!(out.len(), text.len() + 1, "exactly one byte is added");
}

// ---------------------------------------------------------------------------
// Stability
// ---------------------------------------------------------------------------

#[test]
fn same_day_transactions_keep_their_relative_order() {
    // `.latest`-based import dedup assumes this: reshuffling same-day rows
    // changes which rows a later import considers new.
    let text = concat!(
        "2026-02-01 third\n    a:b  $3.00\n    c:d\n\n",
        "2026-01-09 first\n    a:b  $1.00\n    c:d\n\n",
        "2026-01-09 second\n    a:b  $2.00\n    c:d\n",
    );
    let out = apply(text, &plan(text).expect("plans")).expect("applies");
    assert_eq!(
        transactions(&out)
            .iter()
            .filter_map(|txn| txn.first().cloned())
            .collect::<Vec<_>>(),
        [
            "2026-01-09 first".to_string(),
            "2026-01-09 second".to_string(),
            "2026-02-01 third".to_string(),
        ]
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_plan_from_one_file_is_refused_against_another() {
    let (_, plan_of_interleaved, _) = sorted("interleaved.journal");
    let other = fixture("comments.journal");
    assert_eq!(
        apply(&other, &plan_of_interleaved),
        Err(SortError::StalePlan),
        "the bytes written must be the sort the user was shown, of the file as it stands"
    );
}

#[test]
fn a_yearless_date_refuses_the_whole_file_rather_than_guessing() {
    let text = "Y 2026\n\n02/01 later\n    a:b  $1.00\n    c:d\n\n01/09 earlier\n    a:b  $1.00\n    c:d\n";
    assert_eq!(plan(text), Err(SortError::UnreadableDate { line: 3 }));
    assert_eq!(
        apply(
            text,
            &SortPlan {
                moves: Vec::new(),
                unchanged: true
            }
        ),
        Err(SortError::UnreadableDate { line: 3 })
    );
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// A transaction, sometimes with a leading comment, sometimes with a trailing
/// blank line.
fn transaction() -> impl Strategy<Value = String> {
    (
        2020i32..2030,
        1u32..=12,
        1u32..=28,
        "[a-z][a-z ]{0,10}",
        1u32..=99,
        prop::bool::ANY,
        prop::bool::ANY,
    )
        .prop_map(|(y, m, d, desc, cents, lead, blank)| {
            format!(
                "{}{y:04}-{m:02}-{d:02} {desc}\n    assets:a  ${cents}.00\n    expenses:b\n{}",
                if lead { "; why this exists\n" } else { "" },
                if blank { "\n" } else { "" },
            )
        })
}

/// The non-transaction constructs: declarations that may be sorted across, the
/// directives that are barriers, comment runs, and blank lines.
fn other_block() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("account assets:a\n".to_string()),
        Just("payee Someone\n".to_string()),
        Just("P 2026-01-01 AAPL $1.00\n".to_string()),
        Just("commodity $1,000.00\n".to_string()),
        Just("decimal-mark .\n".to_string()),
        Just("Y 2026\n".to_string()),
        Just("include elsewhere.journal\n".to_string()),
        Just("apply account budget\n".to_string()),
        Just("; a standalone note\n\n".to_string()),
        Just("comment\n2026-01-01 not a transaction\nend comment\n".to_string()),
        Just("\n".to_string()),
        Just("   \n".to_string()),
    ]
}

fn journal_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop_oneof![3 => transaction(), 2 => other_block()], 0..14)
        .prop_map(|blocks| blocks.concat())
        .prop_flat_map(|text| {
            // Half the corpus loses its final terminator: the one case where a
            // reorder has to synthesize a byte.
            prop::bool::ANY.prop_map(move |trim| {
                if trim {
                    text.trim_end_matches('\n').to_string()
                } else {
                    text.clone()
                }
            })
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The model's whole claim: the sort is a permutation of the transactions
    /// and every one of them comes back byte for byte.
    #[test]
    fn apply_is_a_permutation_preserving_every_transaction(text in journal_text()) {
        let plan = plan(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let out = apply(&text, &plan).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(multiset(transactions(&text)), multiset(transactions(&out)));
        // Nothing outside a transaction changes either, so the file's whole line
        // multiset is preserved.
        prop_assert_eq!(
            multiset(text.lines().collect::<Vec<_>>()),
            multiset(out.lines().collect::<Vec<_>>())
        );
    }

    /// Sorting twice equals sorting once, and an already-sorted file is the
    /// exact identity — which is what makes the second sort a no-op rather than
    /// a second rewrite.
    #[test]
    fn sorting_twice_equals_sorting_once(text in journal_text()) {
        let first = plan(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let once = apply(&text, &first).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let second = plan(&once).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(second.unchanged);
        prop_assert!(second.moves.is_empty());
        let twice = apply(&once, &second).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&twice, &once);
    }

    /// `unchanged` is exactly "the bytes do not move", and the terminator
    /// convention is never rewritten.
    #[test]
    fn unchanged_means_byte_identical_and_terminators_are_never_rewritten(text in journal_text()) {
        let plan = plan(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let out = apply(&text, &plan).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(plan.unchanged, out == text);
        prop_assert_eq!(text.matches("\r\n").count(), out.matches("\r\n").count());
        // The only byte a sort may add is the terminator a formerly-last item
        // needs when it stops being last.
        prop_assert!(out.len() == text.len() || out.len() == text.len() + 1);
    }
}

// ---------------------------------------------------------------------------
// Opt-in: is the sorted file one real hledger accepts, in date order?
// ---------------------------------------------------------------------------

/// The environment variable that opts in to running the hledger binary.
const OPT_IN: &str = "LEDGELINE_HLEDGER_SORT_CHECK";

/// Every fixture, sorted, must satisfy `hledger check --strict ordereddates`.
///
/// Default-skipped so `cargo test` stays hermetic and needs no hledger. This
/// runs hledger **only** against the committed fixtures, never a user's file.
#[test]
fn sorted_fixtures_are_journals_hledger_accepts_in_date_order() {
    if std::env::var_os(OPT_IN).is_none() {
        eprintln!("skipped; set {OPT_IN}=1 to run it");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("ledgeline-sort-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("create scratch dir");

    for (name, text) in fixtures() {
        let out = apply(&text, &plan(&text).expect("plans")).expect("applies");
        let path = scratch.join(&name);
        std::fs::write(&path, &out).expect("write scratch journal");
        // `interleaved.journal` includes a sibling, so the whole directory has
        // to travel, not just the file under test.
        std::fs::write(
            scratch.join("interleaved-include.journal"),
            fixture("interleaved-include.journal"),
        )
        .expect("write the include target");

        let output = std::process::Command::new("hledger")
            .args(["-f".as_ref(), path.as_os_str(), "check".as_ref()])
            .args(["--strict", "ordereddates"])
            .output()
            .expect("hledger runs");
        assert!(
            output.status.success(),
            "{name} after sorting:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
