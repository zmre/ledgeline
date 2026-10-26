//! Projection scenario FILES (plan 22, Phase 3): what the discovery scan will
//! and will not offer, and what the serializer writes.
//!
//! The scan's guards — the caps, the symlink refusal, the hidden skip, the
//! containment test — are all about a real filesystem, so they are exercised
//! against one here rather than unit-tested against a model of one. The
//! serializer's pure behaviour lives in its own module tests; what is here is
//! the half that needs a directory tree, plus the one check only the hledger
//! binary can make.
//!
//! # The hledger cross-check is opt-in
//!
//! Everything in this file except `hledger_reads_a_scenario_we_wrote` runs
//! under a bare `cargo test`. That one shells out to a binary that may not be
//! installed, so `cargo test` stays hermetic and `LEDGELINE_HLEDGER_PROJECTION_CHECK=1`
//! (via `just hledger-checks`) opts in — the same pattern
//! `rules_hledger_render.rs` and `budget_golden.rs` use.

use ledgeline_core::decimal::Dec;
use ledgeline_core::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, PriceDirective, Transaction,
};
use ledgeline_core::parse::parse_period_spec;
use ledgeline_core::projections::serialize::{
    ProjectionDoc, new_file, scenario_from_text, write_scenario,
};
use ledgeline_core::projections::{
    CounterDefaults, Forecast, ForecastOpts, ForecastWindow, History, forecast_cutoff,
    forecast_transactions, latest_real_date, merged_prices, merged_transactions,
    upgrade_legacy_counters,
};
use ledgeline_core::projections::{
    CreateRefusal, Growth, GrowthUnit, LineRole, LineSource, Scenario, ScenarioEvent, ScenarioLine,
    discover, slug_filename, virtual_posting,
};
use ledgeline_core::reports::account_types::AccountType;
use ledgeline_core::reports::{
    AccountTypes, BsOpts, BsSectionKind, BucketSpan, CfOpts, CfTags, IS_GROUP_TAG, Interval,
    IsOpts, MixedAmount, Valuation, account_decls, account_groups, account_sections, add_days,
    balance_sheet_columns, bs_terms, cash_flow_statement, declared_groups, declared_types,
    income_statement_columns, range_buckets,
};
use ledgeline_core::{Journal, parse_journal};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable that opts the hledger check in.
const OPT_IN: &str = "LEDGELINE_HLEDGER_PROJECTION_CHECK";

// ---------------------------------------------------------------------------
// Scratch trees
// ---------------------------------------------------------------------------

/// A temporary directory that removes itself on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ledgeline-projections-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        // Canonicalized, because `/tmp` is a symlink to `/private/tmp` on
        // macOS and the scan root is canonical. Comparing a non-canonical path
        // against it would fail for a reason that has nothing to do with the
        // guard under test.
        Self(dir.canonicalize().expect("scratch dir canonicalizes"))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Write `text` to `relative`, creating parent directories.
    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, text).expect("write fixture");
        path
    }

    /// The main journal every scan in this file is rooted at.
    fn main_journal(&self) -> PathBuf {
        self.write("main.journal", "; the main journal\n")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A one-line scenario file's text.
fn sample(name: &str) -> String {
    format!(
        "; Ledgeline projection\n; projection: {name}\n; created: 2026-01-01\n; updated: \
         2026-01-01\n\n~ monthly  plan\n    (expenses:rent)  $4200.00\n"
    )
}

fn usd(mantissa: i128, places: u32, precision: u32) -> Amount {
    Amount {
        commodity: Commodity("$".to_string()),
        quantity: Dec::new(mantissa, places),
        style: AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision,
        },
        cost: None,
    }
}

fn line(group: &str, id: &str, account: &str, amount: Amount, period: &str) -> ScenarioLine {
    ScenarioLine {
        counter: None,
        gains: None,
        id: id.to_string(),
        group: group.to_string(),
        role: LineRole::Flow,
        account: AccountName(account.to_string()),
        amount,
        period: parse_period_spec(period),
        growth: None,
        opening: None,
        note: String::new(),
        source: LineSource::Journal,
    }
}

/// What the main journal declares about account types. Empty is the ordinary
/// case for a projection file and is still a real classification: `AccountTypes`
/// infers from a root name when nothing is declared.
fn declared() -> BTreeMap<String, AccountType> {
    BTreeMap::new()
}

// ---------------------------------------------------------------------------
// Discovery: what is listed
// ---------------------------------------------------------------------------

#[test]
fn the_scan_lists_every_journal_and_puts_the_projections_first() {
    let scratch = Scratch::new("listing");
    let main = scratch.main_journal();
    scratch.write("2026.journal", "; a year file\n");
    scratch.write("plans/projection-series-a.journal", &sample("Series A"));
    scratch.write("projection-plan-b.journal", &sample("Plan B"));

    let discovery = discover(&main);
    let ids: Vec<&str> = discovery.files.iter().map(|f| f.id.as_str()).collect();
    // `projection-*` first, then everything else, each group by id.
    assert_eq!(
        ids,
        vec![
            "plans/projection-series-a.journal",
            "projection-plan-b.journal",
            "2026.journal",
            "main.journal",
        ]
    );
    assert!(!discovery.truncated);
    assert!(discovery.warnings.is_empty(), "{:?}", discovery.warnings);
}

#[test]
fn only_a_projection_file_has_its_header_read() {
    // "Reading every journal's head to display a picker is work nobody asked
    // for." A non-projection journal is listed with its filename label and no
    // header, even when it has one.
    let scratch = Scratch::new("headers");
    let main = scratch.main_journal();
    scratch.write("projection-a.journal", &sample("Named A"));
    scratch.write("other.journal", &sample("Named Other"));

    let discovery = discover(&main);
    let named = discovery.resolve("projection-a.journal").expect("listed");
    assert!(named.is_projection);
    assert_eq!(named.name.as_deref(), Some("Named A"));
    assert_eq!(named.created.as_deref(), Some("2026-01-01"));
    assert_eq!(named.label, "a");

    let other = discovery.resolve("other.journal").expect("listed");
    assert!(!other.is_projection);
    assert_eq!(other.name, None, "its header is deliberately not read");
    assert_eq!(other.label, "other");
}

#[test]
fn the_directories_offered_are_the_ones_that_hold_a_journal() {
    let scratch = Scratch::new("dirs");
    let main = scratch.main_journal();
    scratch.write("plans/projection-a.journal", &sample("A"));
    scratch.write("plans/deeper/projection-b.journal", &sample("B"));
    std::fs::create_dir_all(scratch.path().join("empty")).expect("mkdir");

    let discovery = discover(&main);
    assert_eq!(
        discovery.directories(),
        vec![
            "".to_string(),
            "plans".to_string(),
            "plans/deeper".to_string()
        ],
        "an empty directory is not somewhere a projection wants to go"
    );
}

// ---------------------------------------------------------------------------
// Discovery: the guards
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn every_symlink_is_refused_and_says_so() {
    let scratch = Scratch::new("symlink");
    let main = scratch.main_journal();
    let real = scratch.write("plans/projection-real.journal", &sample("Real"));
    std::os::unix::fs::symlink(&real, scratch.path().join("projection-link.journal"))
        .expect("symlink");

    let discovery = discover(&main);
    assert!(
        discovery.resolve("projection-link.journal").is_none(),
        "a symlink is never listed"
    );
    assert!(discovery.resolve("plans/projection-real.journal").is_some());
    assert!(
        discovery
            .warnings
            .iter()
            .any(|w| w.contains("projection-link.journal") && w.contains("symbolic link")),
        "{:?}",
        discovery.warnings
    );
}

