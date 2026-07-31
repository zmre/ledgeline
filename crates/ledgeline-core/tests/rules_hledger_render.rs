//! Opt-in check that the rules **renderer** emits syntax real hledger accepts.
//!
//! Everything else in the rules suite proves we do not damage what we did not
//! touch: round trips, reorders, deletes, and the edit-isolation tests all
//! compare our bytes against our bytes. None of them can tell you whether a line
//! this module *wrote* — a spliced value, an appended matcher, a freshly
//! rendered conditional block — is something hledger will read. That question
//! has exactly one authority, and it is the hledger binary.
//!
//! So: for every fixture under `fixtures/rules/simple/` and `advanced/`, apply a
//! non-trivial edit, write the result to a scratch directory alongside a copy of
//! the fixture's CSV, and run `hledger -f <scratch>/<name>.csv print`. Exit 0 or
//! the test fails with hledger's own complaint.
//!
//! Driving from the CSV rather than `-f FILE.rules` is deliberate and matches
//! `scripts/check-rules-fixtures.sh`: since hledger 1.50, `-f FILE.rules` demands
//! an explicit `source` rule, whereas a CSV picks up its sibling
//! `FILE.csv.rules` the way a real import does.
//!
//! # Why this is default-skipped
//!
//! `cargo test` must stay hermetic and CI-portable, and this shells out to a
//! binary that may not be installed. Set `LEDGELINE_HLEDGER_RENDER_CHECK=1` to
//! run it.
//!
//! # Safety
//!
//! This runs hledger **only** against the committed fixtures, never a user's
//! file. A rules file's `source` directive accepts a `| CMD` form that hledger
//! executes through the shell, so pointing this at arbitrary input would be
//! handing that input a shell. The crate's edit policy refuses to *write* such a
//! rule; this test refuses to *read* one.

mod common;

use ledgeline_core::rules::{
    EditPlan, HledgerField, Item, ItemBody, ItemKind, MatchScope, MatcherSpec, NumberedField,
    RulesDoc, Slot,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable that opts in.
const OPT_IN: &str = "LEDGELINE_HLEDGER_RENDER_CHECK";

/// A matcher no fixture's CSV contains, so adding one changes the file's syntax
/// without changing what it imports.
const INERT_MATCHER: &str = "LEDGELINE RENDER CHECK";

/// A scratch directory that removes itself on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ledgeline-rules-render-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every data-backed fixture, as `(stem, rules path, csv path)`.
///
/// Only `simple/` and `advanced/` qualify: the `edge/` fixtures are named
/// `*.rules` rather than `*.csv.rules` and have no CSV of their own, and two of
/// them are deliberately files hledger rejects.
fn data_backed_fixtures() -> Vec<(String, PathBuf, PathBuf)> {
    let root = common::fixtures_dir().join("rules");
    let mut found: Vec<(String, PathBuf, PathBuf)> = ["simple", "advanced"]
        .iter()
        .flat_map(|group| {
            let dir = root.join(group);
            std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("{} readable: {e}", dir.display()))
                .map(|entry| entry.expect("readable directory entry").path())
                .collect::<Vec<_>>()
        })
        .filter_map(|rules| {
            let name = rules.file_name()?.to_str()?.to_string();
            let stem = name.strip_suffix(".csv.rules")?.to_string();
            let csv = rules.with_file_name(format!("{stem}.csv"));
            csv.is_file().then_some((stem, rules, csv))
        })
        .collect();
    found.sort();
    assert!(
        found.len() >= 3,
        "expected the committed data-backed rules fixtures"
    );
    found
}

/// A conditional block's current matchers and assignments, as an [`ItemBody`] to
/// be edited.
fn block_body(doc: &RulesDoc, item: &Item) -> Option<ItemBody> {
    let block = item.if_block()?;
    Some(ItemBody::IfBlock {
        matchers: block
            .matchers
            .iter()
            .map(|matcher| MatcherSpec {
                scope: matcher.scope.clone(),
                pattern: matcher.pattern.clone(),
            })
            .collect(),
        assignments: block
            .assignments
            .iter()
            .map(|assignment| {
                (
                    assignment.field,
                    doc.text()[assignment.value_span.clone()].to_string(),
                )
            })
            .collect(),
    })
}

