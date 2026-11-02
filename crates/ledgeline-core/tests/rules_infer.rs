//! Rules-file inference for journal transactions (plan 32 D27), over the
//! committed tree `fixtures/review-rules/`: one test per selection rule.
//!
//! The README there maps each transaction to the rule it exercises.

mod common;

use ledgeline_core::model::{Journal, Transaction};
use ledgeline_core::parse_journal;
use ledgeline_core::rules::RulesDoc;
use ledgeline_core::rules::discover;
use ledgeline_core::rules::infer::{
    IndexedRules, RulesCandidate, RulesIndex, RulesInference, effective_account1, journal_matches,
    rules_for,
};
use ledgeline_core::view::Classes;

struct Fixture {
    journal: Journal,
    classes: Classes,
    index: RulesIndex,
}

fn fixture() -> Fixture {
    let main = common::fixtures_dir().join("review-rules/main.journal");
    let text = std::fs::read_to_string(&main).expect("fixture readable");
    let journal = parse_journal(&text, &main.to_string_lossy()).expect("fixture parses");
    let classes = Classes::of(&journal);
    let index = RulesIndex::build(&discover(&main));
    Fixture {
        journal,
        classes,
        index,
    }
}

impl Fixture {
    fn txn(&self, description: &str) -> &Transaction {
        self.journal
            .transactions
            .iter()
            .find(|txn| txn.description == description)
            .unwrap_or_else(|| panic!("no {description} in the fixture"))
    }

    fn infer(&self, description: &str) -> RulesInference {
        rules_for(self.txn(description), &self.classes, &self.index)
    }

    fn candidates(&self, description: &str) -> Vec<RulesCandidate> {
        self.infer(description).candidates
    }
}

fn ids(candidates: &[RulesCandidate]) -> Vec<&str> {
    candidates.iter().map(|c| c.id.as_str()).collect()
}

fn recommended(candidates: &[RulesCandidate]) -> Option<&str> {
    let picked: Vec<&str> = candidates
        .iter()
        .filter(|c| c.recommended)
        .map(|c| c.id.as_str())
        .collect();
    assert!(picked.len() <= 1, "at most one recommendation: {picked:?}");
    picked.first().copied()
}

#[test]
fn the_index_reads_account1_fitid_fields_and_latest_markers() {
    let fixture = fixture();
    let citi = fixture
        .index
        .get("import/citi-2026.csv.rules")
        .expect("discovered");
    assert_eq!(
        citi.account1.as_deref(),
        Some("liabilities:credit-card:citi")
    );
    assert!(!citi.writes_id);
    assert!(citi.names_description);
    assert_eq!(citi.latest.as_deref(), Some("2026-03-15"));
    // The 2025 statement is gone; its marker is still found by the name.
    let old = fixture
        .index
        .get("import/citi-2025.csv.rules")
        .expect("discovered");
    assert_eq!(old.latest.as_deref(), Some("2025-12-30"));
    let chase = fixture
        .index
        .get("import/chase.csv.rules")
        .expect("discovered");
    assert!(chase.writes_id);
    assert_eq!(chase.latest, None);
    // A repeated date is one date.
    let venmo = fixture
        .index
        .get("import/venmo.csv.rules")
        .expect("discovered");
    assert_eq!(venmo.latest.as_deref(), Some("2026-06-30"));
}

#[test]
fn one_file_for_the_account_is_recommended_with_the_description_field() {
    let fixture = fixture();
    let inferred = fixture.infer("CHASE THING");
    assert!(!inferred.needs_choice);
    let found = inferred.candidates;
    assert_eq!(ids(&found), ["import/chase.csv.rules"]);
    assert_eq!(recommended(&found), Some("import/chase.csv.rules"));
    let chase = &found[0];
    assert_eq!(chase.match_field.as_deref(), Some("description"));
    assert_eq!(chase.account1, "assets:bank:chase");
    assert_eq!(chase.family_size, 1);
    assert_eq!(chase.year, None);
    assert!(
        chase
            .reasons
            .iter()
            .any(|r| r.contains("assets:bank:chase")),
        "{:?}",
        chase.reasons
    );
    assert!(
        chase.reasons.iter().any(|r| r.contains("id:")),
        "{:?}",
        chase.reasons
    );
}

#[test]
fn no_file_for_the_account_means_no_candidates() {
    let inferred = fixture().infer("CORNER STORE");
    assert!(inferred.candidates.is_empty());
    assert!(!inferred.needs_choice, "nothing to choose from");
}

#[test]
fn the_transactions_year_picks_within_a_family() {
    let fixture = fixture();
    let new = fixture.candidates("BLUE BOTTLE COFFEE 4412");
    assert_eq!(recommended(&new), Some("import/citi-2026.csv.rules"));
    assert_eq!(new[0].year, Some(2026));
    assert_eq!(new[0].family, "citi");
    assert_eq!(new[0].family_size, 2);
    assert!(new[0].reasons.iter().any(|r| r.contains("Named for 2026")));

    // A 2025 transaction goes to the 2025 file, and the newer one is offered
    // second, for importing ahead.
    // The importing-ahead alternative is only listed: it does not make the
    // choice ambiguous.
    let inferred = fixture.infer("BLUE BOTTLE COFFEE 1001");
    assert!(!inferred.needs_choice);
    let old = inferred.candidates;
    assert_eq!(
        ids(&old),
        ["import/citi-2025.csv.rules", "import/citi-2026.csv.rules"]
    );
    assert_eq!(recommended(&old), Some("import/citi-2025.csv.rules"));
    assert!(old[1].reasons.iter().any(|r| r.contains("importing ahead")));
}