#[test]
#[cfg(unix)]
fn a_symlinked_directory_is_not_descended_into() {
    // A directory link is the one that turns a walk into a cycle, so it is
    // refused for the same reason a file link is.
    let scratch = Scratch::new("dirlink");
    let main = scratch.main_journal();
    scratch.write("real/projection-inside.journal", &sample("Inside"));
    std::os::unix::fs::symlink(scratch.path().join("real"), scratch.path().join("linked"))
        .expect("symlink");

    let discovery = discover(&main);
    let ids: Vec<&str> = discovery.files.iter().map(|f| f.id.as_str()).collect();
    assert!(ids.contains(&"real/projection-inside.journal"));
    assert!(
        !ids.iter().any(|id| id.starts_with("linked/")),
        "reached through the link: {ids:?}"
    );
}

#[test]
fn hidden_entries_and_skipped_directories_are_left_out_silently() {
    let scratch = Scratch::new("hidden");
    let main = scratch.main_journal();
    scratch.write(".hidden.journal", &sample("Hidden"));
    scratch.write(".git/projection-inside-git.journal", &sample("Git"));
    scratch.write("node_modules/projection-dep.journal", &sample("Dep"));
    scratch.write("projection-visible.journal", &sample("Visible"));

    let discovery = discover(&main);
    let ids: Vec<&str> = discovery.files.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, vec!["projection-visible.journal", "main.journal"]);
    assert!(
        discovery.warnings.is_empty(),
        "a policy skip is not a problem to report: {:?}",
        discovery.warnings
    );
}

#[test]
fn the_depth_cap_truncates_rather_than_descending_forever() {
    let scratch = Scratch::new("depth");
    let main = scratch.main_journal();
    // `MAX_PROJECTION_DEPTH` is 8: a file at depth 8 is listed, one at depth 9
    // is not, and the listing says it is incomplete.
    let shallow = "a/b/c/d/e/f/g/h/projection-deep.journal";
    let deep = "a/b/c/d/e/f/g/h/i/projection-deeper.journal";
    scratch.write(shallow, &sample("Deep"));
    scratch.write(deep, &sample("Deeper"));

    let discovery = discover(&main);
    assert!(discovery.resolve(shallow).is_some(), "at the limit");
    assert!(discovery.resolve(deep).is_none(), "past the limit");
    assert!(
        discovery.truncated,
        "and the user is told the list is a subset"
    );
}

#[test]
fn a_file_cap_sets_truncated_rather_than_showing_a_subset_that_looks_complete() {
    // `MAX_PROJECTION_FILES` is 200. Writing 201 is cheap and pins the cap
    // itself rather than the constant's spelling.
    let scratch = Scratch::new("filecap");
    let main = scratch.main_journal();
    for i in 0..201 {
        scratch.write(&format!("projection-{i:04}.journal"), &sample("x"));
    }
    let discovery = discover(&main);
    assert_eq!(discovery.files.len(), 200);
    assert!(discovery.truncated);
}

#[test]
fn an_id_that_is_not_in_the_scanned_set_never_resolves() {
    let scratch = Scratch::new("resolve");
    let main = scratch.main_journal();
    scratch.write("projection-a.journal", &sample("A"));
    let discovery = discover(&main);

    // Resolution is EXACT STRING EQUALITY against the scan's set. None of
    // these is in it, so none of them resolves — not because they are
    // filtered, but because nothing like them is ever in the set.
    for id in [
        "../escape.journal",
        "/etc/passwd.journal",
        "./projection-a.journal",
        "PROJECTION-A.JOURNAL",
        "projection-a.journal/",
        "",
    ] {
        assert!(discovery.resolve(id).is_none(), "resolved `{id}`");
    }
    assert!(discovery.resolve("projection-a.journal").is_some());
}

#[test]
fn resolve_new_refuses_a_traversal_a_hidden_component_and_a_missing_directory() {
    let scratch = Scratch::new("resolvenew");
    let main = scratch.main_journal();
    scratch.write("plans/projection-a.journal", &sample("A"));
    let discovery = discover(&main);

    for (id, want) in [
        ("../outside.journal", CreateRefusal::Malformed),
        ("/absolute.journal", CreateRefusal::Malformed),
        (".hidden/p.journal", CreateRefusal::Malformed),
        ("node_modules/p.journal", CreateRefusal::Malformed),
        ("p.rules", CreateRefusal::Malformed),
        ("a\\b.journal", CreateRefusal::Malformed),
        ("", CreateRefusal::Malformed),
        // A real, non-hidden name below a directory that is not there. No
        // directory is ever created.
        ("nope/p.journal", CreateRefusal::DirectoryMissing),
        // Something is already at that name.
        ("plans/projection-a.journal", CreateRefusal::Exists),
    ] {
        assert_eq!(discovery.resolve_new(id), Err(want), "for `{id}`");
    }

    // And the one that works: a new name in a directory the scan found.
    let path = discovery
        .resolve_new("plans/projection-new.journal")
        .expect("a new file in a real directory");
    assert_eq!(
        path.as_path(),
        scratch.path().join("plans/projection-new.journal")
    );
}

#[test]
#[cfg(unix)]
fn resolve_new_refuses_a_symlinked_parent_directory() {
    let scratch = Scratch::new("newlink");
    let main = scratch.main_journal();
    scratch.write("real/projection-a.journal", &sample("A"));
    std::os::unix::fs::symlink(scratch.path().join("real"), scratch.path().join("linked"))
        .expect("symlink");

    let discovery = discover(&main);
    assert_eq!(
        discovery.resolve_new("linked/projection-b.journal"),
        Err(CreateRefusal::DirectoryMissing),
        "a symlinked directory is refused rather than followed"
    );
}

#[test]
fn no_string_the_scan_produces_contains_the_root() {
    // Layer 5: a dialog is a fine oracle for "does /Users/someone exist".
    let scratch = Scratch::new("nopaths");
    let main = scratch.main_journal();
    scratch.write("projection-a.journal", &sample("A"));
    #[cfg(unix)]
    {
        let fifo = scratch.path().join("projection-fifo.journal");
        // A FIFO produces a warning, which is the string most likely to leak.
        let _ = Command::new("mkfifo").arg(&fifo).status();
    }
    std::fs::create_dir_all(scratch.path().join("sub")).expect("mkdir");

    let discovery = discover(&main);
    let root = scratch.path().to_string_lossy().to_string();
    let label = discovery.root_label();
    assert!(!label.contains('/'), "the label is a component: {label}");
    for warning in &discovery.warnings {
        assert!(
            !warning.contains(&root),
            "warning leaked the root: {warning}"
        );
    }
    for file in &discovery.files {
        assert!(!file.id.contains(&root), "id leaked the root: {}", file.id);
        assert!(!file.label.contains(&root));
    }
}

// ---------------------------------------------------------------------------
// The name → filename slug
// ---------------------------------------------------------------------------

#[test]
fn a_slugged_filename_is_always_one_resolve_new_would_accept() {
    let scratch = Scratch::new("slug");
    let main = scratch.main_journal();
    let discovery = discover(&main);
    for name in [
        "Series A with a hiring ramp",
        "../../etc/passwd",
        "  ...  ",
        "UPPER CASE",
        "a/b/c",
        "tabs\tand\nnewlines",
    ] {
        let Some(filename) = slug_filename(name) else {
            continue;
        };
        assert!(
            discovery.resolve_new(&filename).is_ok(),
            "`{name}` slugged to `{filename}`, which resolve_new refused"
        );
    }
}

// ---------------------------------------------------------------------------
// Writing: the byte-level claim, end to end through a real file
// ---------------------------------------------------------------------------

