//! Sharing categorization between a manual rules file and a sync rules file
//! (plan 43 §3.2, contract WP-R): `rules::share` and the feed template.
//!
//! The hermetic half checks which blocks move and why, the names the sync CSV
//! borrows, and that the split is a pure partition of the original bytes. The
//! hledger half, gated behind `LEDGELINE_HLEDGER_SHARE_CHECK=1` like the other
//! `LEDGELINE_HLEDGER_*_CHECK` suites, proves the split changes nothing hledger
//! prints and that a sync-shaped CSV picks up the moved rules.
//!
//! Goldens live in `fixtures/import/shared-categories/split/`. To regenerate
//! after an intended change, run
//! `LEDGELINE_BLESS=1 cargo test -p ledgeline-core --test rules_share` and
//! review the diff. `just rules-check` runs hledger over the same files.

mod common;

use ledgeline_core::rules::generate::{DEFAULT_ACCOUNT2, FeedColumns, FeedRulesParams, feed_rules};
use ledgeline_core::rules::share::{KeepReason, ShareError, SharePlan, append_include, plan_share};
use proptest::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable that opts the hledger half in.
const OPT_IN: &str = "LEDGELINE_HLEDGER_SHARE_CHECK";

/// The two manual rules files, by stem.
const FIXTURES: [&str; 2] = ["wells-ofx", "firsttech-visa"];

fn dir() -> PathBuf {
    common::fixtures_dir().join("import/shared-categories")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{} readable: {e}", path.display()))
}

fn manual(stem: &str) -> String {
    read(&dir().join(format!("{stem}.csv.rules")))
}

fn categories_name(stem: &str) -> String {
    format!("{stem}.categories.rules")
}

fn plan(stem: &str) -> SharePlan {
    plan_share(&manual(stem), &categories_name(stem)).expect("the fixture shares")
}

/// `(header, reason)` for every kept block, for compact assertions.
fn kept(plan: &SharePlan) -> Vec<(&str, KeepReason)> {
    plan.kept
        .iter()
        .map(|block| (block.header.as_str(), block.reason.clone()))
        .collect()
}

fn not_in_sync(name: &str) -> KeepReason {
    KeepReason::FieldNotInSync(name.to_string())
}

/// The first seven names; `mcc`, `mcc_name` and `extra` keep their defaults.
fn columns(names: [&str; 7]) -> FeedColumns {
    let [date, amount, payee, description, memo, id, transacted] = names.map(str::to_string);
    FeedColumns {
        date,
        amount,
        payee,
        description,
        memo,
        id,
        transacted,
        ..FeedColumns::default()
    }
}

/// Undo a split: take the manual text before the include, and put each moved
/// item back at its original offset, reading its bytes from the CATEGORIES
/// file. Equal to the original only if the categories file holds the moved
/// items verbatim, in order, and nothing else moved.
fn reassemble(plan: &SharePlan) -> String {
    let remaining = &plan.manual_new_text[..plan.include_offset];
    let header_end = plan
        .categories_text
        .find("\n\n")
        .or_else(|| plan.categories_text.find("\n\r\n").map(|at| at + 1))
        .expect("the categories header ends with a blank line")
        + 2;
    let mut categories = &plan.categories_text[header_end..];
    let mut out = String::new();
    let mut taken = 0;
    for span in &plan.moved_spans {
        let before = span.start - out.len();
        out.push_str(&remaining[taken..taken + before]);
        taken += before;
        let (moved, rest) = categories.split_at(span.len());
        out.push_str(moved);
        // A moved item that ended the file without a terminator got one.
        categories = if moved.ends_with('\n') {
            rest
        } else {
            rest.trim_start_matches(['\r', '\n'])
        };
    }
    out.push_str(&remaining[taken..]);
    assert!(
        categories.is_empty(),
        "nothing but moved items: {categories:?}"
    );
    out
}

fn compare_or_bless(path: &Path, actual: &str) {
    if std::env::var_os("LEDGELINE_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(path, actual).expect("write golden");
        return;
    }
    assert_eq!(
        read(path),
        actual,
        "{} changed; if intended, re-run with LEDGELINE_BLESS=1 and review the diff",
        path.display()
    );
}

