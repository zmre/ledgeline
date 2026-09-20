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
use ledgeline_core::model::{AccountName, Amount, AmountStyle, Commodity, CommoditySide};
use ledgeline_core::parse::parse_period_spec;
use ledgeline_core::projections::serialize::{
    ProjectionDoc, new_file, scenario_from_text, write_scenario,
};
use ledgeline_core::projections::{
    CreateRefusal, Growth, GrowthUnit, LineSource, Scenario, ScenarioEvent, ScenarioLine, discover,
    slug_filename, virtual_posting,
};
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
        id: id.to_string(),
        group: group.to_string(),
        account: AccountName(account.to_string()),
        amount,
        period: parse_period_spec(period),
        growth: None,
        note: String::new(),
        source: LineSource::Journal,
    }
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

    let doc = ProjectionDoc::parse(before, &name).expect("it parses");
    let mut scenario = scenario_from_text(before, &name, "plan").expect("it reads");
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
    let reread = scenario_from_text(&std::fs::read_to_string(&path).unwrap(), &name, "plan")
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

    let doc = ProjectionDoc::parse(&before, &name).expect("it parses");
    let scenario = scenario_from_text(&before, &name, "plan").expect("it reads");
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
    let text = new_file(&scenario, &path.to_string_lossy()).expect("it writes");
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
    // The honest caveat `docs/projections.md` states, pinned: hledger has no
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
    let text = new_file(&scenario, &path.to_string_lossy()).expect("it writes");
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