#[test]
fn a_save_that_changes_one_amount_changes_only_that_line_s_bytes() {
    let scratch = Scratch::new("bytes");
    // A real multi-line literal, not a `\`-continued one: the continuation
    // escape eats the leading whitespace of the next line, and this file's
    // whole point is that leading whitespace survives.
    let before = concat!(
        "; Ledgeline projection\n",
        "; projection: Plan\n",
        "; created: 2026-01-01\n",
        "; updated: 2026-01-01\n",
        "\n",
        "; the user's own note\n",
        "account expenses:rent\n",
        "\n",
        "~ monthly  plan\n",
        "\t(expenses:rent)\t$4200.00\n",
        "\n",
        "~ monthly  plan\n",
        "    (expenses:food)          $600.00\n",
    );
    let path = scratch.write("projection-plan.journal", before);
    let name = path.to_string_lossy().to_string();

    let doc = ProjectionDoc::parse(before, &name, &declared()).expect("it parses");
    let mut scenario = scenario_from_text(before, &name, "plan", &declared()).expect("it reads");
    scenario.lines[1].amount = usd(65_000, 2, 2);
    let after = write_scenario(&doc, &scenario, &name).expect("it writes");

    // Everything outside the second block is the byte it was — including the
    // first rule's TABS, which a re-render would have turned into spaces.
    assert_eq!(
        after,
        before.replace(
            "    (expenses:food)          $600.00\n",
            "    (expenses:food)  $650.00\n"
        )
    );
    assert!(after.contains("\t(expenses:rent)\t$4200.00\n"), "{after:?}");

    // And it is still the same scenario, read back off a real file.
    std::fs::write(&path, &after).expect("write");
    let reread = scenario_from_text(
        &std::fs::read_to_string(&path).unwrap(),
        &name,
        "plan",
        &declared(),
    )
    .expect("it re-reads");
    assert_eq!(reread.lines[1].amount.quantity, Dec::new(65_000, 2));
}

#[test]
fn a_save_that_changes_nothing_produces_the_same_bytes() {
    // The no-op short-circuit every writer here has: a byte-identical result
    // is what lets the HTTP layer decline to write at all, which is what keeps
    // a user's own `entr` loop quiet.
    let scratch = Scratch::new("noop");
    let before = sample("Plan");
    let path = scratch.write("projection-plan.journal", &before);
    let name = path.to_string_lossy().to_string();

    let doc = ProjectionDoc::parse(&before, &name, &declared()).expect("it parses");
    let scenario = scenario_from_text(&before, &name, "plan", &declared()).expect("it reads");
    assert_eq!(
        write_scenario(&doc, &scenario, &name).expect("it writes"),
        before
    );
}

// ---------------------------------------------------------------------------
// The hledger cross-check
// ---------------------------------------------------------------------------

/// The scenario the cross-check writes: flat lines, a step, and both kinds of
/// one-off — everything the format has except growth, which hledger cannot
/// apply and which is asserted about separately below.
fn cross_check_scenario() -> Scenario {
    Scenario {
        cash: None,
        name: "Cross check".to_string(),
        created: Some("2026-01-01".to_string()),
        updated: Some("2026-01-02".to_string()),
        lines: vec![
            ScenarioLine {
                note: "projection".to_string(),
                ..line(
                    "rule:0",
                    "rule:0:0",
                    "expenses:rent",
                    usd(420_000, 2, 2),
                    "monthly",
                )
            },
            ScenarioLine {
                note: "projection".to_string(),
                // A growing line, so the file states a `growth:` tag hledger
                // has to be willing to read (and then ignore).
                growth: Some(Growth {
                    rate: Dec::new(3, 2),
                    unit: GrowthUnit::Year,
                }),
                ..line(
                    "rule:0",
                    "rule:0:1",
                    "revenues:salary",
                    usd(-1_200_000, 2, 2),
                    "monthly",
                )
            },
            ScenarioLine {
                note: "payroll".to_string(),
                ..line(
                    "rule:1",
                    "payroll",
                    "expenses:payroll",
                    usd(5_000_000, 2, 2),
                    "monthly to 2027-04-01",
                )
            },
            ScenarioLine {
                note: "payroll".to_string(),
                ..line(
                    "rule:2",
                    "payroll",
                    "expenses:payroll",
                    usd(12_000_000, 2, 2),
                    "monthly from 2027-04-01",
                )
            },
        ],
        events: vec![ScenarioEvent {
            counter: None,
            id: "rule:3".to_string(),
            date: "2027-03-01".to_string(),
            description: "Series A".to_string(),
            postings: vec![
                virtual_posting(
                    AccountName("assets:cash".to_string()),
                    usd(200_000_000, 2, 2),
                ),
                virtual_posting(
                    AccountName("equity:preferred".to_string()),
                    usd(-200_000_000, 2, 2),
                ),
            ],
        }],
    }
}

/// The scenario the ASSET cross-check writes: the format
/// `plans/23-asset-growth.md` verified, built from the model so that what
/// hledger reads is what this crate's own writer produces.
fn asset_scenario() -> Scenario {
    let growing = |rate: i128| Growth {
        rate: Dec::new(rate, 2),
        unit: GrowthUnit::Year,
    };
    Scenario {
        name: "Asset cross check".to_string(),
        lines: vec![
            // A balance that only compounds: the contribution is `$0`.
            ScenarioLine {
                role: LineRole::Asset,
                growth: Some(growing(7)),
                note: "projection".to_string(),
                ..line(
                    "rule:0",
                    "rule:0:0",
                    "assets:brokerage",
                    usd(0, 2, 2),
                    "monthly",
                )
            },
            // A balance that is paid into: the contribution IS the goal.
            ScenarioLine {
                role: LineRole::Asset,
                growth: Some(growing(4)),
                note: "projection".to_string(),
                ..line(
                    "rule:0",
                    "rule:0:1",
                    "assets:savings",
                    usd(200_000, 2, 2),
                    "monthly",
                )
            },
            // An override, which rides along as a posting tag.
            ScenarioLine {
                role: LineRole::Asset,
                growth: Some(growing(3)),
                opening: Some(usd(50_000_000, 2, 2)),
                note: "projection".to_string(),
                ..line(
                    "rule:0",
                    "rule:0:2",
                    "assets:house",
                    usd(0, 2, 2),
                    "monthly",
                )
            },
            // An ordinary flow beside them, so the two shapes share one block.
            ScenarioLine {
                note: "projection".to_string(),
                ..line(
                    "rule:0",
                    "rule:0:3",
                    "expenses:rent",
                    usd(420_000, 2, 2),
                    "monthly",
                )
            },
        ],
        ..Scenario::default()
    }
}

/// hledger, if the opt-in is set and the binary is there.
fn hledger(args: &[&str]) -> Option<std::process::Output> {
    if std::env::var(OPT_IN).is_err() {
        return None;
    }
    Command::new("hledger").args(args).output().ok()
}