// ---------------------------------------------------------------------------
// Which blocks move
// ---------------------------------------------------------------------------

#[test]
fn the_ofx_file_moves_its_payee_rules_and_keeps_the_rest_with_reasons() {
    let plan = plan("wells-ofx");
    assert_eq!(plan.moved, 6, "{}", plan.categories_text);
    assert_eq!(
        kept(&plan),
        [
            ("if %trntype CHECK", not_in_sync("trntype")),
            ("if,account2,comment", KeepReason::Opaque),
            ("if %name PAYPAL", KeepReason::Opaque),
            ("if %name ATM", not_in_sync("fitid")),
            ("if ^2024-01-0", KeepReason::LayoutDependentPattern),
            ("if %name VENMO", KeepReason::SetsComment),
            (
                "if ONLINE TRANSFER",
                KeepReason::OrderDependent("code".to_string())
            ),
            ("if %checknum [0-9]", not_in_sync("checknum")),
        ]
    );
    for moved in [
        "# groceries\nif WHOLE FOODS",
        "# coffee shops\nif\n%name STARBUCKS",
        "if %name ^AMAZON",
        "if %memo REFUND\n& %amount ^[0-9]",
        "if %name PENDING\n  skip",
        "if TRANSFER TO SAVINGS",
    ] {
        assert!(plan.categories_text.contains(moved), "{moved:?} moved");
        assert!(!plan.manual_new_text.contains(moved), "{moved:?} left");
    }
    assert!(
        plan.manual_new_text
            .ends_with("\n\ninclude wells-ofx.categories.rules\n")
    );
    assert_eq!(
        plan.columns,
        columns([
            "date",
            "amount",
            "name",
            "sfdescription",
            "memo",
            "id",
            "transacted"
        ])
    );
}

#[test]
fn the_bank_csv_borrows_its_own_names_and_keeps_its_us_date_matcher() {
    let plan = plan("firsttech-visa");
    assert_eq!(
        kept(&plan),
        [
            ("if %type Payment", not_in_sync("type")),
            ("if %posting_date ^12/", KeepReason::LayoutDependentPattern),
            ("if %description RECURRING", KeepReason::SetsComment),
            ("if %card 4321", not_in_sync("card")),
            ("if %merchant SHELL OIL", KeepReason::SetsComment),
        ]
    );
    assert_eq!(plan.moved, 2);
    // `description %merchant` makes `merchant` the payee, so the bank's own
    // `description` column is the sync description, and both are shareable.
    assert_eq!(
        plan.columns,
        columns([
            "posting_date",
            "amount",
            "merchant",
            "description",
            "memo",
            "id",
            "transacted"
        ])
    );
    // It sets the comment, so it stays — sync would lose its ids to it.
    assert!(
        plan.manual_new_text
            .contains("if %merchant SHELL OIL\n  comment fuel:%description\n")
    );
}

#[test]
fn a_block_setting_the_transaction_comment_stays_but_a_posting_comment_moves() {
    let text = "fields date, amount, name\ndescription %name\n\
                if COFFEE\n  comment coffee\n\n\
                if TEA\n  comment2 tea\n";
    let plan = plan_share(text, "x.categories.rules").expect("shares");
    assert_eq!(kept(&plan), [("if COFFEE", KeepReason::SetsComment)]);
    assert_eq!(plan.moved, 1);
    assert_eq!(
        KeepReason::SetsComment.to_string(),
        "Sets the transaction comment, which sync uses for its id."
    );
}

#[test]
fn a_moved_block_may_not_jump_a_later_kept_block_that_sets_the_same_field() {
    let text = "skip 1\nfields date, amount, name, trntype\ndescription %name\n\
                if COFFEE\n  account2 expenses:coffee\n  code c\n\
                if TEA\n  code t\n\
                if %trntype DEBIT\n  account2 expenses:debit\n\
                if JUICE\n  account2 expenses:juice\n";
    let plan = plan_share(text, "x.categories.rules").expect("shares");
    assert_eq!(
        kept(&plan),
        [
            (
                "if COFFEE",
                KeepReason::OrderDependent("account2".to_string())
            ),
            ("if %trntype DEBIT", not_in_sync("trntype")),
        ]
    );
    // TEA sets only `code`, so it may move past the `account2` block; JUICE
    // already came after it.
    assert_eq!(plan.moved, 2);
}

