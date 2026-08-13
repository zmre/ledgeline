//! Rules-file candidate matching (WP-11 lane D).
//!
//! The suite is built around one claim, which is the reason the module exists:
//!
//! > **Parse success is not a matching signal.** A mismatched rules file
//! > frequently succeeds with garbage, exit 0. (`plans/11-enhanced-import.md`,
//! > fact 4.)
//!
//! `fixtures/import/match/garbage-success.rules` and `no-currency.rules` are that
//! finding, committed. hledger reads both against `checking.csv` without
//! complaint — `hledger check` exits 0 on both — and both must score near the
//! bottom anyway. A test that only proved "hledger accepted it" would be
//! asserting the bug.
//!
//! # Hermetic
//!
//! No test here requires hledger on `PATH`. Stage 2's input is real
//! `hledger print -O json` output, generated once by `scripts/gen-match-golden.sh`
//! and committed under `fixtures/import/match/golden/`. That keeps the bytes
//! honest — nobody is testing their own idea of hledger's shape — while keeping
//! `cargo test` runnable anywhere.
//!
//! `LEDGELINE_HLEDGER_MATCH_CHECK=1` opts in to re-running hledger and comparing,
//! mirroring `LEDGELINE_HLEDGER_RENDER_CHECK` in `rules_hledger_render.rs`. That
//! is the check that catches an hledger upgrade changing the contract.

mod common;

use ledgeline_core::convert::{SourceFormat, Tabular, delimited};
use ledgeline_core::rules::RulesDoc;
use ledgeline_core::rules::matching::{
    Candidate, MatchError, Ranking, Score, Signals, prefilter, rank, score,
    signals_from_hledger_json,
};
use proptest::prelude::*;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// The environment variable that opts in to the live hledger comparison.
const OPT_IN: &str = "LEDGELINE_HLEDGER_MATCH_CHECK";

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn match_dir() -> PathBuf {
    common::fixtures_dir().join("import/match")
}

/// One committed `*.rules` fixture, parsed.
fn rules(name: &str) -> RulesDoc {
    let path = match_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} readable: {e}"));
    RulesDoc::parse(&text)
}

/// One committed CSV fixture, through the real preprocessor — so stage 1 is
/// tested against the [`Tabular`] it will actually be handed, not a hand-built
/// one that might differ in where the header went.
fn table(name: &str) -> Tabular {
    let path = match_dir().join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{name} readable: {e}"));
    delimited::parse(&bytes, SourceFormat::Csv).unwrap_or_else(|e| panic!("{name} converts: {e}"))
}

/// One committed golden, as JSON.
fn golden(stem: &str) -> Value {
    let path = match_dir()
        .join("golden")
        .join(format!("{stem}.print.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{stem} golden: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{stem} golden parses: {e}"))
}

// ---------------------------------------------------------------------------
// Stage 1 — rejection without a subprocess
// ---------------------------------------------------------------------------

#[test]
fn stage_one_accepts_the_rules_file_that_actually_fits() {
    let pass = prefilter(&rules("checking.csv.rules"), &table("checking.csv"))
        .expect("the correct rules file must survive stage 1");

    assert!(pass.column_count_matches, "four fields, four columns");
    assert!(
        pass.header_matches_source,
        "`date` and `description` are named on both sides"
    );
    assert_eq!(pass.declared_columns, Some(4));
    assert_eq!(pass.data_columns, 4);
    assert_eq!(pass.expected_commodity.as_deref(), Some("$"));
    assert_eq!(
        (pass.dates_tried, pass.dates_parsed),
        (5, 5),
        "every sampled date reads under %m/%d/%Y"
    );
}

#[test]
fn stage_one_rejects_a_date_format_that_cannot_read_the_data() {
    // The whole point of `wrong-dateformat.rules`: it is right about everything
    // else — four fields against four columns — so the ONLY thing that can
    // reject it is the date format. A test that passed because the widths
    // differed would not be testing what it claims to.
    let doc = rules("wrong-dateformat.rules");
    let data = table("checking.csv");
    assert_eq!(
        doc.settings().fields.map(|setting| setting.value.len()),
        Some(4),
        "the fixture must not reject on width instead"
    );
    assert!(
        prefilter(&doc, &data).is_none(),
        "%d.%m.%Y cannot read 01/15/2024, and no subprocess is needed to know it"
    );
}