#[test]
fn hledger_reads_a_scenario_we_wrote() {
    let Some(_) = hledger(&["--version"]) else {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    };
    let scratch = Scratch::new("hledger");
    let scenario = cross_check_scenario();
    let path = scratch.path().join("projection-cross-check.journal");
    let text = new_file(&scenario, &path.to_string_lossy(), &declared()).expect("it writes");
    std::fs::write(&path, &text).expect("write");
    let file = path.to_string_lossy().to_string();

    // 1. It is a journal hledger can read at all. `print` over a file whose
    //    only content is `~` rules prints nothing, so the exit status is the
    //    assertion — and the point of running it is that a `~` header, a
    //    `growth:` tag or a virtual posting we wrote wrongly is a PARSE error,
    //    not a silent difference.
    let printed = hledger(&["-f", &file, "print"]).expect("hledger runs");
    assert!(
        printed.status.success(),
        "hledger print refused the file:\n{}\n--- the file ---\n{text}",
        String::from_utf8_lossy(&printed.stderr)
    );

    // 2. `balance --budget -M` reads the rules AS GOALS, and the goals it
    //    reports are the figures we asked for. One month, so the flat lines'
    //    goals are exactly their amounts — this is the check that a bounded
    //    header (`monthly to 2027-04-01`) selects the segment we meant.
    let budget = hledger(&[
        "-f",
        &file,
        "balance",
        "--budget",
        "-M",
        "-b",
        "2027-01-01",
        "-e",
        "2027-02-01",
        "--layout=bare",
    ])
    .expect("hledger runs");
    assert!(
        budget.status.success(),
        "hledger balance --budget refused:\n{}",
        String::from_utf8_lossy(&budget.stderr)
    );
    let report = String::from_utf8_lossy(&budget.stdout);
    for goal in ["4200.00", "12000.00", "50000.00"] {
        assert!(
            report.contains(goal),
            "hledger's own budget goals do not carry {goal}:\n{report}\n--- the file ---\n{text}"
        );
    }
    // The LATER segment of the step does not fire in January 2027 — its rule
    // starts in April — so its figure must be absent.
    assert!(
        !report.contains("120000.00"),
        "the `from 2027-04-01` segment fired in January:\n{report}"
    );

    // 3. `--forecast` turns the same rules into future transactions, which is
    //    the other reading of a `~` block. The dated one-off is the one worth
    //    naming: it is a rule with no interval, and a writer that mis-rendered
    //    its header would simply lose it.
    let forecast = hledger(&[
        "-f",
        &file,
        "register",
        "--forecast=2027-03-01..2027-04-01",
        "assets:cash",
    ])
    .expect("hledger runs");
    assert!(
        forecast.status.success(),
        "hledger --forecast refused:\n{}",
        String::from_utf8_lossy(&forecast.stderr)
    );
    let register = String::from_utf8_lossy(&forecast.stdout);
    assert!(
        register.contains("Series A") && register.contains("2000000.00"),
        "the dated one-off did not forecast:\n{register}\n--- the file ---\n{text}"
    );
}

#[test]
fn hledger_reads_the_growth_tag_as_a_tag_and_not_as_arithmetic() {
    // The honest caveat `docs/projections/file-format.md` states, pinned: hledger has no
    // arithmetic in amounts, so `growth:` is a comment to it and the goal it
    // reports is the BASE amount in every year. A test rather than a sentence,
    // because the day hledger gains arithmetic this should fail and the doc
    // should change.
    let Some(_) = hledger(&["--version"]) else {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    };
    let scratch = Scratch::new("growth");
    let scenario = Scenario {
        name: "Growth".to_string(),
        lines: vec![ScenarioLine {
            note: "plan".to_string(),
            growth: Some(Growth {
                rate: Dec::new(50, 2),
                unit: GrowthUnit::Year,
            }),
            ..line(
                "rule:0",
                "rule:0:0",
                "expenses:rent",
                usd(100_000, 2, 2),
                "monthly",
            )
        }],
        ..Scenario::default()
    };
    let path = scratch.path().join("projection-growth.journal");
    let text = new_file(&scenario, &path.to_string_lossy(), &declared()).expect("it writes");
    std::fs::write(&path, &text).expect("write");
    let file = path.to_string_lossy().to_string();

    let report = hledger(&[
        "-f",
        &file,
        "balance",
        "--budget",
        "-Y",
        "-b",
        "2027-01-01",
        "-e",
        "2029-01-01",
        "--layout=bare",
    ])
    .expect("hledger runs");
    let out = String::from_utf8_lossy(&report.stdout);
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    // 12 × $1000 in BOTH years — never 12 × $1500 in the second.
    assert!(
        out.contains("12000.00") && !out.contains("18000.00"),
        "hledger applied the growth tag after all:\n{out}"
    );
}

/// **The asset-row cross-check** (plan 23, Phase 2). Four claims, none of which
/// a round trip through our own parser can make, because all four are about
/// what somebody ELSE's reader does with the bytes we wrote.
///
/// 1. A `$0` posting carrying a `growth:` tag is a journal hledger parses.
/// 2. `balance --budget` reports the CONTRIBUTION as a goal — and reports NO
///    goal at all for a row whose contribution is `$0`. That is the honest
///    answer rather than a gap: a growth rate is not a goal, and hledger has no
///    way to express one.
/// 3. `print --forecast` round-trips the `$0` posting WITH its tags, including
///    an `opening:` that rides beside a `growth:` on the same comment.
/// 4. `tag:growth` matches the generated postings, so the marker this crate
///    keys the asset/flow distinction off is a marker hledger agrees exists.
#[test]
fn hledger_reads_an_asset_row_as_a_contribution_and_never_as_a_goal() {
    let Some(_) = hledger(&["--version"]) else {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    };
    let scratch = Scratch::new("assets");
    let path = scratch.path().join("projection-assets.journal");
    let text =
        new_file(&asset_scenario(), &path.to_string_lossy(), &declared()).expect("it writes");
    std::fs::write(&path, &text).expect("write");
    let file = path.to_string_lossy().to_string();

    // 1. It parses.
    let printed = hledger(&["-f", &file, "print"]).expect("hledger runs");
    assert!(
        printed.status.success(),
        "hledger print refused the file:\n{}\n--- the file ---\n{text}",
        String::from_utf8_lossy(&printed.stderr)
    );

    // 2. The contribution is the goal, and a `$0` row has none.
    let budget = hledger(&[
        "-f",
        &file,
        "balance",
        "--budget",
        "-M",
        "-b",
        "2027-01-01",
        "-e",
        "2027-02-01",
        "--layout=bare",
    ])
    .expect("hledger runs");
    assert!(
        budget.status.success(),
        "hledger balance --budget refused:\n{}",
        String::from_utf8_lossy(&budget.stderr)
    );
    let report = String::from_utf8_lossy(&budget.stdout);
    assert!(
        report.contains("assets:savings") && report.contains("2000.00"),
        "the contribution is not reported as a goal:\n{report}\n--- the file ---\n{text}"
    );
    assert!(
        report.contains("expenses:rent") && report.contains("4200.00"),
        "the flow beside it lost its goal:\n{report}"
    );
    for silent in ["assets:brokerage", "assets:house"] {
        assert!(
            !report.contains(silent),
            "a `$0` contribution must state NO goal — a growth rate is not one, \
             and hledger cannot express one. {silent} appeared:\n{report}"
        );
    }

    // 3. The `$0` posting and every tag survive a forecast round trip.
    let forecast = hledger(&["-f", &file, "print", "--forecast=2027-01-01..2027-02-01"])
        .expect("hledger runs");
    assert!(
        forecast.status.success(),
        "hledger --forecast refused:\n{}",
        String::from_utf8_lossy(&forecast.stderr)
    );
    let generated = String::from_utf8_lossy(&forecast.stdout);
    for expected in [
        "(assets:brokerage)",
        "growth: 7%/yr",
        "growth: 3%/yr, opening: 500000.00",
        "$0.00",
    ] {
        assert!(
            generated.contains(expected),
            "`{expected}` did not survive `print --forecast`:\n{generated}\n--- the file ---\n{text}"
        );
    }

    // 4. `tag:growth` is a query hledger can answer over them.
    let tagged = hledger(&[
        "-f",
        &file,
        "register",
        "--forecast=2027-01-01..2027-02-01",
        "tag:growth",
    ])
    .expect("hledger runs");
    let matched = String::from_utf8_lossy(&tagged.stdout);
    assert!(
        tagged.status.success(),
        "{}",
        String::from_utf8_lossy(&tagged.stderr)
    );
    for account in ["assets:brokerage", "assets:savings", "assets:house"] {
        assert!(
            matched.contains(account),
            "`tag:growth` did not match {account}:\n{matched}"
        );
    }
}