#[test]
fn the_ordering_guard_is_transitive_and_includes_are_walls() {
    let text = "fields date, amount, name, trntype\n\
                if A\n  code a\n\
                if B\n  code b\n  account2 x\n\
                if %trntype T\n  account2 y\n\
                include other.rules\n\
                if C\n  code c\n";
    let plan = plan_share(text, "x.categories.rules").expect("shares");
    // B stays because of the trntype block; A then stays because B (now kept)
    // sets `code` too. C is after the include, so nothing later stays.
    assert_eq!(
        kept(&plan).into_iter().map(|(h, _)| h).collect::<Vec<_>>(),
        ["if A", "if B", "if %trntype T"]
    );
    assert_eq!(plan.moved, 1);

    let walled = "fields date, amount, name\nif A\n  code a\ninclude other.rules\n";
    let plan = plan_share(walled, "x.categories.rules").expect("shares");
    assert_eq!(
        kept(&plan),
        [("if A", KeepReason::OrderDependent("code".to_string()))]
    );
}

#[test]
fn whole_record_patterns_move_unless_they_depend_on_layout() {
    // The movable one goes last: they all set `account2`, so anything that
    // moved from before a kept one would trip the ordering guard instead.
    let text = "fields date, amount, name\ndescription %name\n\
                if ^GROCER\n  account2 b\n\n\
                if GROCER$\n  account2 c\n\n\
                if 5,GROCER\n  account2 d\n\n\
                if \"GROCER\n  account2 e\n\n\
                if GROCER\n  account2 a\n";
    let plan = plan_share(text, "x.categories.rules").expect("shares");
    assert!(
        plan.kept
            .iter()
            .all(|block| block.reason == KeepReason::LayoutDependentPattern),
        "{:?}",
        plan.kept
    );
    assert_eq!((plan.moved, plan.kept.len()), (1, 4));
}

#[test]
fn numbered_and_unknown_field_references_stay() {
    let text = "fields date, amount, name\ndescription %name\n\
                if %3 X\n  account2 a\n\n\
                if %name Y\n  comment %2\n\n\
                if %name Z\n  comment %nosuch\n";
    let plan = plan_share(text, "x.categories.rules").expect("shares");
    assert_eq!(
        kept(&plan),
        [
            ("if %3 X", not_in_sync("3")),
            ("if %name Y", not_in_sync("2")),
            ("if %name Z", not_in_sync("nosuch")),
        ]
    );
}

// ---------------------------------------------------------------------------
// Borrowing names
// ---------------------------------------------------------------------------

fn borrowed(fields_and_rules: &str) -> FeedColumns {
    plan_share(fields_and_rules, "x.categories.rules")
        .expect("shares")
        .columns
}

#[test]
fn a_literal_description_column_is_the_payee_when_nothing_assigns_one() {
    assert_eq!(
        borrowed("fields date, amount, description\n"),
        columns([
            "date",
            "amount",
            "description",
            "sfdescription",
            "memo",
            "id",
            "transacted"
        ])
    );
}

#[test]
fn colliding_defaults_are_prefixed_until_unique() {
    assert_eq!(
        borrowed(
            "fields Date, id, transacted, Amount, desc, sfdescription, payee\ndescription %desc\n"
        ),
        columns([
            "Date",
            "Amount",
            "desc",
            "sfsfdescription",
            "memo",
            "sfid",
            "sftransacted"
        ])
    );
}

#[test]
fn borrowed_names_follow_assignments_and_refuse_hledger_s_own_field_names() {
    // A compound description borrows nothing, so its source column's matchers
    // stay; a `code` column would be read by hledger as the code field.
    let compound = plan_share(
        "fields date, amount, name, memo\ndescription %name %memo\nif %name X\n  account2 a\n",
        "x.categories.rules",
    )
    .expect("shares");
    assert_eq!(compound.columns.payee, "payee");
    assert_eq!(kept(&compound), [("if %name X", not_in_sync("name"))]);

    let code = borrowed("fields date, amount, code\ndescription %code\n");
    assert_eq!(code.payee, "payee");

    let assigned = borrowed("fields when, amt, who\ndate %when\namount %amt\ndescription %who\n");
    assert_eq!(
        (
            assigned.date.as_str(),
            assigned.amount.as_str(),
            assigned.payee.as_str()
        ),
        ("when", "amt", "who")
    );
}