#[test]
fn stage_one_rejects_a_fields_list_wider_than_the_data() {
    // Five fields against four columns: the rules file addresses a column the
    // file does not have.
    assert!(
        prefilter(&rules("creditcard.csv.rules"), &table("checking.csv")).is_none(),
        "a rules file cannot describe columns that are not there"
    );
    // And the same pair the other way is rejected too, by the OTHER rule — the
    // credit-card statement's ISO dates defeat %m/%d/%Y.
    assert!(
        prefilter(&rules("checking.csv.rules"), &table("creditcard.csv")).is_none(),
        "%m/%d/%Y cannot read 2024-02-03"
    );
    // Each fixture does fit its own data.
    assert!(prefilter(&rules("creditcard.csv.rules"), &table("creditcard.csv")).is_some());
}

#[test]
fn stage_one_cannot_tell_the_fact_four_impostors_apart() {
    // This is the negative space that justifies stage 2 existing at all. Both
    // files are cheap-checkable-identical to the correct one: same width, same
    // date format. Nothing pure can reject them, and hledger will exit 0 on both.
    for name in ["garbage-success.rules", "no-currency.rules"] {
        let pass = prefilter(&rules(name), &table("checking.csv"))
            .unwrap_or_else(|| panic!("{name} must survive stage 1 — that is the point"));
        assert!(pass.column_count_matches, "{name} is as wide as the data");
        assert_eq!(pass.dates_parsed, 5, "{name} reads every date");
    }
}

#[test]
fn stage_one_reports_the_commodity_only_when_the_file_declares_one() {
    let with = prefilter(&rules("checking.csv.rules"), &table("checking.csv")).expect("accepted");
    let without = prefilter(&rules("no-currency.rules"), &table("checking.csv")).expect("accepted");
    assert_eq!(with.expected_commodity.as_deref(), Some("$"));
    assert_eq!(
        without.expected_commodity, None,
        "the missing `currency` line is exactly what makes this the trap"
    );
}

#[test]
fn stage_one_rejects_a_skip_that_swallows_the_file() {
    let data = table("checking.csv");
    // `skip 99` on a six-record file imports nothing at all.
    let doc = RulesDoc::parse(
        "skip 99\nfields date, description, amount-out, amount-in\ndate-format %m/%d/%Y\n",
    );
    assert!(prefilter(&doc, &data).is_none());

    // The same rules file against a TRUNCATED extract is not rejected: `rows` is
    // then a lower bound, so the count proves nothing.
    let truncated = Tabular {
        truncated: true,
        ..data
    };
    assert!(
        prefilter(&doc, &truncated).is_some(),
        "a preview must not reject a legitimately long preamble"
    );
}

#[test]
fn stage_one_declines_to_reject_a_question_it_cannot_answer() {
    let data = table("checking.csv");
    let cases = [
        // No `date-format` at all — hledger has built-in formats we do not model.
        (
            "no date-format",
            "fields date, description, amount-out, amount-in\nskip 1\n",
        ),
        // A specifier this module does not model: declining costs one subprocess,
        // guessing costs the user their correct rules file.
        (
            "an unmodelled specifier",
            "fields date, description, amount-out, amount-in\nskip 1\ndate-format %Y-%j\n",
        ),
        // No `fields` list, so there is no date column to look in.
        ("no fields list", "skip 1\ndate-format %d.%m.%Y\n"),
    ];
    for (why, text) in cases {
        let pass = prefilter(&RulesDoc::parse(text), &data)
            .unwrap_or_else(|| panic!("{why}: an unasked question must never reject"));
        assert_eq!(pass.dates_tried, 0, "{why}: the check must not have run");
    }
}

// ---------------------------------------------------------------------------
// Stage 2 — reading hledger's JSON
// ---------------------------------------------------------------------------

/// A posting, in hledger's `print -O json` shape.
fn posting(account: &str, amounts: Value) -> Value {
    json!({ "paccount": account, "pamount": amounts })
}

/// An amount, in hledger's shape. Only `acommodity` is read by the module.
fn amount(commodity: &str) -> Value {
    json!({ "acommodity": commodity, "aquantity": { "decimalMantissa": 1, "decimalPlaces": 0 } })
}

fn transaction(description: &str, postings: Vec<Value>) -> Value {
    json!({ "tdescription": description, "tpostings": postings })
}