/// Rewrite every `accountN` value, deepening it by one subaccount.
///
/// Account names are free-form, so this changes bytes on many lines without
/// changing whether hledger can read the file — which is exactly what is being
/// measured.
fn deepen_accounts(doc: &RulesDoc) -> Option<EditPlan> {
    let mut plan = EditPlan::keep_all(doc);
    let mut touched = false;
    for item in doc.items() {
        let at = item.id.0 as usize;
        match &item.kind {
            ItemKind::Assignment(assignment)
                if matches!(
                    assignment.field,
                    HledgerField::Numbered {
                        base: NumberedField::Account,
                        ..
                    }
                ) =>
            {
                plan.order[at] = Slot::Replace(
                    item.id,
                    ItemBody::Assignment {
                        field: assignment.field,
                        value: format!("{}:edited", &doc.text()[assignment.value_span.clone()]),
                    },
                );
                touched = true;
            }
            ItemKind::IfBlock(_) => {
                let Some(ItemBody::IfBlock {
                    matchers,
                    assignments,
                }) = block_body(doc, item)
                else {
                    continue;
                };
                plan.order[at] = Slot::Replace(
                    item.id,
                    ItemBody::IfBlock {
                        matchers,
                        assignments: assignments
                            .into_iter()
                            .map(|(field, value)| match field {
                                HledgerField::Numbered {
                                    base: NumberedField::Account,
                                    ..
                                } => (field, format!("{value}:edited")),
                                _ => (field, value),
                            })
                            .collect(),
                    },
                );
                touched = true;
            }
            _ => {}
        }
    }
    touched.then_some(plan)
}

/// Append a matcher to every editable conditional block.
fn add_matchers(doc: &RulesDoc) -> Option<EditPlan> {
    let mut plan = EditPlan::keep_all(doc);
    let mut touched = false;
    for item in doc.items() {
        let Some(ItemBody::IfBlock {
            mut matchers,
            assignments,
        }) = block_body(doc, item)
        else {
            continue;
        };
        matchers.push(MatcherSpec {
            scope: MatchScope::WholeRecord,
            pattern: INERT_MATCHER.to_string(),
        });
        plan.order[item.id.0 as usize] = Slot::Replace(
            item.id,
            ItemBody::IfBlock {
                matchers,
                assignments,
            },
        );
        touched = true;
    }
    touched.then_some(plan)
}

/// Append an assignment to every editable conditional block.
fn add_assignments(doc: &RulesDoc) -> Option<EditPlan> {
    let mut plan = EditPlan::keep_all(doc);
    let mut touched = false;
    for item in doc.items() {
        let Some(ItemBody::IfBlock {
            matchers,
            mut assignments,
        }) = block_body(doc, item)
        else {
            continue;
        };
        // `comment2` annotates the second posting, which every one of these
        // fixtures has, and no fixture already assigns it.
        assignments.push((
            HledgerField::Numbered {
                base: NumberedField::Comment,
                n: 2,
            },
            "rendered by ledgeline".to_string(),
        ));
        plan.order[item.id.0 as usize] = Slot::Replace(
            item.id,
            ItemBody::IfBlock {
                matchers,
                assignments,
            },
        );
        touched = true;
    }
    touched.then_some(plan)
}

/// Swap the first adjacent pair whose swap `verify` accepts.
///
/// Some pairs cannot be swapped without changing meaning — a conditional table
/// moved next to another one swallows it — and `verify` is the module's own
/// judge of that, so the scenario asks it rather than second-guessing.
fn reorder_two(doc: &RulesDoc) -> Option<EditPlan> {
    (0..doc.items().len().saturating_sub(1)).find_map(|at| {
        let mut plan = EditPlan::keep_all(doc);
        plan.order.swap(at, at + 1);
        let out = doc.apply(&plan).ok()?;
        doc.verify(&plan, &out).ok().map(|()| plan)
    })
}

/// Insert a freshly rendered top-level assignment at the end.
fn insert_assignment(doc: &RulesDoc) -> Option<EditPlan> {
    let mut plan = EditPlan::keep_all(doc);
    plan.order.push(Slot::Insert(ItemBody::Assignment {
        field: HledgerField::Comment,
        value: "rendered by ledgeline".to_string(),
    }));
    Some(plan)
}