// ---------------------------------------------------------------------------
// Errors and append_include
// ---------------------------------------------------------------------------

#[test]
fn a_plan_is_refused_for_the_reasons_it_should_be() {
    assert_eq!(
        plan_share("if X\n  account2 a\n", "x.categories.rules"),
        Err(ShareError::NoFieldsLine)
    );
    assert_eq!(
        plan_share(
            "fields date, amount\ninclude x.categories.rules\n",
            "x.categories.rules"
        ),
        Err(ShareError::AlreadyShared("x.categories.rules".to_string()))
    );
    assert_eq!(
        plan_share("fields date\n", "x.rules\ninclude /etc/passwd"),
        Err(ShareError::BadFileName)
    );
}

#[test]
fn append_include_is_idempotent_and_keeps_the_file_s_line_endings() {
    for text in [
        "fields a\n",
        "fields a",
        "fields a\r\nif X\r\n  account2 b\r\n",
        "if,account2\nX,y",
        "",
    ] {
        let once = append_include(text, "c.categories.rules").expect("appends");
        assert!(once.starts_with(text), "{text:?}: nothing before moves");
        assert_eq!(
            append_include(&once, "c.categories.rules").expect("appends"),
            once,
            "{text:?}"
        );
        if text.contains("\r\n") {
            assert!(once.ends_with("\r\n\r\ninclude c.categories.rules\r\n"));
        }
    }
    // A table ending the file gets the blank line its extent needs.
    assert_eq!(
        append_include("if,account2\nX,y", "c.rules").expect("appends"),
        "if,account2\nX,y\n\ninclude c.rules\n"
    );
    assert_eq!(
        append_include("fields a\n", "../c.rules"),
        Err(ShareError::BadFileName)
    );
}

// ---------------------------------------------------------------------------
// Byte preservation
// ---------------------------------------------------------------------------

#[test]
fn every_fixture_split_reassembles_to_the_original_bytes() {
    for stem in FIXTURES {
        let original = manual(stem);
        let plan = plan(stem);
        assert_eq!(reassemble(&plan), original, "{stem}");
        let suffix = &plan.manual_new_text[plan.include_offset..];
        assert_eq!(
            suffix.trim_start_matches('\n'),
            format!("include {}\n", categories_name(stem)),
            "{stem}"
        );
        assert!(
            plan.manual_new_text
                .contains(&format!("\n\ninclude {}", categories_name(stem))),
            "{stem}: a blank line before the include"
        );
    }
}

/// Pieces a realistic rules file is made of, each one line-terminated.
const PIECES: &[&str] = &[
    "# a comment\n",
    "\n",
    "account2 expenses:unknown\n",
    "include other.rules\n",
    "if %name COFFEE\n  account2 expenses:coffee\n",
    "if %trntype CHECK\n  account2 expenses:checks\n",
    "if GROCER\n  account2 expenses:food\n\n",
    "if ^2024\n  comment x\n",
    "if,account2,comment\nNETFLIX,expenses:subs,tv\n\n",
    "# coffee\nif\n%name PEETS\n%memo CARD\n  account2 expenses:coffee\n",
    "if %name A\n& ! %memo B\n  account2 x\n",
    "if %name X\n  comment m:%memo\n",
    "if %name X\n  code %checknum\n",
    "if %name P\n  skip\n",
    "if %amount ^-\n& %memo FEE\n  account2 expenses:fees\n  comment fee\n\n",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn any_split_is_a_partition_of_the_original(
        picks in proptest::collection::vec(0..PIECES.len(), 0..24),
        crlf in any::<bool>(),
        unterminated in any::<bool>(),
    ) {
        let body: String = picks.iter().map(|&i| PIECES[i]).collect();
        let mut text = format!(
            "skip 1\nfields date, amount, name, memo, trntype, checknum\ndescription %name\n{body}"
        );
        if unterminated {
            text = text.trim_end_matches('\n').to_string();
        }
        if crlf {
            text = text.replace('\n', "\r\n");
        }
        let plan = plan_share(&text, "x.categories.rules").expect("a fixture-shaped file shares");
        prop_assert_eq!(reassemble(&plan), text);
        prop_assert_eq!(plan.moved, plan.moved_spans.len());
        let ending = if crlf {
            "include x.categories.rules\r\n"
        } else {
            "include x.categories.rules\n"
        };
        prop_assert!(plan.manual_new_text.ends_with(ending));
    }
}