#[test]
fn each_penalty_is_read_from_the_json_that_carries_it() {
    let json = json!([
        // Clean: two postings, both in $, both categorised, with a payee.
        transaction(
            "GROCER",
            vec![
                posting("assets:bank:checking", json!([amount("$")])),
                posting("expenses:food:groceries", json!([amount("$")])),
            ],
        ),
        // Fact 4a: an EMPTY `pamount` array is a posting with no amount.
        transaction("RENT", vec![posting("assets:bank:checking", json!([]))],),
        // Fact 4b: `acommodity: ""` is the bare-commodity trap.
        transaction(
            "PAYROLL",
            vec![
                posting("assets:bank:checking", json!([amount("")])),
                posting("income:salary", json!([amount("")])),
            ],
        ),
        // hledger's uncategorised fallback, and an empty payee.
        transaction(
            "  ",
            vec![
                posting("assets:bank:checking", json!([amount("$")])),
                posting("expenses:unknown", json!([amount("$")])),
            ],
        ),
    ]);

    let signals = signals_from_hledger_json(&json, Some("$")).expect("well-formed");
    assert_eq!(
        signals,
        Signals {
            txns: 4,
            postings: 7,
            amountless_postings: 1,
            bare_commodity_amounts: 2,
            unknown_accounts: 1,
            empty_descriptions: 1,
            // Not derivable from hledger's output — see `Signals::with_prefilter`.
            column_count_matches: false,
            header_matches_source: false,
        }
    );
}

#[test]
fn an_expected_commodity_widens_the_trap_to_the_wrong_commodity() {
    let json = json!([transaction(
        "FX",
        vec![posting("assets:bank:checking", json!([amount("EUR")]))],
    )]);

    // Not told what to expect: EUR is a commodity, so nothing is wrong with it.
    let unaware = signals_from_hledger_json(&json, None).expect("well-formed");
    assert_eq!(unaware.bare_commodity_amounts, 0);

    // Told the import is in $: EUR is exactly as invisible to the $ balance as a
    // bare amount would be, and is counted the same.
    let aware = signals_from_hledger_json(&json, Some("$")).expect("well-formed");
    assert_eq!(aware.bare_commodity_amounts, 1);

    // A blank expectation is not an expectation.
    let blank = signals_from_hledger_json(&json, Some("   ")).expect("well-formed");
    assert_eq!(blank.bare_commodity_amounts, 0);
}

#[test]
fn a_shape_we_do_not_understand_is_an_error_and_never_a_clean_score() {
    // The trap one level up: returning zeroed signals for output we failed to
    // read would score an unreadable candidate as PERFECT.
    assert_eq!(
        signals_from_hledger_json(&json!({}), None),
        Err(MatchError::NotTransactions)
    );
    assert_eq!(
        signals_from_hledger_json(&json!("nope"), None),
        Err(MatchError::NotTransactions)
    );
    assert_eq!(
        signals_from_hledger_json(&json!([{ "tdescription": "x" }]), None),
        Err(MatchError::MalformedTransaction)
    );
    assert_eq!(
        signals_from_hledger_json(
            &json!([transaction("x", vec![json!({ "paccount": "a" })])]),
            None
        ),
        Err(MatchError::MalformedPosting),
        "an ABSENT pamount is a shape change; an EMPTY one is the fact-4 signal"
    );
    assert_eq!(
        signals_from_hledger_json(
            &json!([transaction("x", vec![json!({ "pamount": [] })])]),
            None
        ),
        Err(MatchError::MalformedPosting)
    );
    // An empty journal is well-formed and simply has nothing in it.
    assert_eq!(
        signals_from_hledger_json(&json!([]), None),
        Ok(Signals::default())
    );
}

#[test]
fn an_absent_or_odd_description_counts_as_empty_rather_than_erroring() {
    // The one field with a safe default: "no payee" is a real reading of a
    // missing payee, and it LOWERS the score rather than raising it.
    let json = json!([
        json!({ "tpostings": [posting("a:b", json!([amount("$")]))] }),
        json!({ "tdescription": Value::Null, "tpostings": [posting("a:b", json!([amount("$")]))] }),
    ]);
    let signals = signals_from_hledger_json(&json, None).expect("well-formed");
    assert_eq!(signals.empty_descriptions, 2);
    assert_eq!(signals.txns, 2);
}

// ---------------------------------------------------------------------------
// The goldens — real hledger output, committed
// ---------------------------------------------------------------------------