/// Insert a freshly rendered conditional block at the end — the renderer output
/// with the least in common with the file that produced it.
fn insert_block(doc: &RulesDoc) -> Option<EditPlan> {
    let mut plan = EditPlan::keep_all(doc);
    plan.order.push(Slot::Insert(ItemBody::IfBlock {
        matchers: vec![
            MatcherSpec {
                scope: MatchScope::WholeRecord,
                pattern: INERT_MATCHER.to_string(),
            },
            MatcherSpec {
                scope: MatchScope::Field("description".to_string()),
                pattern: "NEVER MATCHES".to_string(),
            },
        ],
        assignments: vec![
            (
                HledgerField::Numbered {
                    base: NumberedField::Account,
                    n: 2,
                },
                "expenses:ledgeline:render-check".to_string(),
            ),
            (HledgerField::Comment, "rendered by ledgeline".to_string()),
        ],
    }));
    Some(plan)
}

/// The scenarios, each a non-trivial edit of a different kind.
type Scenario = (&'static str, fn(&RulesDoc) -> Option<EditPlan>);
const SCENARIOS: &[Scenario] = &[
    ("change-a-value", deepen_accounts),
    ("add-a-matcher", add_matchers),
    ("add-an-assignment", add_assignments),
    ("reorder-two-items", reorder_two),
    ("insert-an-assignment", insert_assignment),
    ("insert-a-block", insert_block),
];

/// `text` with a conditional TABLE glued to EOF — the shape that used to make
/// appending a rule impossible.
///
/// A table's extent runs to the first empty line or to EOF, so this one has no
/// terminating blank line, and the file has no final newline either: an edit
/// that places anything after it needs BOTH supplied. Every scenario is then run
/// against this variant as well as against the fixture, which is what puts the
/// renderer's supplied terminators in front of the only authority on whether
/// they are the right ones.
///
/// Written as TEXT rather than through the engine because a table is keep-only —
/// the renderer will never write one — and a hand-maintained file is exactly
/// where they come from. The row's matcher appears in no fixture CSV, so the
/// table parses and imports nothing.
fn with_a_table_at_eof(text: &str) -> String {
    format!(
        "{}\n\nif,account2\n{INERT_MATCHER},expenses:ledgeline:table",
        text.trim_end()
    )
}

/// Run `hledger -f <csv> print`, returning its combined output on failure.
fn hledger_reads(csv: &Path) -> Result<(), String> {
    let output = Command::new("hledger")
        .arg("-f")
        .arg(csv)
        .arg("print")
        .output()
        .map_err(|e| format!("could not run hledger: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "hledger exited {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

#[test]
fn hledger_reads_every_edit_this_renderer_writes() {
    if std::env::var_os(OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {OPT_IN}=1 to run the hledger renderer check");
        return;
    }

    let scratch = Scratch::new("edits");
    let mut checked = 0usize;
    let mut problems: Vec<String> = Vec::new();

    for (stem, rules_path, csv_path) in data_backed_fixtures() {
        let text = std::fs::read_to_string(&rules_path)
            .unwrap_or_else(|e| panic!("{} readable: {e}", rules_path.display()));
        // The fixture as committed, and the same fixture ending in a conditional
        // table with no terminator of any kind. Every scenario runs against both.
        let variants = [
            ("", text.clone()),
            ("-after-a-table", with_a_table_at_eof(&text)),
        ];

        for (suffix, text) in variants {
            let doc = RulesDoc::parse(&text);

            for (name, build) in SCENARIOS {
                let case = format!("{stem}-{name}{suffix}");
                let Some(plan) = build(&doc) else {
                    continue;
                };
                let out = match doc.apply(&plan) {
                    Ok(out) => out,
                    Err(e) => {
                        problems.push(format!("{case}: apply refused the plan: {e}"));
                        continue;
                    }
                };
                if let Err(e) = doc.verify(&plan, &out) {
                    problems.push(format!("{case}: verify refused the result: {e}"));
                    continue;
                }

                // One scratch pair per scenario, so a failure leaves a name that
                // says which edit produced it.
                let scratch_csv = scratch.0.join(format!("{case}.csv"));
                std::fs::copy(&csv_path, &scratch_csv).expect("copy the fixture CSV");
                std::fs::write(scratch.0.join(format!("{case}.csv.rules")), &out)
                    .expect("write the edited rules file");

                checked += 1;
                if let Err(e) = hledger_reads(&scratch_csv) {
                    problems.push(format!("{case}: {e}\n--- rules ---\n{out}"));
                }
            }
        }
    }

    assert!(
        checked >= 24,
        "the scenarios should reach every data-backed fixture, with and without a \
         trailing table; only {checked} ran"
    );
    assert!(problems.is_empty(), "{}", problems.join("\n\n"));
    eprintln!("{checked} rendered rules files accepted by hledger");
}