/// The committed asset fixture reads as the asset rows it states, and rewriting
/// it changes not one byte.
///
/// `fixtures/asset-growth-scenario.journal` is what
/// `fixtures/native/v1/projections-asset.json` is a response to, so it is the
/// one scenario file in the repo whose exact bytes a golden depends on. A
/// re-render that silently reformatted it would move the golden with it and
/// nothing would say so.
#[test]
fn the_committed_asset_fixture_reads_and_rewrites_unchanged() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/asset-growth-scenario.journal")
        .canonicalize()
        .expect("the fixture resolves");
    let name = path.to_string_lossy().to_string();
    let text = std::fs::read_to_string(&path).expect("the fixture reads");

    let scenario = scenario_from_text(&text, &name, "fallback", &declared()).expect("it parses");
    assert_eq!(scenario.name, "Asset growth");
    let roles: Vec<(&str, LineRole)> = scenario
        .lines
        .iter()
        .map(|line| (line.account.0.as_str(), line.role))
        .collect();
    assert_eq!(
        roles,
        [
            ("assets:broker:taxable:vti", LineRole::Asset),
            ("assets:bank:savings", LineRole::Asset),
            ("assets:property:home", LineRole::Asset),
            // `growth:` on a non-asset account is a growing FLOW.
            ("expenses:housing:rent", LineRole::Flow),
            // …and no `growth:` at all is a flow whatever the account, which is
            // what keeps every pre-existing scenario file meaning what it did.
            ("assets:bank:savings", LineRole::Flow),
            ("assets:bank:checking", LineRole::Flow),
        ]
    );

    let doc = ProjectionDoc::parse(&text, &name, &declared()).expect("it parses");
    let rewritten = write_scenario(&doc, &scenario, &name).expect("it writes");
    // The fixture predates plan 28, and its growing asset rows book
    // appreciation to `equity:unrealized gains`, which neither the main journal
    // nor the file declares. So the FIRST save declares it in the file
    // (Decision 4), under the header.
    //
    // And it drops the one `line:` tag the fixture carries: `line: savings` is
    // on a row with a single segment, and `line:` is written only for a step
    // change now (plan 28, follow-ups 2026-10-26). That block is re-rendered
    // without it — in the writer's own alignment — and nothing else changes.
    assert_eq!(
        rewritten,
        text.replacen(
            "; updated: 2026-09-20\n\n",
            "; updated: 2026-09-20\n\naccount equity:unrealized gains  ; type: E\n\n",
            1,
        )
        .replacen(
            "    (assets:broker:taxable:vti)     $0.00  ; growth: 7%/yr\n\
             \x20   (assets:bank:savings)         $250.00  ; growth: 4%/yr, line: savings\n\
             \x20   (assets:property:home)          $0.00  ; growth: 3%/yr, opening: 640000.00\n\
             \x20   (expenses:housing:rent)      $1875.00  ; growth: 2%/yr\n",
            "    (assets:broker:taxable:vti)  $0.00  ; growth: 7%/yr\n\
             \x20   (assets:bank:savings)        $250.00  ; growth: 4%/yr\n\
             \x20   (assets:property:home)       $0.00  ; growth: 3%/yr, opening: 640000.00\n\
             \x20   (expenses:housing:rent)      $1875.00  ; growth: 2%/yr\n",
            1,
        ),
        "a save that changes nothing must change no byte but the gains declaration and the \
         unneeded `line:` tag"
    );
    assert!(!rewritten.contains("line:"), "{rewritten}");
    let doc = ProjectionDoc::parse(&rewritten, &name, &declared()).expect("it parses");
    assert_eq!(
        write_scenario(&doc, &scenario, &name).expect("it writes"),
        rewritten,
        "a save of the upgraded file must change no byte at all"
    );
}

/// Plan 28: a scenario with counter-accounts is ordinary BALANCED hledger.
/// The complaint the plan opens with — `hledger bs --forecast` showed salary in
/// the P&L and never in any bank account — is what this pins: forecast over the
/// written file, checking moves by the net of the rule, and the file's own
/// gains declaration passes `check`.
#[test]
fn hledger_forecasts_a_balanced_scenario_into_the_counter_account() {
    let Some(_) = hledger(&["--version"]) else {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    };
    let scratch = Scratch::new("hledger-balanced");
    let counter = Some(AccountName("assets:checking".to_string()));
    let scenario = Scenario {
        name: "Balanced".to_string(),
        cash: counter.clone(),
        lines: vec![
            ScenarioLine {
                counter: counter.clone(),
                ..line(
                    "rule:0",
                    "rule:0:0",
                    "revenues:salary",
                    usd(-1_200_000, 2, 2),
                    "monthly",
                )
            },
            ScenarioLine {
                counter: counter.clone(),
                ..line(
                    "rule:0",
                    "rule:0:1",
                    "expenses:rent",
                    usd(420_000, 2, 2),
                    "monthly",
                )
            },
            ScenarioLine {
                role: LineRole::Asset,
                growth: Some(Growth {
                    rate: Dec::new(4, 2),
                    unit: GrowthUnit::Year,
                }),
                ..line(
                    "rule:1",
                    "rule:1:0",
                    "assets:house",
                    usd(0, 2, 2),
                    "monthly",
                )
            },
        ],
        ..Scenario::default()
    };
    let path = scratch.path().join("projection-balanced.journal");
    let text = new_file(&scenario, &path.to_string_lossy(), &declared()).expect("it writes");
    assert!(
        text.contains("account equity:unrealized gains  ; type: E"),
        "{text}"
    );
    std::fs::write(&path, &text).expect("write");
    let file = path.to_string_lossy().to_string();

    let checked = hledger(&["-f", &file, "check"]).expect("hledger runs");
    assert!(
        checked.status.success(),
        "hledger check refused the file:\n{}\n--- the file ---\n{text}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let balance = hledger(&[
        "-f",
        &file,
        "balance",
        "--forecast=2027-01-01..2027-03-01",
        "assets:checking",
        "--layout=bare",
        "-N",
    ])
    .expect("hledger runs");
    assert!(
        balance.status.success(),
        "{}",
        String::from_utf8_lossy(&balance.stderr)
    );
    let report = String::from_utf8_lossy(&balance.stdout);
    // Two months of $12,000 in and $4,200 out.
    assert!(
        report.contains("15600.00"),
        "the forecast did not land in checking:\n{report}\n--- the file ---\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Plan 28, Phase 6: per-account, per-period parity with hledger
// ---------------------------------------------------------------------------
//
// For every scenario under `fixtures/projections-parity/` WITHOUT growth, the
// three statements Ledgeline computes over `merged_transactions` (the same
// core calls `projections_api::projected_statements` makes) must equal what
// the documented hledger command prints over `main.journal` plus the file our
// serializer wrote, account by account and column by column. With growth, the
// two accepted deviations (Decisions 2 and 3) are asserted as exactly what
// they are and nothing more.

/// "Today" for every parity run. The main journal's latest transaction is
/// 2026-10-05, so the monthly and quarterly cutoff is 2026-10-01 and the
/// October transaction is real data AFTER it that both tools must leave out.
const PARITY_TODAY: &str = "2026-10-20";

/// One forecast shape: the interval, hledger's flag for it, and the columns.
#[derive(Clone, Copy, Debug)]
struct Cadence {
    interval: Interval,
    flag: &'static str,
    count: usize,
    lookback: usize,
}

const MONTHLY: Cadence = Cadence {
    interval: Interval::Monthly,
    flag: "-M",
    count: 9,
    lookback: 2,
};
const QUARTERLY: Cadence = Cadence {
    interval: Interval::Quarterly,
    flag: "-Q",
    count: 3,
    lookback: 2,
};
const WEEKLY: Cadence = Cadence {
    interval: Interval::Weekly,
    flag: "-W",
    count: 8,
    lookback: 2,
};

/// `(account, commodity)` → one figure per column.
type Table = BTreeMap<(String, String), Vec<Dec>>;

/// One scenario fixture, saved through the serializer exactly as the server's
/// save handler saves it, then expanded exactly as its run handler expands it.
struct Parity {
    /// Held for its `Drop`: the written scenario file lives in it.
    _scratch: Scratch,
    file: &'static str,
    main: String,
    scenario: String,
    text: String,
    journal: Journal,
    declared: BTreeMap<String, AccountType>,
    forecast: Forecast,
    txns: Vec<Transaction>,
    prices: Vec<PriceDirective>,
    cadence: Cadence,
}

fn parity_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/projections-parity")
        .canonicalize()
        .expect("the parity fixtures resolve")
}