/// What each golden must say, and the commodity the caller would have declared.
///
/// These numbers are the empirical finding. `garbage-success` and `no-currency`
/// are files hledger accepts with exit 0.
const GOLDEN_SIGNALS: &[(&str, Option<&str>, Signals)] = &[
    (
        "checking",
        Some("$"),
        Signals {
            txns: 5,
            postings: 10,
            amountless_postings: 0,
            bare_commodity_amounts: 0,
            unknown_accounts: 0,
            empty_descriptions: 0,
            column_count_matches: false,
            header_matches_source: false,
        },
    ),
    (
        "creditcard",
        Some("$"),
        Signals {
            txns: 4,
            postings: 8,
            amountless_postings: 0,
            bare_commodity_amounts: 0,
            unknown_accounts: 0,
            empty_descriptions: 0,
            column_count_matches: false,
            header_matches_source: false,
        },
    ),
    (
        "garbage-success",
        Some("$"),
        Signals {
            txns: 5,
            // Four of the five rows have an empty Deposit cell, so hledger writes
            // a lone posting with NO AMOUNT for each of them.
            postings: 6,
            amountless_postings: 4,
            bare_commodity_amounts: 0,
            unknown_accounts: 1,
            empty_descriptions: 0,
            column_count_matches: false,
            header_matches_source: false,
        },
    ),
    (
        "no-currency",
        // The caller expects `$` because that is what the journal is in; the
        // rules file's missing `currency` line is exactly why nothing is.
        Some("$"),
        Signals {
            txns: 5,
            postings: 10,
            amountless_postings: 0,
            bare_commodity_amounts: 10,
            unknown_accounts: 0,
            empty_descriptions: 0,
            column_count_matches: false,
            header_matches_source: false,
        },
    ),
];

#[test]
fn the_goldens_carry_the_signals_they_were_authored_to_carry() {
    for (stem, commodity, expected) in GOLDEN_SIGNALS {
        let signals = signals_from_hledger_json(&golden(stem), *commodity)
            .unwrap_or_else(|e| panic!("{stem}: {e}"));
        assert_eq!(&signals, expected, "{stem}");
    }
}

#[test]
fn the_bare_commodity_trap_is_caught_even_with_no_expectation_declared() {
    // `acommodity: ""` is bare on its own terms. The caller not knowing what
    // commodity to expect must not make the trap invisible.
    let signals = signals_from_hledger_json(&golden("no-currency"), None).expect("well-formed");
    assert_eq!(signals.bare_commodity_amounts, 10);
}

#[test]
fn hledger_accepting_a_rules_file_does_not_make_it_a_match() {
    // FACT 4, asserted directly, and end to end: each candidate goes through the
    // real stage 1 against the real fixture and then through stage 2 over real
    // hledger output. All three survive stage 1 — nothing cheap separates them —
    // and hledger exited 0 on all three.
    let data = table("checking.csv");
    let scored = |rules_name: &str, stem: &str| {
        let pass = prefilter(&rules(rules_name), &data)
            .unwrap_or_else(|| panic!("{rules_name} must survive stage 1"));
        let signals = signals_from_hledger_json(&golden(stem), pass.expected_commodity.as_deref())
            .expect("well-formed")
            .with_prefilter(&pass);
        score(&signals)
    };

    let correct = scored("checking.csv.rules", "checking");
    let garbage = scored("garbage-success.rules", "garbage-success");
    let no_currency = scored("no-currency.rules", "no-currency");

    assert_eq!(correct, Score::new(1.0), "nothing was wrong with this one");
    assert!(
        garbage.value() < 0.25,
        "amountless postings must not rate as a match: {garbage:?}"
    );
    assert_eq!(
        no_currency,
        Score::ZERO,
        "every amount in a commodity of its own is a total failure, not a partial one"
    );
    assert!(garbage < correct && no_currency < correct);
}

// ---------------------------------------------------------------------------
// Scoring and ranking
// ---------------------------------------------------------------------------

/// Build a candidate from a golden, as the server lane will.
fn candidate(id: &str, stem: &str, commodity: Option<&str>, column_match: bool) -> Candidate {
    let signals = signals_from_hledger_json(&golden(stem), commodity)
        .expect("well-formed")
        .with_prefilter(&ledgeline_core::rules::matching::PrefilterPass {
            column_count_matches: column_match,
            header_matches_source: true,
            ..Default::default()
        });
    Candidate {
        id: id.to_string(),
        label: id.trim_end_matches(".rules").to_string(),
        score: score(&signals),
        signals,
    }
}