// ---------------------------------------------------------------------------
// Goldens: the split files and the feed template
// ---------------------------------------------------------------------------

/// The sync rules file for a shared fixture: the feed template over the
/// borrowed columns, including the categories file.
fn sync_rules(stem: &str, plan: &SharePlan, account: &str) -> String {
    let categories = categories_name(stem);
    feed_rules(&FeedRulesParams {
        label: "Checking ••1234",
        provider: "SimpleFIN",
        columns: &plan.columns,
        journal_account: account,
        fallback_account: DEFAULT_ACCOUNT2,
        categories_include: Some(&categories),
        currency: Some("$"),
    })
}

#[test]
fn the_split_files_match_their_goldens() {
    let split = dir().join("split");
    for (stem, account) in [
        ("wells-ofx", "assets:bank:wells:checking"),
        ("firsttech-visa", "liabilities:firsttech:visa"),
    ] {
        let plan = plan(stem);
        compare_or_bless(
            &split.join(format!("{stem}.csv.rules")),
            &plan.manual_new_text,
        );
        compare_or_bless(&split.join(categories_name(stem)), &plan.categories_text);
        compare_or_bless(
            &split.join(format!("{stem}.sync.csv.rules")),
            &sync_rules(stem, &plan, account),
        );
    }
}

#[test]
fn the_default_feed_template_matches_its_golden() {
    let text = feed_rules(&FeedRulesParams {
        label: "First Tech · Checking ••1234",
        provider: "SimpleFIN",
        columns: &FeedColumns::default(),
        journal_account: "assets:bank:firsttech:checking",
        fallback_account: DEFAULT_ACCOUNT2,
        categories_include: None,
        currency: None,
    });
    compare_or_bless(&dir().join("feed/checking.sync.csv.rules"), &text);
}

#[test]
fn the_feed_template_follows_the_plan_s_shape() {
    let text = feed_rules(&FeedRulesParams {
        label: "Checking ••1234",
        provider: "SimpleFIN",
        columns: &columns([
            "date",
            "amount",
            "name",
            "description",
            "memo",
            "id",
            "transacted",
        ]),
        journal_account: "assets:bank:firsttech:checking",
        fallback_account: "expenses:unknown",
        categories_include: Some("firsttech-checking.categories.rules"),
        currency: Some("$"),
    });
    assert_eq!(
        text,
        "# ledgeline: SimpleFIN sync rules for Checking ••1234\n\
         skip 1\n\
         fields date, amount, name, description, memo, id, transacted, mcc, mcc_name, extra\n\
         date-format %Y-%m-%d\n\
         currency $\n\
         account1 assets:bank:firsttech:checking\n\
         account2 expenses:unknown\n\
         description %name\n\
         comment id:%id\n\
         if %transacted .\n  date2 %transacted\n\
         include firsttech-checking.categories.rules\n\
         if %id .\n  comment id:%id\n\
         if %mcc_name .\n  comment id:%id, category:%mcc_name\n"
    );
}

#[test]
fn a_bank_supplied_label_cannot_start_a_rule_of_its_own() {
    let text = feed_rules(&FeedRulesParams {
        label: "Checking\nsource | rm -rf ~\r\x1b[2J",
        provider: "SimpleFIN\n",
        columns: &FeedColumns::default(),
        journal_account: "assets:bank",
        fallback_account: DEFAULT_ACCOUNT2,
        categories_include: None,
        currency: None,
    });
    let first = text.lines().next().expect("a header");
    assert_eq!(
        first,
        "# ledgeline: SimpleFIN sync rules for Checking source | rm -rf ~ [2J"
    );
    assert!(!text.lines().any(|line| line.starts_with("source")));
}

