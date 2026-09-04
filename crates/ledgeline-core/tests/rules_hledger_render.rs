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
    ControlField, EditPlan, HledgerField, Item, ItemBody, ItemKind, MatchScope, MatcherGroupSpec,
    MatcherSpec, NumberedField, RulesDoc, Slot,
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

/// A conditional block's current matcher groups and assignments, as an
/// [`ItemBody`] to be edited.
fn block_body(doc: &RulesDoc, item: &Item) -> Option<ItemBody> {
    let block = item.if_block()?;
    Some(ItemBody::IfBlock {
        groups: block
            .groups
            .iter()
            .map(|group| MatcherGroupSpec {
                matchers: group
                    .matchers
                    .iter()
                    .map(|matcher| MatcherSpec {
                        scope: matcher.scope.clone(),
                        pattern: matcher.pattern.clone(),
                    })
                    .collect(),
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
        control: block.control.as_ref().map(|control| control.kind),
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
                    groups,
                    assignments,
                    control,
                }) = block_body(doc, item)
                else {
                    continue;
                };
                plan.order[at] = Slot::Replace(
                    item.id,
                    ItemBody::IfBlock {
                        groups,
                        control,
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
            mut groups,
            assignments,
            control,
        }) = block_body(doc, item)
        else {
            continue;
        };
        groups.push(MatcherGroupSpec {
            matchers: vec![MatcherSpec {
                scope: MatchScope::WholeRecord,
                pattern: INERT_MATCHER.to_string(),
            }],
        });
        plan.order[item.id.0 as usize] = Slot::Replace(
            item.id,
            ItemBody::IfBlock {
                groups,
                assignments,
                control,
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
            groups,
            mut assignments,
            control,
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
                groups,
                assignments,
                control,
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
    // Two groups, the second an AND chain: the renderer's `if A\n& B\nC` shape
    // in the one scenario whose output has the least in common with the file it
    // came from.
    plan.order.push(Slot::Insert(ItemBody::IfBlock {
        groups: vec![
            MatcherGroupSpec {
                matchers: vec![MatcherSpec {
                    scope: MatchScope::WholeRecord,
                    pattern: INERT_MATCHER.to_string(),
                }],
            },
            MatcherGroupSpec {
                matchers: vec![
                    MatcherSpec {
                        scope: MatchScope::Field("description".to_string()),
                        pattern: "NEVER MATCHES".to_string(),
                    },
                    MatcherSpec {
                        scope: MatchScope::Field("description".to_string()),
                        pattern: "NOR THIS".to_string(),
                    },
                ],
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
        control: None,
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

// ---------------------------------------------------------------------------
// The AND/OR semantics check
// ---------------------------------------------------------------------------
//
// The scenarios above ask hledger only "can you read this?". A renderer that
// wrote an OR list where the model said AND would pass every one of them: the
// file parses, imports, and exits 0 — it just categorises the wrong records.
// This asks the harder question, and it is the only test in the repo that does.

/// The account the AND-only block below assigns, and the row it must reach.
///
/// `AMAZON` also heads the fixture's FIRST block (`AMAZON and personal`), so an
/// inserted block that hledger read as an OR rather than an AND would swallow
/// that block's row as well — later blocks win. An assertion over every row is
/// therefore what tells AND from OR, not just an assertion over this one.
const AND_ONLY_ACCOUNT: &str = "expenses:shopping:business";

/// Replace every editable block with its own typed groups, and append one
/// freshly rendered two-matcher AND group.
///
/// The replaces put [`RulesDoc::render_if_block`]'s `&` prefixes in front of
/// hledger; the insert puts `fresh_if_block`'s there, which shares no bytes
/// with the file at all.
fn regrouped(doc: &RulesDoc) -> EditPlan {
    let mut plan = EditPlan::keep_all(doc);
    for item in doc.items() {
        if let Some(body) = block_body(doc, item) {
            plan.order[item.id.0 as usize] = Slot::Replace(item.id, body);
        }
    }
    plan.order.push(Slot::Insert(ItemBody::IfBlock {
        groups: vec![MatcherGroupSpec {
            matchers: vec![
                MatcherSpec {
                    scope: MatchScope::WholeRecord,
                    pattern: "AMAZON".to_string(),
                },
                MatcherSpec {
                    scope: MatchScope::Field("card".to_string()),
                    pattern: "business".to_string(),
                },
            ],
        }],
        assignments: vec![(
            HledgerField::Numbered {
                base: NumberedField::Account,
                n: 2,
            },
            AND_ONLY_ACCOUNT.to_string(),
        )],
        control: None,
    }));
    plan
}

/// What the model says each CSV record's `account2` should be: the LAST block
/// whose OR-of-AND-groups all match, or the file's top-level default.
///
/// Deliberately a second, independent reading of the same groups — literal
/// containment rather than a regex engine, which this crate does not own and
/// the fixture does not need, every pattern in it being a literal. If this and
/// hledger disagree, one of the two is wrong about what a `&` line means, and
/// that is the finding.
fn expected_accounts(doc: &RulesDoc, records: &[&str], fields: &[&str]) -> Vec<String> {
    let default = doc
        .items()
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Assignment(a) if is_account2(a.field) => {
                Some(doc.text()[a.value_span.clone()].trim().to_string())
            }
            _ => None,
        })
        .expect("the fixture assigns a top-level account2");

    records
        .iter()
        .map(|record| {
            let column = |name: &str| {
                let at = fields.iter().position(|field| *field == name)?;
                record.split(',').nth(at)
            };
            doc.items()
                .iter()
                .filter_map(|item| item.if_block())
                .filter(|block| {
                    block.groups.iter().any(|group| {
                        group.matchers.iter().all(|matcher| {
                            let haystack = match &matcher.scope {
                                MatchScope::WholeRecord => Some(*record),
                                MatchScope::Field(name) => column(name),
                            };
                            haystack.is_some_and(|text| text.contains(&matcher.pattern))
                        })
                    })
                })
                .filter_map(|block| {
                    block
                        .assignments
                        .iter()
                        .find(|assignment| is_account2(assignment.field))
                        .map(|assignment| {
                            doc.text()[assignment.value_span.clone()].trim().to_string()
                        })
                })
                .next_back()
                .unwrap_or_else(|| default.clone())
        })
        .collect()
}

fn is_account2(field: HledgerField) -> bool {
    field
        == HledgerField::Numbered {
            base: NumberedField::Account,
            n: 2,
        }
}

/// Every posting-2 account `hledger print -O json` produced, in record order.
///
/// `-O json` rather than the human-readable output for the reason
/// `rules::matching` gives: hledger's `print` layout is a display format that
/// may change between releases, and the JSON is the one it commits to.
fn imported_accounts(csv: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("hledger")
        .args(["print", "-O", "json", "-f"])
        .arg(csv)
        .output()
        .map_err(|e| format!("could not run hledger: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "hledger exited {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("hledger JSON: {e}"))?;
    json.as_array()
        .ok_or_else(|| "hledger print -O json emits an array".to_string())?
        .iter()
        .map(|transaction| {
            transaction["tpostings"][1]["paccount"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("no second posting in {transaction}"))
        })
        .collect()
}

#[test]
fn hledger_ands_and_ors_the_groups_the_way_the_classifier_reads_them() {
    if std::env::var_os(OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {OPT_IN}=1 to run the hledger renderer check");
        return;
    }

    let root = common::fixtures_dir().join("rules/simple");
    let text = std::fs::read_to_string(root.join("and-groups.csv.rules"))
        .expect("the and-groups fixture is readable");
    let csv = std::fs::read_to_string(root.join("and-groups.csv")).expect("its CSV is readable");
    let mut rows = csv.lines();
    let fields = rows
        .next()
        .expect("a header row")
        .split(',')
        .collect::<Vec<_>>();
    let records = rows.collect::<Vec<_>>();

    let doc = RulesDoc::parse(&text);
    let plan = regrouped(&doc);
    let out = doc.apply(&plan).expect("the regrouped plan renders");
    doc.verify(&plan, &out).expect("and verifies");

    let scratch = Scratch::new("and-groups");
    let scratch_csv = scratch.0.join("and-groups.csv");
    std::fs::write(&scratch_csv, &csv).expect("write the CSV");
    std::fs::write(scratch.0.join("and-groups.csv.rules"), &out).expect("write the rules");

    // Read the EXPECTATION off what was written, not off what went in: the
    // claim under test is that our rendered `&` lines mean what our parsed
    // groups say, so both halves have to come from the same bytes.
    let reparsed = RulesDoc::parse(&out);
    let expected = expected_accounts(&reparsed, &records, &fields);
    let actual = imported_accounts(&scratch_csv).expect("hledger reads the rendered file");

    assert_eq!(actual, expected, "rendered rules:\n{out}");
    assert!(
        actual.iter().any(|account| account == AND_ONLY_ACCOUNT),
        "the inserted AND-only block must reach a record, or this proves nothing:\n{out}"
    );
    // The AND is doing work: `AMAZON` alone reaches two records, and only one of
    // them also carries `card=business`.
    assert_eq!(
        actual
            .iter()
            .filter(|account| *account == AND_ONLY_ACCOUNT)
            .count(),
        1,
        "an inserted `&` line hledger read as OR would have taken both AMAZON \
         records:\n{out}"
    );
    // The fixture's same-line `&&` blocks, under the same guard. Each is an AND
    // of conditions that individually reach MORE rows than the conjunction
    // does, so a `&&` hledger read as an OR — or that this module split
    // somewhere hledger does not — shows up as a different count.
    for (account, hits) in [
        ("expenses:shopping:target", 1),
        ("expenses:office:supplies", 1),
    ] {
        assert_eq!(
            actual.iter().filter(|got| *got == account).count(),
            hits,
            "a same-line `&&` must AND exactly the rows the classifier says:\n{out}"
        );
    }
}

/// Every imported transaction's description, in record order.
///
/// The question `skip`/`end` raise is which records import **at all**, so this
/// reads the surviving descriptions rather than [`imported_accounts`]'
/// per-record account: a dropped row has no account to disagree about.
fn imported_descriptions(csv: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("hledger")
        .args(["print", "-O", "json", "-f"])
        .arg(csv)
        .output()
        .map_err(|e| format!("could not run hledger: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "hledger exited {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("hledger JSON: {e}"))?;
    json.as_array()
        .ok_or_else(|| "hledger print -O json emits an array".to_string())?
        .iter()
        .map(|transaction| {
            transaction["tdescription"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("no description in {transaction}"))
        })
        .collect()
}

#[test]
fn hledger_skips_and_ends_exactly_where_the_classifier_says() {
    if std::env::var_os(OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {OPT_IN}=1 to run the hledger renderer check");
        return;
    }

    let root = common::fixtures_dir().join("rules/simple");
    let text = std::fs::read_to_string(root.join("control-flow.csv.rules"))
        .expect("the control-flow fixture is readable");
    let csv = std::fs::read_to_string(root.join("control-flow.csv")).expect("its CSV is readable");

    // What the classifier claims about the file, asserted before hledger is
    // asked anything: two editable control blocks, one of each word.
    let doc = RulesDoc::parse(&text);
    let controls = doc
        .items()
        .iter()
        .filter_map(|item| Some(item.if_block()?.control.as_ref()?.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        controls,
        vec![ControlField::Skip, ControlField::End],
        "the fixture's `skip` and `end` blocks must both be editable, not opaque"
    );

    let scratch = Scratch::new("control-flow");
    let scratch_csv = scratch.0.join("control-flow.csv");
    std::fs::write(&scratch_csv, &csv).expect("write the CSV");

    // Unedited first, so the expectation below is anchored to the fixture
    // rather than to whatever the renderer happens to produce.
    std::fs::write(scratch.0.join("control-flow.csv.rules"), &text).expect("write the rules");
    assert_eq!(
        imported_descriptions(&scratch_csv).expect("hledger reads the fixture"),
        vec!["COFFEE SHOP", "GROCERY MART"],
        "`skip` drops its own row and `end` drops the rest of the file"
    );

    // Now save it: re-render every block from its own typed body — which drives
    // the control word through the splicing path — and insert a FRESH `skip`
    // block, whose bytes `fresh_if_block` wrote and which shares nothing with
    // the file. Placed last, AFTER the `end` block: verified against 1.52,
    // reordering a control block among other blocks does not change a row.
    let mut plan = EditPlan::keep_all(&doc);
    for item in doc.items() {
        if let Some(body) = block_body(&doc, item) {
            plan.order[item.id.0 as usize] = Slot::Replace(item.id, body);
        }
    }
    plan.order.push(Slot::Insert(ItemBody::IfBlock {
        groups: vec![MatcherGroupSpec {
            matchers: vec![MatcherSpec {
                scope: MatchScope::Field("description".to_string()),
                pattern: "COFFEE".to_string(),
            }],
        }],
        assignments: Vec::new(),
        control: Some(ControlField::Skip),
    }));
    let out = doc.apply(&plan).expect("the control-flow plan renders");
    doc.verify(&plan, &out).expect("and verifies");

    // The re-rendered blocks are byte-identical: nothing about them changed, so
    // the leaf splicer must not have moved a byte.
    assert!(
        out.starts_with(&text),
        "re-rendering an unchanged control block must reproduce it:\n{out}"
    );

    std::fs::write(scratch.0.join("control-flow.csv.rules"), &out).expect("write the rules");
    assert_eq!(
        imported_descriptions(&scratch_csv).expect("hledger reads the saved file"),
        vec!["GROCERY MART"],
        "a freshly rendered `skip` must drop its row too:\n{out}"
    );
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