#[test]
fn the_right_rules_file_ranks_first_and_the_impostors_rank_last() {
    let at = |seconds: u64| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
    // Deliberately adversarial ordering: the two impostors are the most recently
    // modified files in the directory, so a ranking that leaned on mtime first
    // would put them on top.
    let mut rankings = vec![
        Ranking {
            candidate: candidate("garbage-success.rules", "garbage-success", Some("$"), true),
            modified: at(3_000),
        },
        Ranking {
            candidate: candidate("no-currency.rules", "no-currency", Some("$"), true),
            modified: at(4_000),
        },
        Ranking {
            candidate: candidate("checking.csv.rules", "checking", Some("$"), true),
            modified: at(1_000),
        },
    ];
    rank(&mut rankings);

    let order: Vec<&str> = rankings
        .iter()
        .map(|ranking| ranking.candidate.id.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "checking.csv.rules",
            "garbage-success.rules",
            "no-currency.rules"
        ],
        "score decides first; mtime only breaks ties"
    );
    assert_eq!(rankings[0].candidate.score, Score::new(1.0));
    assert!(rankings[1].candidate.score.value() < 0.25);
    assert_eq!(rankings[2].candidate.score, Score::ZERO);
}

#[test]
fn equal_scores_are_broken_by_the_most_recently_touched_file() {
    // The real case: several years of near-identical rules files that score the
    // same, where the one still in use is the one most recently edited. No
    // filename is ever parsed for a year.
    let at = |seconds: u64| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
    let same = |id: &str| candidate(id, "checking", Some("$"), true);
    let mut rankings = vec![
        Ranking {
            candidate: same("checking-2024.csv.rules"),
            modified: at(1_000),
        },
        Ranking {
            candidate: same("checking-2026.csv.rules"),
            modified: at(3_000),
        },
        Ranking {
            candidate: same("checking-2025.csv.rules"),
            modified: at(2_000),
        },
        Ranking {
            candidate: same("checking-undated.csv.rules"),
            modified: None,
        },
    ];
    rank(&mut rankings);

    let order: Vec<&str> = rankings
        .iter()
        .map(|ranking| ranking.candidate.id.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "checking-2026.csv.rules",
            "checking-2025.csv.rules",
            "checking-2024.csv.rules",
            // Absent is not evidence of recency, so it sorts last.
            "checking-undated.csv.rules",
        ]
    );
}

#[test]
fn the_id_makes_the_order_total_so_two_runs_agree() {
    let mut rankings: Vec<Ranking> = ["c.rules", "a.rules", "b.rules"]
        .iter()
        .map(|id| Ranking {
            candidate: candidate(id, "checking", Some("$"), true),
            modified: None,
        })
        .collect();
    rank(&mut rankings);
    let order: Vec<&str> = rankings
        .iter()
        .map(|ranking| ranking.candidate.id.as_str())
        .collect();
    assert_eq!(order, ["a.rules", "b.rules", "c.rules"]);
}

#[test]
fn a_rules_file_that_produced_nothing_is_not_a_match_at_any_score() {
    assert_eq!(score(&Signals::default()), Score::ZERO);
}