#[test]
fn borrowed_date_and_amount_names_are_assigned_explicitly() {
    let plan = plan("firsttech-visa");
    let text = sync_rules("firsttech-visa", &plan, "liabilities:firsttech:visa");
    assert!(text.contains("\ndate %posting_date\n"), "{text}");
    assert!(!text.contains("\namount %"), "{text}");
}

// ---------------------------------------------------------------------------
// hledger: the split prints identically, and sync picks up the moved rules
// ---------------------------------------------------------------------------

fn opted_in() -> bool {
    let on = std::env::var_os(OPT_IN).is_some_and(|value| !value.is_empty());
    if !on {
        eprintln!("skipping: set {OPT_IN}=1 to run the hledger share check");
    }
    on
}

/// `hledger --no-conf -I -f <csv> --rules <rules> print [extra]`, as bytes.
/// `--no-conf` and `-I` for the reasons `rules_generate.rs` gives.
fn hledger_print(csv: &Path, rules: &Path, extra: &[&str]) -> Vec<u8> {
    let output = Command::new("hledger")
        .arg("--no-conf")
        .arg("-I")
        .arg("-f")
        .arg(csv)
        .arg("--rules")
        .arg(rules)
        .arg("print")
        .args(extra)
        .output()
        .expect("hledger runs");
    assert!(
        output.status.success(),
        "hledger failed on {}:\n{}",
        rules.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// A fresh scratch directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ledgeline-share-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn hledger_prints_the_same_before_and_after_the_split() {
    if !opted_in() {
        return;
    }
    for stem in FIXTURES {
        let csv = dir().join(format!("{stem}.csv"));
        let before = hledger_print(&csv, &dir().join(format!("{stem}.csv.rules")), &[]);

        let work = scratch(stem);
        let plan = plan(stem);
        let rules = work.join(format!("{stem}.csv.rules"));
        std::fs::write(&rules, &plan.manual_new_text).expect("write manual");
        std::fs::write(work.join(categories_name(stem)), &plan.categories_text)
            .expect("write categories");
        let after = hledger_print(&csv, &rules, &[]);
        assert_eq!(
            String::from_utf8_lossy(&after),
            String::from_utf8_lossy(&before),
            "{stem}: the split changed what hledger prints"
        );
        let _ = std::fs::remove_dir_all(&work);
    }
}

#[test]
fn hledger_reads_the_committed_split_and_sync_files() {
    if !opted_in() {
        return;
    }
    let split = dir().join("split");
    for stem in FIXTURES {
        let csv = dir().join(format!("{stem}.csv"));
        let before = hledger_print(&csv, &dir().join(format!("{stem}.csv.rules")), &[]);
        let after = hledger_print(&csv, &split.join(format!("{stem}.csv.rules")), &[]);
        assert_eq!(after, before, "{stem}: the committed split prints the same");
    }
}

/// What hledger made of each sync row: description → (accounts, date2, comment).
fn by_description(json: &[u8]) -> Vec<(String, Vec<String>, Option<String>, String)> {
    let txns: Value = serde_json::from_slice(json).expect("hledger JSON");
    txns.as_array()
        .expect("an array")
        .iter()
        .map(|t| {
            (
                t["tdescription"].as_str().unwrap_or_default().to_string(),
                t["tpostings"]
                    .as_array()
                    .expect("postings")
                    .iter()
                    .map(|p| p["paccount"].as_str().unwrap_or_default().to_string())
                    .collect(),
                t["tdate2"].as_str().map(str::to_string),
                t["tcomment"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn a_sync_csv_with_borrowed_names_gets_the_moved_categorizations() {
    if !opted_in() {
        return;
    }
    let plan = plan("wells-ofx");
    let work = scratch("sync");
    std::fs::write(
        work.join(categories_name("wells-ofx")),
        &plan.categories_text,
    )
    .expect("write categories");
    let rules = work.join("wells-ofx.sync.csv.rules");
    std::fs::write(
        &rules,
        sync_rules("wells-ofx", &plan, "assets:bank:wells:checking"),
    )
    .expect("write sync rules");
    let header = plan.columns.in_order().join(",");
    let csv = work.join("wells-ofx.sync.csv");
    std::fs::write(
        &csv,
        format!(
            "{header}\n\
             2024-02-01,-54.20,WHOLE FOODS MARKET #9,WHOLE FOODS SEATTLE WA,,a1b2c3d4e5-1,\n\
             2024-02-02,-6.45,STARBUCKS,STARBUCKS 0042,,a1b2c3d4e5-2,\n\
             2024-02-03,25.00,AMAZON MKTP,AMAZON MKTP REFUND,REFUND,a1b2c3d4e5-3,2024-02-01\n\
             2024-02-04,-40.00,VENMO,VENMO PAYMENT,rent share,a1b2c3d4e5-4,\n\
             2024-02-05,-9.99,PENDING HOLD,PENDING HOLD,,a1b2c3d4e5-5,\n\
             2024-02-06,-15.49,NETFLIX.COM,NETFLIX.COM,,a1b2c3d4e5-6,\n"
        ),
    )
    .expect("write sync csv");

    let rows = by_description(&hledger_print(&csv, &rules, &["-O", "json"]));
    let account2 = |desc: &str| {
        rows.iter()
            .find(|row| row.0 == desc)
            .map(|row| row.1[1].clone())
            .unwrap_or_else(|| panic!("{desc} imported: {rows:?}"))
    };
    assert_eq!(account2("WHOLE FOODS MARKET #9"), "expenses:food:groceries");
    assert_eq!(account2("STARBUCKS"), "expenses:food:coffee");
    assert_eq!(account2("AMAZON MKTP"), "income:refunds");
    // The table stayed in the manual file, so sync doesn't see it.
    assert_eq!(account2("NETFLIX.COM"), DEFAULT_ACCOUNT2);
    assert!(
        rows.iter().all(|row| row.0 != "PENDING HOLD"),
        "the moved skip rule applies"
    );
    // The VENMO block sets the comment, so it stayed in the manual file and
    // the sync row keeps its id.
    let venmo = rows.iter().find(|row| row.0 == "VENMO").expect("venmo");
    assert_eq!(venmo.3, "id:a1b2c3d4e5-4");
    let starbucks = rows.iter().find(|row| row.0 == "STARBUCKS").expect("sbux");
    assert_eq!(starbucks.3, "id:a1b2c3d4e5-2");
    assert_eq!(starbucks.2, None, "no transacted date, no date2");
    let refund = rows
        .iter()
        .find(|row| row.0 == "AMAZON MKTP")
        .expect("refund");
    assert_eq!(refund.2.as_deref(), Some("2024-02-01"));
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn the_id_guard_beats_a_comment_rule_in_the_categories_file() {
    if !opted_in() {
        return;
    }
    let work = scratch("id-guard");
    std::fs::write(
        work.join("x.categories.rules"),
        "if VENMO\n  comment memo:%memo\n  account2 expenses:venmo\n",
    )
    .expect("write categories");
    let rules = work.join("x.sync.csv.rules");
    std::fs::write(
        &rules,
        feed_rules(&FeedRulesParams {
            label: "Checking",
            provider: "SimpleFIN",
            columns: &FeedColumns::default(),
            journal_account: "assets:bank:checking",
            fallback_account: DEFAULT_ACCOUNT2,
            categories_include: Some("x.categories.rules"),
            currency: Some("$"),
        }),
    )
    .expect("write sync rules");
    let csv = work.join("x.sync.csv");
    std::fs::write(
        &csv,
        "date,amount,payee,description,memo,id,transacted\n\
         2024-02-04,-40.00,VENMO,VENMO PAYMENT,rent share,abc-4,\n",
    )
    .expect("write csv");
    let rows = by_description(&hledger_print(&csv, &rules, &["-O", "json"]));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].3, "id:abc-4", "the id survives the shared comment");
    assert_eq!(rows[0].1[1], "expenses:venmo", "the category still applies");
    let printed = String::from_utf8(hledger_print(&csv, &rules, &[])).expect("utf8");
    assert!(printed.contains("$-40.00"), "currency applies: {printed}");
    let _ = std::fs::remove_dir_all(&work);
}