impl Parity {
    fn new(file: &'static str, cadence: Cadence) -> Self {
        let scratch = Scratch::new("parity");
        let main_path = parity_dir().join("main.journal");
        let main = main_path.to_string_lossy().to_string();
        let journal = parse_journal(
            &std::fs::read_to_string(&main_path).expect("main reads"),
            &main,
        )
        .expect("main parses");
        let declared = declared_types(&account_decls(&journal));
        let types = AccountTypes::from_declared(declared.clone());
        let history = History {
            txns: &journal.transactions,
            prices: &journal.prices,
        };

        // The SAVE: `projections_api::save` upgrades legacy rules with defaults
        // at a monthly cutoff, then writes through the byte-preserving writer.
        let source = std::fs::read_to_string(parity_dir().join(file)).expect("scenario reads");
        let path = scratch.path().join(format!("projection-{file}"));
        let scenario = path.to_string_lossy().to_string();
        let doc = ProjectionDoc::parse(&source, &scenario, &declared).expect("it parses");
        let read = scenario_from_text(&source, &scenario, "parity", &declared).expect("it reads");
        let cutoff = forecast_cutoff(
            PARITY_TODAY,
            latest_real_date(&journal.transactions),
            Interval::Monthly,
        )
        .expect("cutoff");
        let defaults = CounterDefaults::new(&history, &types, &cutoff, read.cash.as_ref(), None)
            .expect("defaults");
        let upgraded = upgrade_legacy_counters(&read, &defaults);
        let text = write_scenario(&doc, &upgraded, &scenario).expect("it writes");
        std::fs::write(&path, &text).expect("write");

        // The RUN, over what the next load of that file reads.
        let reread = scenario_from_text(&text, &scenario, "parity", &declared).expect("re-reads");
        let forecast = forecast_transactions(
            &reread,
            &ForecastOpts {
                today: PARITY_TODAY,
                interval: cadence.interval,
                count: cadence.count,
                lookback: cadence.lookback,
                value_in: None,
            },
            &types,
            &history,
        )
        .expect("it expands");
        let txns = merged_transactions(&journal.transactions, &forecast);
        let prices = merged_prices(&journal.prices, &forecast);
        Self {
            _scratch: scratch,
            file,
            main,
            scenario,
            text,
            journal,
            declared,
            forecast,
            txns,
            prices,
            cadence,
        }
    }

    fn window(&self) -> &ForecastWindow {
        &self.forecast.window
    }

    /// hledger's `-e` and the `--forecast=` end are exclusive; ours is the
    /// last day.
    fn end_exclusive(&self) -> String {
        add_days(&self.window().end, 1)
    }

    fn spans(&self) -> Vec<BucketSpan> {
        range_buckets(
            &self.window().from,
            &self.window().end,
            Some(self.cadence.interval),
        )
        .expect("spans")
    }