#[test]
fn the_shape_flags_can_break_a_tie_and_never_decide_one() {
    let clean = Signals {
        txns: 5,
        postings: 10,
        ..Signals::default()
    };
    let corroborated = Signals {
        column_count_matches: true,
        header_matches_source: true,
        ..clean.clone()
    };
    let uncorroborated = clean.clone();
    assert!(
        score(&corroborated) > score(&uncorroborated),
        "it breaks ties"
    );
    // But an uncorroborated CLEAN candidate still beats a corroborated broken one
    // by a wide margin — the shape terms cannot overturn what hledger produced.
    let broken = Signals {
        amountless_postings: 5,
        column_count_matches: true,
        header_matches_source: true,
        ..clean
    };
    assert!(score(&uncorroborated) > score(&broken));
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// An arbitrary tally. Counts are allowed to exceed their denominators, because
/// nothing stops hledger from producing more amounts than postings and a score
/// must stay in range regardless.
fn any_signals() -> impl Strategy<Value = Signals> {
    (
        0usize..40,
        0usize..80,
        0usize..80,
        0usize..80,
        0usize..80,
        0usize..40,
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(txns, postings, amountless, bare, unknown, empty, columns, header)| Signals {
                txns,
                postings,
                amountless_postings: amountless,
                bare_commodity_amounts: bare,
                unknown_accounts: unknown,
                empty_descriptions: empty,
                column_count_matches: columns,
                header_matches_source: header,
            },
        )
}

proptest! {
    /// The contract [`score`]'s documentation claims, asserted rather than argued.
    #[test]
    fn increasing_any_penalty_never_raises_the_score(
        base in any_signals(),
        which in 0usize..4,
        bump in 1usize..30,
    ) {
        let mut worse = base.clone();
        match which {
            0 => worse.amountless_postings += bump,
            1 => worse.bare_commodity_amounts += bump,
            2 => worse.unknown_accounts += bump,
            _ => worse.empty_descriptions += bump,
        }
        prop_assert!(
            score(&worse) <= score(&base),
            "penalty {which} +{bump}: {:?} -> {:?}",
            score(&base),
            score(&worse)
        );
    }

    /// Clearing a corroborating flag is the same obligation.
    #[test]
    fn clearing_a_shape_flag_never_raises_the_score(base in any_signals()) {
        let no_columns = Signals { column_count_matches: false, ..base.clone() };
        let no_header = Signals { header_matches_source: false, ..base.clone() };
        let neither = Signals { column_count_matches: false, header_matches_source: false, ..base.clone() };
        prop_assert!(score(&no_columns) <= score(&base));
        prop_assert!(score(&no_header) <= score(&base));
        prop_assert!(score(&neither) <= score(&base));
    }

    /// The newtype's invariant, over arbitrary input: in range, and never `NaN`.
    #[test]
    fn a_score_is_always_a_usable_number(signals in any_signals()) {
        let value = score(&signals).value();
        prop_assert!(value.is_finite(), "{value}");
        prop_assert!((0.0..=1.0).contains(&value), "{value}");
    }

    /// Either fact-4 signal, at any rate at all, caps the score.
    #[test]
    fn any_silently_broken_output_is_never_a_confident_match(
        signals in any_signals(),
        amountless in 1usize..30,
    ) {
        let broken = Signals { amountless_postings: amountless, ..signals };
        prop_assert!(broken.txns == 0 || score(&broken).value() <= 0.25);
    }
}

// ---------------------------------------------------------------------------
// The live check — opt-in, so `cargo test` stays hermetic
// ---------------------------------------------------------------------------

/// Re-run hledger over the fixtures and require the committed goldens to still
/// be what it produces.
///
/// Default-skipped for the same reason as `rules_hledger_render.rs`: `cargo test`
/// must be runnable without hledger installed. Set
/// `LEDGELINE_HLEDGER_MATCH_CHECK=1` to run it. When it fails after an hledger
/// upgrade, rerun `scripts/gen-match-golden.sh` and **review the diff** — a change
/// here is a change in the contract stage 2 reads.
///
/// Runs hledger only against the committed fixtures, never a user's file: a
/// rules file's `source` directive accepts a `| CMD` form hledger executes.
#[test]
fn hledger_still_produces_the_committed_goldens() {
    if std::env::var_os(OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {OPT_IN}=1 to compare the goldens against a live hledger");
        return;
    }

    let cases = [
        ("checking", "checking.csv", "checking.csv.rules"),
        ("creditcard", "creditcard.csv", "creditcard.csv.rules"),
        ("garbage-success", "checking.csv", "garbage-success.rules"),
        ("no-currency", "checking.csv", "no-currency.rules"),
    ];

    for (stem, data, rules_name) in cases {
        let output = std::process::Command::new("hledger")
            .arg("print")
            .arg("-f")
            .arg(match_dir().join(data))
            .arg("--rules")
            .arg(match_dir().join(rules_name))
            .args(["-O", "json"])
            .output()
            .unwrap_or_else(|e| panic!("{stem}: could not run hledger: {e}"));
        assert!(
            output.status.success(),
            "{stem}: hledger exited {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        let live: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{stem}: hledger's output is not JSON: {e}"));
        // Compared on SIGNALS, not on bytes: the goldens have their absolute
        // `sourceName` stripped (see the generator), and `tsourcepos` is the one
        // part of this JSON the module is forbidden to read anyway.
        let expected = GOLDEN_SIGNALS
            .iter()
            .find(|(name, _, _)| *name == stem)
            .map(|(_, commodity, signals)| (*commodity, signals))
            .unwrap_or_else(|| panic!("{stem} has no expectation"));
        assert_eq!(
            &signals_from_hledger_json(&live, expected.0).unwrap_or_else(|e| panic!("{stem}: {e}")),
            expected.1,
            "{stem}: live hledger disagrees with the committed golden"
        );
    }
    eprintln!("{} goldens confirmed against a live hledger", cases.len());
}