#[test]
fn sibling_year_directories_are_one_family_and_the_newest_is_second() {
    let found = fixture().candidates("SCHWAB FEE");
    assert_eq!(
        ids(&found),
        [
            "bank/2026/schwab.csv.rules",
            "bank/2027/schwab.csv.rules",
            "bank/2025/schwab.csv.rules"
        ]
    );
    assert_eq!(recommended(&found), Some("bank/2026/schwab.csv.rules"));
    assert!(
        found
            .iter()
            .all(|c| c.family == "schwab" && c.family_size == 3)
    );
}

#[test]
fn two_families_for_one_account_are_a_tie() {
    let inferred = fixture().infer("AMEX STORE");
    assert!(inferred.needs_choice, "the user picks");
    let found = inferred.candidates;
    assert_eq!(
        ids(&found),
        ["import/amex.csv.rules", "import/amex-business.csv.rules"],
        "families in label order"
    );
    assert_eq!(recommended(&found), None);
    assert!(
        found
            .iter()
            .all(|c| c.reasons.iter().any(|r| r.contains("fits as well")))
    );
}

#[test]
fn the_fitid_convention_breaks_a_tie_both_ways() {
    let fixture = fixture();
    assert_eq!(
        recommended(&fixture.candidates("WELLS ATM")),
        Some("import/wells-ofx.csv.rules"),
        "an id: tag came from the file that writes one"
    );
    assert_eq!(
        recommended(&fixture.candidates("WELLS BRANCH")),
        Some("import/wells.csv.rules"),
        "no id: tag came from the file that writes none"
    );
    // Both stay offered.
    assert_eq!(fixture.candidates("WELLS ATM").len(), 2);
}

#[test]
fn a_latest_marker_before_the_date_rules_a_file_out_of_a_tie() {
    let inferred = fixture().infer("VENMO FRIEND");
    assert!(!inferred.needs_choice);
    let found = inferred.candidates;
    assert_eq!(recommended(&found), Some("import/venmo.csv.rules"));
    assert_eq!(found[0].id, "import/venmo.csv.rules");
    let legacy = found
        .iter()
        .find(|c| c.id == "import/venmo-legacy.csv.rules")
        .expect("still offered");
    assert!(
        legacy
            .reasons
            .iter()
            .any(|r| r.contains("before this transaction")),
        "{:?}",
        legacy.reasons
    );
}

#[test]
fn the_journal_fallback_counts_the_accounts_transactions() {
    let fixture = fixture();
    let found = journal_matches(
        &fixture.journal,
        &fixture.classes,
        "assets:bank:schwab",
        &["schwab.*fee"],
    )
    .expect("compiles");
    assert_eq!((found.matched, found.total), (1, 2));
    let wide = journal_matches(
        &fixture.journal,
        &fixture.classes,
        "assets:bank:schwab",
        &["SCHWAB"],
    )
    .expect("compiles");
    assert_eq!((wide.matched, wide.total), (2, 2));
    assert!(
        journal_matches(
            &fixture.journal,
            &fixture.classes,
            "assets:bank:schwab",
            &["("]
        )
        .is_err()
    );
}

#[test]
fn an_included_account1_is_not_seen_through_the_include() {
    // `paypal.csv.rules` takes its account1 from an include, which inference
    // does not follow; the included file itself is the candidate.
    let fixture = fixture();
    let paypal = fixture
        .index
        .get("import/paypal.csv.rules")
        .expect("discovered");
    assert_eq!(paypal.account1, None);
    assert_eq!(
        effective_account1(&ledgeline_core::rules::RulesDoc::parse("include x.rules\n")),
        None
    );
}

#[test]
fn a_family_the_year_rule_cannot_separate_needs_a_choice() {
    // One family (`<year>/bank-<year>.csv.rules`), two files named for 2026,
    // no markers, no mtimes: only the id tells them apart, which is no rule.
    let doc = RulesDoc::parse("fields date, description, amount\naccount1 assets:bank:chase\n");
    let twins = RulesIndex::new(
        ["2026/bank-2026.csv.rules", "2026-01/bank-2026.csv.rules"]
            .iter()
            .map(|id| IndexedRules::describe(id, &doc, None, None))
            .collect(),
    );
    let fixture = fixture();
    let inferred = rules_for(fixture.txn("CHASE THING"), &fixture.classes, &twins);
    assert_eq!(inferred.candidates.len(), 2);
    assert_eq!(recommended(&inferred.candidates), None);
    assert!(inferred.needs_choice);
    assert!(inferred.candidates.iter().all(|c| c.family_size == 2));

    // A marker separates them: decided.
    let marked = RulesIndex::new(vec![
        IndexedRules::describe(
            "2026/bank-2026.csv.rules",
            &doc,
            Some("2026-03-01".into()),
            None,
        ),
        IndexedRules::describe("2026-01/bank-2026.csv.rules", &doc, None, None),
    ]);
    let inferred = rules_for(fixture.txn("CHASE THING"), &fixture.classes, &marked);
    assert!(!inferred.needs_choice);
    assert_eq!(
        recommended(&inferred.candidates),
        Some("2026/bank-2026.csv.rules")
    );
}