    /// The command `docs/projections/cash.md` documents, verbatim in shape:
    /// `hledger -f <main> -f <scenario> <cmd> --forecast=<cutoff>..<end>
    /// -b <lookback start> -e <end> -<interval> expr:"NOT (date:<cutoff>.. AND
    /// NOT tag:generated)"`.
    fn documented(&self, command: &str) -> Vec<String> {
        let cutoff = &self.window().cutoff;
        let end = self.end_exclusive();
        [
            "-f",
            &self.main,
            "-f",
            &self.scenario,
            command,
            &format!("--forecast={cutoff}..{end}"),
            "-b",
            &self.window().from,
            "-e",
            &end,
            self.cadence.flag,
            &format!("expr:NOT (date:{cutoff}.. AND NOT tag:generated)"),
        ]
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    fn run(&self, args: &[String]) -> String {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = hledger(&argv).expect("hledger runs");
        assert!(
            out.status.success(),
            "{}: hledger {argv:?} refused:\n{}\n--- the scenario file ---\n{}",
            self.file,
            String::from_utf8_lossy(&out.stderr),
            self.text
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// The documented command as CSV, one row per account and commodity.
    /// `value` adds `--value=end,$`: market value at each column's end, which
    /// is what Ledgeline's `Valuation::Market` statements are.
    fn hledger_table(&self, command: &str, value: bool) -> Table {
        let mut args = self.documented(command);
        args.extend(["-O", "csv", "--layout=bare"].map(String::from));
        if value {
            args.push("--value=end,$".to_string());
        }
        let csv = self.run(&args);
        let columns = self.spans().len();
        let mut table = Table::new();
        // Line 1 is the title, line 2 the header.
        for fields in csv.lines().skip(2).map(csv_fields) {
            let [account, commodity, values @ ..] = fields.as_slice() else {
                continue;
            };
            if account.ends_with(':') && (account == "Total:" || account == "Net:") {
                continue;
            }
            if values.iter().all(String::is_empty) {
                continue; // a section heading
            }
            assert_eq!(values.len(), columns, "{}: {csv}", self.file);
            let figures: Vec<Dec> = values
                .iter()
                .map(|v| Dec::parse(v, '.').unwrap_or_else(|e| panic!("{v}: {e:?}")))
                .collect();
            if commodity.is_empty() && figures.iter().all(Dec::is_zero) {
                continue;
            }
            table.insert((account.clone(), commodity.clone()), figures);
        }
        table
    }

    /// Ledgeline's balance sheet rows, split into (assets + liabilities,
    /// equity): `balance_sheet_columns` at each span's end, market-valued into
    /// the forecast's target, exactly as `projected_statements` calls it.
    /// Computed groups (retained earnings, the valuation adjustment) have no
    /// rows and hledger has no such accounts, so only rows are compared.
    fn ledgeline_bs(&self) -> (Table, Table) {
        let ends: Vec<String> = self.spans().into_iter().map(|span| span.to).collect();
        let sheets = balance_sheet_columns(
            &self.txns,
            &self.prices,
            &BsOpts {
                as_of: &self.window().end,
                depth: None,
                value: Valuation::Market,
                value_in: self.forecast.target.clone(),
            },
            &ends,
            &self.declared,
            &account_groups(&self.journal),
            &bs_terms(&self.journal),
        )
        .expect("balance sheet columns");
        let (mut sheet, mut equity) = (Table::new(), Table::new());
        for (column, report) in sheets.iter().enumerate() {
            assert!(report.balanced, "{}: column {column} unbalanced", self.file);
            for section in &report.sections {
                let table = if section.kind == BsSectionKind::Equity {
                    &mut equity
                } else {
                    &mut sheet
                };
                for row in section.groups.iter().flat_map(|group| &group.rows) {
                    put(table, &row.account, &row.own, column, ends.len());
                }
            }
        }
        (sheet, equity)
    }

    /// Ledgeline's income statement rows, one figure per column.
    fn ledgeline_is(&self) -> Table {
        let report = income_statement_columns(
            &self.txns,
            &self.prices,
            &IsOpts {
                from: &self.window().from,
                to: &self.window().end,
                value: Valuation::Market,
                value_in: self.forecast.target.clone(),
                compare: false,
            },
            self.cadence.interval,
            &self.declared,
            &account_sections(&self.journal),
            &declared_groups(&self.journal, IS_GROUP_TAG),
        )
        .expect("income statement");
        let columns = report.buckets.len();
        let mut table = Table::new();
        for row in report
            .sections
            .iter()
            .flat_map(|section| &section.groups)
            .flat_map(|group| &group.rows)
        {
            for (column, figure) in row.amounts.values.iter().enumerate() {
                put(&mut table, &row.account, figure, column, columns);
            }
        }
        table
    }

    /// Ledgeline's cash flow statement, reduced to what hledger's `cf` is:
    /// the change in each cash account per period (ending − beginning).
    /// hledger's `cf` is a direct list of cash-account changes, not our
    /// indirect statement (net income, adjustments, investing, financing), so
    /// the per-cash-account change is the comparable part. The rest of our
    /// statement reconciles to exactly this (`reconciles`).
    fn ledgeline_cf(&self) -> Table {
        let statement = cash_flow_statement(
            &self.txns,
            &CfOpts {
                from: &self.window().from,
                to: &self.window().end,
                interval: Some(self.cadence.interval),
                depth: 99,
            },
            &CfTags::from_journal(&self.journal),
        )
        .expect("cash flow statement");
        assert!(statement.reconciles, "{}: cf does not reconcile", self.file);
        let columns = statement.buckets.len();
        let mut table = Table::new();
        for row in statement.cash.iter().flat_map(|line| &line.rows) {
            for column in 0..columns {
                let change = row.ending.values[column]
                    .ma_add(&row.beginning.values[column].ma_neg().expect("neg"))
                    .expect("change");
                put(&mut table, &row.account, &change, column, columns);
            }
        }
        table
    }

    /// Every generated posting, as hledger generates it from the file we wrote.
    fn hledger_generated(&self) -> Vec<String> {
        let cutoff = &self.window().cutoff;
        let end = self.end_exclusive();
        let csv = self.run(
            &[
                "-f",
                &self.main,
                "-f",
                &self.scenario,
                "print",
                &format!("--forecast={cutoff}..{end}"),
                "tag:generated",
                "-O",
                "csv",
            ]
            .map(String::from),
        );
        let mut lines = csv.lines().map(csv_fields);
        let header = lines.next().expect("a header row");
        let at = |name: &str| {
            header
                .iter()
                .position(|field| field == name)
                .unwrap_or_else(|| panic!("no `{name}` column in {header:?}"))
        };
        let (date, account, amount, commodity) =
            (at("date"), at("account"), at("amount"), at("commodity"));
        let mut postings: Vec<String> = lines
            .filter_map(|fields| {
                let quantity = Dec::parse(&fields[amount], '.').expect("amount");
                // A legacy `(account)` line stays virtual in hledger, which still
                // counts it in every report run without `-R`. The expansion books
                // it as a REGULAR posting, because the cash flow statement drops
                // virtual ones. Same account, same amount, same reports: the marker
                // is compared away.
                let name = fields[account].trim_start_matches(['(', '[']);
                let name = name.trim_end_matches([')', ']']);
                (!quantity.is_zero())
                    .then(|| posting_key(&fields[date], name, &fields[commodity], quantity))
            })
            .collect();
        postings.sort();
        postings
    }

    /// Every generated posting, as Ledgeline expands the same file.
    fn ledgeline_generated(&self) -> Vec<String> {
        let mut postings: Vec<String> = self
            .forecast
            .txns
            .iter()
            .flat_map(|txn| {
                txn.postings.iter().flat_map(move |posting| {
                    posting
                        .amounts
                        .iter()
                        .filter(|amount| !amount.quantity.is_zero())
                        .map(move |amount| {
                            posting_key(
                                &txn.date,
                                &posting.account.0,
                                &amount.commodity.0,
                                amount.quantity,
                            )
                        })
                })
            })
            .collect();
        postings.sort();
        postings
    }

    /// `hledger check` with every check this fixture can satisfy: the default
    /// ones (parseable, balanced, assertions), `--strict` (every account and
    /// commodity declared) and `ordereddates`.
    fn assert_hledger_checks(&self) {
        self.run(
            &[
                "-f",
                &self.main,
                "-f",
                &self.scenario,
                "check",
                "--strict",
                "ordereddates",
            ]
            .map(String::from),
        );
    }

    fn assert_agree(&self, what: &str, hledger: &Table, ours: &Table, skip: &dyn Fn(&str) -> bool) {
        assert!(
            !hledger.is_empty() && !ours.is_empty(),
            "{}: {what} compared nothing (hledger {hledger:?}, ours {ours:?})",
            self.file
        );
        let columns: Vec<String> = self.spans().into_iter().map(|span| span.key).collect();
        let zeros = vec![Dec::zero(); columns.len()];
        let keys: BTreeSet<&(String, String)> = hledger.keys().chain(ours.keys()).collect();
        let mismatches: Vec<String> = keys
            .into_iter()
            .filter(|(account, _)| !skip(account))
            .flat_map(|key| {
                let theirs = hledger.get(key).unwrap_or(&zeros);
                let mine = ours.get(key).unwrap_or(&zeros);
                columns
                    .iter()
                    .enumerate()
                    .filter(|&(column, _)| cents(theirs[column]) != cents(mine[column]))
                    .map(|(column, label)| {
                        format!(
                            "{} {} @ {label}: hledger {} ledgeline {} (cents)",
                            key.0,
                            key.1,
                            cents(theirs[column]),
                            cents(mine[column])
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "{} ({:?}) {what} disagrees with hledger:\n{}\n--- the scenario file ---\n{}",
            self.file,
            self.cadence,
            mismatches.join("\n"),
            self.text
        );
    }

    /// The full parity claim for a scenario without growth.
    fn assert_parity(&self) {
        let none = |_: &str| false;
        self.assert_hledger_checks();
        assert_eq!(
            self.hledger_generated(),
            self.ledgeline_generated(),
            "{} ({:?}): the generated postings differ (date account commodity amount)\n--- \
             the scenario file ---\n{}",
            self.file,
            self.cadence,
            self.text
        );
        let (sheet, equity) = self.ledgeline_bs();
        self.assert_agree("bs", &self.hledger_table("bs", true), &sheet, &none);
        let hledger_equity: Table = self
            .hledger_table("bse", true)
            .into_iter()
            .filter(|((account, _), _)| account.starts_with("equity"))
            .collect();
        self.assert_agree("bse equity", &hledger_equity, &equity, &none);
        self.assert_agree(
            "is",
            &self.hledger_table("is", true),
            &self.ledgeline_is(),
            &none,
        );
        self.assert_agree(
            "cf",
            &self.hledger_table("cf", false),
            &self.ledgeline_cf(),
            &none,
        );
    }
}

/// Whole cents, the precision both tools display every fixture commodity at
/// or above.
fn cents(dec: Dec) -> i128 {
    dec.rounded(2).expect("cents fit").mantissa
}

fn posting_key(date: &str, account: &str, commodity: &str, quantity: Dec) -> String {
    format!("{date} {account} {commodity} {}", cents(quantity))
}

/// Add `amount` into `table` at `column`, one row per commodity.
fn put(table: &mut Table, account: &str, amount: &MixedAmount, column: usize, columns: usize) {
    for (commodity, quantity) in amount.iter() {
        let row = table
            .entry((account.to_string(), commodity.0.clone()))
            .or_insert_with(|| vec![Dec::zero(); columns]);
        row[column] = row[column].add(*quantity).expect("no overflow");
    }
}

/// One line of hledger's CSV: every field is double-quoted and none of the
/// fixture's values contains a quote or a comma.
fn csv_fields(line: &str) -> Vec<String> {
    let line = line.trim();
    let inner = line.strip_prefix('"').unwrap_or(line);
    inner
        .strip_suffix('"')
        .unwrap_or(inner)
        .split("\",\"")
        .map(ToString::to_string)
        .collect()
}

/// Run the parity claim for each fixture × cadence, skipping without the
/// opt-in.
fn parity(files: &[&'static str], cadences: &[Cadence]) {
    if hledger(&["--version"]).is_none() {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    }
    for file in files {
        for cadence in cadences {
            Parity::new(file, *cadence).assert_parity();
        }
    }
}

#[test]
fn parity_flows_with_explicit_counters() {
    parity(&["flows.journal"], &[MONTHLY, QUARTERLY, WEEKLY]);
}

#[test]
fn parity_accrual_settlement_and_equity_raise() {
    parity(&["events.journal"], &[MONTHLY, QUARTERLY]);
}

#[test]
fn parity_legacy_rules_once_upgraded_on_save() {
    if hledger(&["--version"]).is_none() {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    }
    let run = Parity::new("legacy.journal", MONTHLY);
    // The upgrade did happen: the unbalanced rules gained an amountless
    // counter posting, the balanced ones did not.
    assert!(
        run.text.contains("    assets:checking\n"),
        "the legacy rule was not upgraded:\n{}",
        run.text
    );
    assert!(
        run.text.contains("    (assets:checking)     $-300.00\n"),
        "{}",
        run.text
    );
    run.assert_parity();
    parity(&["legacy.journal"], &[QUARTERLY]);
}

#[test]
fn parity_asset_row_without_growth_and_its_gains_account() {
    parity(&["assets.journal"], &[MONTHLY]);
}

#[test]
fn parity_step_change_with_from_and_to_bounds() {
    parity(&["step.journal"], &[MONTHLY, QUARTERLY]);
}

#[test]
fn parity_multi_commodity_lines() {
    parity(&["commodities.journal"], &[MONTHLY]);
}

/// The fixture really does hold real data AFTER the cutoff, and it is the
/// documented `expr:` that leaves it out: without it, hledger's October food
/// carries the real 2026-10-05 $77 on top of the forecast. So the parity above
/// is a test of the documented command, not of a journal that happens to end
/// before the cutoff.
#[test]
fn parity_needs_the_documented_expr_to_leave_out_real_data_after_the_cutoff() {
    if hledger(&["--version"]).is_none() {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    }
    let run = Parity::new("flows.journal", MONTHLY);
    let october = run.cadence.lookback;
    let key = ("expenses:food".to_string(), "$".to_string());
    let filtered = run.hledger_table("is", true)[&key][october];
    let mut args = run.documented("is");
    args.pop(); // the expr
    args.extend(["-O", "csv", "--layout=bare"].map(String::from));
    let unfiltered = run
        .run(&args)
        .lines()
        .map(csv_fields)
        .find(|fields| fields.first() == Some(&key.0))
        .map(|fields| Dec::parse(&fields[2 + october], '.').expect("amount"))
        .expect("an expenses:food row");
    assert_eq!(cents(unfiltered) - cents(filtered), 7_700);
    assert_eq!(cents(run.ledgeline_is()[&key][october]), cents(filtered));
}

/// WITH growth, hledger and Ledgeline differ by exactly the two accepted
/// deviations (plan 28 Decisions 2 and 3) and by nothing else:
///
/// 1. `growth:` on a flow: hledger books the BASE amount in every period; we
///    book the grown one. The difference reaches the counter-account (cash)
///    and nowhere else.
/// 2. `growth:` on an unpriced asset row: we post appreciation from the asset
///    to `equity:unrealized gains`; hledger posts nothing, so the house is
///    flat and the gains account is absent.
/// 3. `growth:` on a priced holding: we grow AAPL's price in memory; hledger
///    values the holding at its last real `P`, flat.
///
/// Every other account agrees per column. The day hledger gains arithmetic or
/// we stop deviating, this fails and `docs/projections/` must change.
#[test]
fn parity_with_growth_differs_only_by_the_accepted_deviations() {
    if hledger(&["--version"]).is_none() {
        eprintln!("skipped: set {OPT_IN}=1 (see `just hledger-checks`)");
        return;
    }
    let run = Parity::new("growth.journal", MONTHLY);
    // Decision 4: the gains account the main journal lacks is declared in the
    // scenario, so `check --strict` passes over both files.
    assert!(
        run.text
            .contains("account equity:unrealized gains  ; type: E"),
        "{}",
        run.text
    );
    run.assert_hledger_checks();

    let columns = run.spans().len();
    let forecast = run.cadence.lookback..columns;
    let usd = |table: &Table, account: &str| -> Vec<i128> {
        table
            .get(&(account.to_string(), "$".to_string()))
            .map_or_else(
                || vec![0; columns],
                |row| row.iter().map(|d| cents(*d)).collect(),
            )
    };
    let grown = [
        "expenses:rent",
        "assets:checking",
        "assets:house",
        "assets:brokerage",
    ];
    let gains = "equity:unrealized gains";

    // 1. The flow: hledger's rent is the base in every column; ours grows
    //    from the second forecast month on (2%/mo, anchored at the cutoff).
    let (their_is, our_is) = (run.hledger_table("is", true), run.ledgeline_is());
    let (their_rent, our_rent) = (
        usd(&their_is, "expenses:rent"),
        usd(&our_is, "expenses:rent"),
    );
    assert!(their_rent.iter().all(|&c| c == 200_000), "{their_rent:?}");
    assert_eq!(our_rent[forecast.start], 200_000, "{our_rent:?}");
    assert!(
        our_rent[forecast.start + 1..]
            .windows(2)
            .all(|pair| pair[0] > 200_000 && pair[1] > pair[0]),
        "{our_rent:?}"
    );
    run.assert_agree("is (outside growth)", &their_is, &our_is, &|a| {
        grown.contains(&a)
    });

    // …and the extra rent is exactly what cash is short by, per period.
    let extra: Vec<i128> = our_rent
        .iter()
        .zip(&their_rent)
        .map(|(o, t)| o - t)
        .collect();
    let (their_cf, our_cf) = (run.hledger_table("cf", false), run.ledgeline_cf());
    let (their_cash, our_cash) = (
        usd(&their_cf, "assets:checking"),
        usd(&our_cf, "assets:checking"),
    );
    for column in 0..columns {
        assert_eq!(
            their_cash[column] - our_cash[column],
            extra[column],
            "column {column}"
        );
    }
    run.assert_agree("cf (outside growth)", &their_cf, &our_cf, &|a| {
        grown.contains(&a)
    });

    // 2 and 3. The balance sheet: hledger's house and brokerage are flat; ours
    // rise every forecast month. Checking differs by the cumulative extra rent.
    let (their_bs, (our_bs, our_equity)) = (run.hledger_table("bs", true), run.ledgeline_bs());
    for account in ["assets:house", "assets:brokerage"] {
        let (theirs, ours) = (usd(&their_bs, account), usd(&our_bs, account));
        assert!(
            theirs.windows(2).all(|pair| pair[0] == pair[1]),
            "{account} {theirs:?}"
        );
        assert!(
            ours[forecast.start..]
                .windows(2)
                .all(|pair| pair[1] > pair[0]),
            "{account} {ours:?}"
        );
        assert_eq!(
            ours[..forecast.start],
            theirs[..forecast.start],
            "{account}: actuals"
        );
    }
    let (their_checking, our_checking) = (
        usd(&their_bs, "assets:checking"),
        usd(&our_bs, "assets:checking"),
    );
    let mut shortfall = 0;
    for column in 0..columns {
        shortfall += extra[column];
        assert_eq!(
            their_checking[column] - our_checking[column],
            shortfall,
            "column {column}"
        );
    }
    run.assert_agree("bs (outside growth)", &their_bs, &our_bs, &|a| {
        grown.contains(&a)
    });

    // The house's appreciation sits in the gains account, which hledger never
    // sees; every other equity account agrees.
    let their_equity: Table = run
        .hledger_table("bse", true)
        .into_iter()
        .filter(|((account, _), _)| account.starts_with("equity"))
        .collect();
    assert!(!their_equity.keys().any(|(account, _)| account == gains));
    let house = usd(&our_bs, "assets:house");
    let booked = usd(&our_equity, gains);
    for column in 0..columns {
        assert_eq!(booked[column], house[column] - house[0], "column {column}");
    }
    run.assert_agree(
        "bse equity (outside gains)",
        &their_equity,
        &our_equity,
        &|a| a == gains,
    );
}
