//! Integration + property suite for the CSV import-rules model
//! ([`ledgeline_core::rules`]).
//!
//! This is the regression net for every later step of the imports work. The
//! headline test is [`round_trip_identity`]: parsing a rules file and rendering
//! the identity [`EditPlan`] must reproduce it byte for byte, over a corpus that
//! deliberately includes the things that break naive text handling — CRLF, a
//! UTF-8 BOM, no final newline, a zero-byte file, and a file that is nothing but
//! comments.
//!
//! The property tests matter more than the fixtures for the *model*: a
//! hand-written corpus never reaches the strange `Opaque` paths (an `if` table
//! glued to EOF, a matcher run of `#` lines, a whitespace-only indented body
//! line), and those are exactly where a span model loses a byte.
//!
//! Step 3's classification adds a second contract to check here, and it is the
//! one the editing step will splice against: every span a *typed* item records
//! for one of its parts must lie inside that item's own body and must cover the
//! text the model claims for it. See [`part_spans`]. The fixture corpus pins
//! which constructs classify — `simple/` entirely, `advanced/mixed.csv.rules`
//! not at all — because "the classifier quietly stopped recognizing `account2`"
//! is otherwise a silent regression that every byte-level test still passes.

mod common;

use ledgeline_core::rules::{
    Assignment, BalanceType, DirectiveName, DirectiveValue, EditPlan, IfBlock, Item, ItemBody,
    ItemId, ItemKind, MatchScope, Matcher, MatcherGroupSpec, MatcherSpec, Newline, OpaqueReason,
    RulesDoc, RulesError, Separator, Slot, Span,
};
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

/// Every `fixtures/rules/**/*.rules`, as `(relative name, text)`, sorted so the
/// suite fails in a stable order.
fn fixtures() -> Vec<(String, String)> {
    let root = common::fixtures_dir().join("rules");
    let mut paths = Vec::new();
    collect_rules(&root, &mut paths);
    paths.sort();
    assert!(
        paths.len() >= 8,
        "expected the committed rules fixtures under {}",
        root.display()
    );
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} readable: {e}", path.display()));
            (name, text)
        })
        .collect()
}

fn collect_rules(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{} readable: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("readable directory entry").path();
        if path.is_dir() {
            collect_rules(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rules") {
            out.push(path);
        }
    }
}

fn fixture_text(name: &str) -> String {
    fixtures()
        .into_iter()
        .find(|(fixture, _)| fixture == name)
        .unwrap_or_else(|| panic!("fixture {name} exists"))
        .1
}

// ---------------------------------------------------------------------------
// Shared assertions
// ---------------------------------------------------------------------------

/// The invariant the whole model rests on: spans partition the text.
///
/// Re-implemented here rather than exported from the crate on purpose — a test
/// that checks the invariant with the same code that establishes it checks
/// nothing.
fn tiles(doc: &RulesDoc) -> bool {
    let items = doc.items();
    let contiguous = items
        .windows(2)
        .all(|pair| pair[0].span.end == pair[1].span.start);
    let bounded = match (items.first(), items.last()) {
        (Some(first), Some(last)) => first.span.start == 0 && last.span.end == doc.text().len(),
        _ => doc.text().is_empty(),
    };
    contiguous && bounded
}

fn item_texts(doc: &RulesDoc) -> Vec<&str> {
    doc.items()
        .iter()
        .map(|item| &doc.text()[item.span.clone()])
        .collect()
}

fn plan_for(order: &[ItemId], delete: &[ItemId]) -> EditPlan {
    EditPlan {
        order: order.iter().copied().map(Slot::Keep).collect(),
        delete: delete.to_vec(),
    }
}

fn ids(doc: &RulesDoc) -> Vec<ItemId> {
    doc.items().iter().map(|item| item.id).collect()
}

/// The text a kept-only plan must produce: each part's bytes, in order, plus the
/// terminators a part that is no longer last needs in order to keep meaning what
/// it meant.
///
/// There are two, and both can only fire on the item that used to end the file —
/// every other item's span ends where the next one's begins, mid-file, after a
/// line break and after whatever its own extent required.
///
/// 1. **A line terminator**, if the part has none. Without it the part and its
///    new successor are spliced into one line.
/// 2. **A blank line**, if the part is a conditional TABLE with none. A table's
///    extent runs to the first empty line or to EOF, so without it the following
///    item's lines are read as more of the table's data rows.
///
/// Both are corruptions `verify` catches, which is why supplying them is what
/// makes moving the last item — or adding one after it — legal at all.
fn expected_reorder(doc: &RulesDoc, order: &[ItemId]) -> String {
    let last = order.len().saturating_sub(1);
    let newline = doc.newline().as_str();
    order
        .iter()
        .enumerate()
        .map(|(at, id)| {
            let part = doc.item_text(*id).expect("known id").to_string();
            if at == last {
                return part;
            }
            let terminated = if part.ends_with('\n') {
                part
            } else {
                part + newline
            };
            if needs_a_blank_line(doc, *id) {
                terminated + newline
            } else {
                terminated
            }
        })
        .collect()
}

/// Is this item a conditional table with no empty line after its body — i.e. one
/// whose extent would swallow whatever is placed after it?
///
/// Asked of the item's TRAILING RUN (`body.end..span.end`) rather than of its
/// last bytes, because that run is exactly where a construct's terminator lives.
fn needs_a_blank_line(doc: &RulesDoc, id: ItemId) -> bool {
    let Some(item) = doc.items().iter().find(|item| item.id == id) else {
        return false;
    };
    let is_table = item
        .opaque()
        .is_some_and(|opaque| opaque.reason == OpaqueReason::IfTable);
    is_table
        && !doc.text()[item.body.end..item.span.end]
            .lines()
            .any(str::is_empty)
}

// ---------------------------------------------------------------------------
// (1) Round-trip identity — the headline test
// ---------------------------------------------------------------------------

#[test]
fn round_trip_identity() {
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        let plan = EditPlan::keep_all(&doc);
        assert_eq!(
            doc.apply(&plan).as_deref(),
            Ok(text.as_str()),
            "{name}: the identity plan must reproduce the file byte for byte"
        );
    }
}

#[test]
fn spans_tile_every_fixture() {
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        assert!(tiles(&doc), "{name}: item spans must partition the text");
        assert_eq!(
            item_texts(&doc).concat(),
            text,
            "{name}: concatenating the items must reproduce the text"
        );
        // Every body sits inside its own span.
        assert!(
            doc.items()
                .iter()
                .all(|item| item.span.start <= item.body.start && item.body.end <= item.span.end),
            "{name}: every body must lie inside its span"
        );
    }
}

// ---------------------------------------------------------------------------
// (2) Reorder isolation
// ---------------------------------------------------------------------------

#[test]
fn swapping_adjacent_items_permutes_the_parts() {
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        let original = item_texts(&doc);
        for at in 0..doc.items().len().saturating_sub(1) {
            let mut order = ids(&doc);
            order.swap(at, at + 1);

            // A byte-order mark is only a byte-order mark at offset 0, so the
            // one swap that would move it away from the front is refused rather
            // than performed. Everything else about the fixture still swaps.
            if at == 0 && text.starts_with('\u{feff}') {
                assert_eq!(
                    doc.apply(&plan_for(&order, &[])),
                    Err(RulesError::BomMustLeadDocument),
                    "{name}: swapping the byte-order-marked item off the front must be refused"
                );
                continue;
            }

            let out = doc
                .apply(&plan_for(&order, &[]))
                .unwrap_or_else(|e| panic!("{name}: swap at {at} should apply: {e}"));

            // The multiset of parts is unchanged...
            let mut before = original.clone();
            let mut after: Vec<&str> = order
                .iter()
                .map(|id| doc.item_text(*id).expect("known id"))
                .collect();
            before.sort_unstable();
            after.sort_unstable();
            assert_eq!(before, after, "{name}: swap at {at} changed the parts");

            // ...and the output is exactly those parts in the requested order,
            // give or take the terminators a formerly-last part needs.
            let expected = expected_reorder(&doc, &order);
            assert_eq!(
                out, expected,
                "{name}: swap at {at} did not emit the requested order"
            );
            // Nothing but those terminators is ever added, and there are at most
            // two of them: a line terminator and a table's blank line.
            assert!(
                out.len() >= text.len()
                    && out.len() - text.len() <= 2 * doc.newline().as_str().len(),
                "{name}: swap at {at} changed length by more than the supplied terminators"
            );

            // The result is still a document this model can read back.
            let reparsed = RulesDoc::parse(&out);
            assert!(tiles(&reparsed), "{name}: swap at {at} broke the tiling");
        }
    }
}

// ---------------------------------------------------------------------------
// (3) Delete isolation
// ---------------------------------------------------------------------------

#[test]
fn deleting_each_item_leaves_the_rest_byte_identical() {
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        for victim in ids(&doc) {
            let survivors: Vec<ItemId> = ids(&doc).into_iter().filter(|id| *id != victim).collect();
            let out = doc
                .apply(&plan_for(&survivors, &[victim]))
                .unwrap_or_else(|e| panic!("{name}: deleting {victim} should apply: {e}"));

            let expected: String = survivors
                .iter()
                .map(|id| doc.item_text(*id).expect("known id"))
                .collect();
            assert_eq!(
                out, expected,
                "{name}: deleting {victim} disturbed a survivor"
            );
            assert_eq!(
                out.len(),
                text.len() - doc.item_text(victim).expect("known id").len(),
                "{name}: deleting {victim} removed the wrong number of bytes"
            );
            assert!(tiles(&RulesDoc::parse(&out)));
        }
    }
}

// ---------------------------------------------------------------------------
// (4) verify
// ---------------------------------------------------------------------------

#[test]
fn verify_accepts_the_identity_plan() {
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        let plan = EditPlan::keep_all(&doc);
        assert_eq!(doc.verify(&plan, &text), Ok(()), "{name}");
    }
}

#[test]
fn verify_rejects_sabotaged_text() {
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        let plan = EditPlan::keep_all(&doc);

        // Extra bytes.
        assert_eq!(
            doc.verify(&plan, &format!("{text}; sabotage\n")),
            Err(RulesError::RoundTripMismatch),
            "{name}: appended text must be rejected"
        );
        // A same-length substitution, which no length check would notice.
        let flipped: String = text
            .chars()
            .map(|c| if c == 'e' { 'E' } else { c })
            .collect();
        if flipped != text {
            assert_eq!(
                doc.verify(&plan, &flipped),
                Err(RulesError::RoundTripMismatch),
                "{name}: a substituted byte must be rejected"
            );
        }
        // Truncation.
        if !text.is_empty() {
            assert_eq!(
                doc.verify(&plan, ""),
                Err(RulesError::RoundTripMismatch),
                "{name}: truncation must be rejected"
            );
        }
    }
}

#[test]
fn a_reorder_past_a_missing_final_newline_supplies_the_terminator() {
    // `no-final-newline.rules` ends without a terminator. Emitting its last item
    // in the middle used to concatenate it into the following line: every byte
    // survived and the meaning did not, so `verify` refused a reorder that was
    // perfectly reasonable. The renderer now supplies the terminator rather than
    // trusting the bytes, so the same plan succeeds and verifies — and the item
    // is emitted as its own line rather than glued to its neighbour.
    let text = fixture_text("edge/no-final-newline.rules");
    let doc = RulesDoc::parse(&text);
    assert_eq!(doc.items().len(), 3, "fixture shape changed");
    assert!(!text.ends_with('\n'));

    let plan = plan_for(&[ItemId(0), ItemId(2), ItemId(1)], &[]);
    let out = doc.apply(&plan).expect("the plan is valid");
    assert_eq!(
        out.len(),
        text.len() + 1,
        "exactly one terminator was supplied"
    );
    assert!(!out.contains("coffeefields"), "the items stayed separate");
    assert_eq!(
        out,
        expected_reorder(&doc, &[ItemId(0), ItemId(2), ItemId(1)])
    );
    assert_eq!(doc.verify(&plan, &out), Ok(()));

    // And the moved item still ends the file without a terminator when it is
    // still the one at the end: nothing is added that was not needed.
    let identity = EditPlan::keep_all(&doc);
    assert_eq!(doc.apply(&identity).as_deref(), Ok(text.as_str()));
}

// ---------------------------------------------------------------------------
// Fixture shapes — these pin the extent rules against real files
// ---------------------------------------------------------------------------

#[test]
fn well_formed_fixtures_warn_about_nothing() {
    // `simple/` and `advanced/` are files hledger accepts. A warning here means
    // an extent rule is wrong — e.g. a block whose assignment run was cut short
    // looks like a block with no assignments.
    for (name, text) in fixtures() {
        if !(name.starts_with("simple") || name.starts_with("advanced")) {
            continue;
        }
        let doc = RulesDoc::parse(&text);
        assert!(
            doc.warnings().is_empty(),
            "{name}: unexpected warnings {:?}",
            doc.warnings()
        );
    }
}

/// A comparable, span-free summary of what an item classified as.
///
/// Spelled out here rather than exported from the crate: an integration test
/// that describes shapes in the crate's own words checks less than one that
/// describes them in its own.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shape {
    Trivia,
    Directive,
    Include,
    Fields,
    Assignment,
    IfBlock,
    Opaque(OpaqueReason),
}

fn shapes(doc: &RulesDoc) -> Vec<Shape> {
    doc.items()
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Trivia => Shape::Trivia,
            ItemKind::Directive(_) => Shape::Directive,
            ItemKind::Include(_) => Shape::Include,
            ItemKind::Fields(_) => Shape::Fields,
            ItemKind::Assignment(_) => Shape::Assignment,
            ItemKind::IfBlock(_) => Shape::IfBlock,
            ItemKind::Opaque(opaque) => Shape::Opaque(opaque.reason),
        })
        .collect()
}

#[test]
fn checking_fixture_has_the_expected_paragraphs() {
    let text = fixture_text("simple/checking.csv.rules");
    let doc = RulesDoc::parse(&text);
    assert_eq!(
        shapes(&doc),
        vec![
            Shape::Trivia,     // the header comment block + blank
            Shape::Directive,  // skip 1
            Shape::Fields,     // fields
            Shape::Directive,  // date-format
            Shape::Assignment, // currency + blank
            Shape::Assignment, // account1
            Shape::Assignment, // account2 + blank
            Shape::IfBlock,    // if ACME PAYROLL
            Shape::IfBlock,    // bare if with stacked matchers
            Shape::IfBlock,    // ; comment + if LANDLORD LLC
        ]
    );
    // The commented block carries its comment: the item starts at the `;` line
    // but the body starts at the `if`.
    let annotated = doc.items().last().expect("items");
    assert!(text[annotated.span.clone()].starts_with("; Rent is always"));
    assert!(text[annotated.body.clone()].starts_with("if LANDLORD LLC"));
}

#[test]
fn and_groups_fixture_classifies_as_an_or_of_and_groups() {
    // The `&`-chain fixture is EDITABLE, which is the whole point of it: before
    // AND-groups existed here every one of these three blocks was
    // `Opaque(CombinedMatcher)`. The round-trip, reorder, delete, part-span and
    // edit-isolation suites above all reach it through `fixtures()`, so the
    // multi-group round trip is covered by the same proofs as every other file.
    let text = fixture_text("simple/and-groups.csv.rules");
    let doc = RulesDoc::parse(&text);
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());

    let blocks = doc
        .items()
        .iter()
        .filter_map(|item| item.if_block())
        .map(|block| {
            block
                .groups
                .iter()
                .map(|group| {
                    group
                        .matchers
                        .iter()
                        .map(|matcher| (matcher.scope.clone(), matcher.pattern.as_str()))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let whole = |pattern| (MatchScope::WholeRecord, pattern);
    let card = |pattern| (MatchScope::Field("card".to_string()), pattern);
    let field = |name: &str, pattern| (MatchScope::Field(name.to_string()), pattern);
    assert_eq!(
        blocks,
        vec![
            // Inline header, one AND-group.
            vec![vec![whole("AMAZON"), card("personal")]],
            // Stacked, two AND-groups OR-ed.
            vec![
                vec![whole("GROCER"), card("personal")],
                vec![whole("FARMERS"), card("business")],
            ],
            // A plain OR list is one matcher per group, not a second shape.
            vec![vec![whole("SHELL")], vec![whole("CHEVRON")]],
            // hledger's same-line `&&`: one group, exactly as the `&`-chain
            // above, and each piece scoped on its own.
            vec![vec![field("description", "TARGET"), card("business")]],
            // The two spellings composed: a leading `&` continues the group the
            // line above opened, and that line's own `&&` splits it further —
            // one group of three, not two groups.
            vec![vec![
                whole("STAPLES"),
                card("business"),
                field("amount", "-85"),
            ]],
        ]
    );
}

#[test]
fn mixed_fixture_keeps_the_table_and_the_blocks_whole() {
    let text = fixture_text("advanced/mixed.csv.rules");
    let doc = RulesDoc::parse(&text);

    let table = doc
        .items()
        .iter()
        .find(|item| {
            item.opaque()
                .is_some_and(|o| o.reason == OpaqueReason::IfTable)
        })
        .expect("the fixture has a conditional table");
    // Header + three data rows, with the terminating blank line in the span but
    // not the body.
    assert_eq!(table.opaque().expect("opaque").lines, 4);
    assert!(text[table.body.clone()].ends_with("LANDLORD LLC,expenses:home:rent,rent\n"));
    assert!(text[table.span.clone()].ends_with("rent\n\n"));

    // Six conditional blocks, and every one of them keeps its tab-indented
    // assignment lines. None is editable — see `mixed_fixture_is_all_opaque`.
    let blocks: Vec<_> = doc
        .items()
        .iter()
        .filter(|item| {
            let body = &text[item.body.clone()];
            body.starts_with("if\n") || body.starts_with("if ")
        })
        .collect();
    assert_eq!(blocks.len(), 6);
    assert!(
        blocks
            .iter()
            .all(|block| text[block.body.clone()].contains('\t'))
    );
}

// ---------------------------------------------------------------------------
// Classification against the fixture corpus
// ---------------------------------------------------------------------------

#[test]
fn simple_fixtures_classify_completely() {
    // `simple/` is the shape most rules files actually have. If anything in one
    // of these is opaque, the classifier has a hole a real user would hit — so
    // the assertion is "no `Opaque` at all", not "few".
    let expected_blocks = [
        ("simple/checking.csv.rules", 3),
        ("simple/creditcard1.csv.rules", 2),
    ];
    for (name, blocks) in expected_blocks {
        let text = fixture_text(name);
        let doc = RulesDoc::parse(&text);
        let opaque: Vec<_> = doc
            .items()
            .iter()
            .filter_map(|item| item.opaque().map(|o| (item.line, o.reason, &o.label)))
            .collect();
        assert!(
            opaque.is_empty(),
            "{name}: unexpected opaque items {opaque:?}"
        );
        assert_eq!(
            doc.items()
                .iter()
                .filter(|item| item.if_block().is_some())
                .count(),
            blocks,
            "{name}: editable conditional blocks"
        );
    }
}

#[test]
fn mixed_fixture_is_all_opaque_and_names_why() {
    // `advanced/mixed.csv.rules` exists to exercise the constructs an editor
    // must decline. Not one of its conditionals may be editable, and each must
    // say which rule stopped it.
    let text = fixture_text("advanced/mixed.csv.rules");
    let doc = RulesDoc::parse(&text);

    assert_eq!(
        doc.items()
            .iter()
            .filter(|item| item.if_block().is_some())
            .count(),
        0,
        "no conditional in this fixture may be editable"
    );

    let reasons: BTreeSet<OpaqueReason> = doc
        .items()
        .iter()
        .filter_map(|item| item.opaque().map(|o| o.reason))
        .collect();
    assert_eq!(
        reasons,
        BTreeSet::from([
            OpaqueReason::IfTable,
            OpaqueReason::CombinedMatcher,
            OpaqueReason::MatchGroup,
            OpaqueReason::CommentLikeMatcher,
            OpaqueReason::ControlFlowInBlock,
        ]),
        "the reasons this fixture exercises"
    );

    // Everything else in it classifies, so an opaque item here is always a
    // conditional and never a directive the classifier failed to read.
    assert_eq!(
        shapes(&doc)
            .into_iter()
            .filter(|shape| !matches!(shape, Shape::Opaque(_)))
            .collect::<Vec<_>>(),
        vec![
            Shape::Trivia,    // the header comment block + blank
            Shape::Directive, // separator:,
            Shape::Directive, // skip 1
            Shape::Fields,    // fields date, description, amount, note
        ]
    );
}

#[test]
fn every_fixture_projects_settings_that_name_real_items() {
    // `settings()` is a view, not a copy: every entry must point at an item this
    // document actually has, or the preferences panel edits a ghost.
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        let settings = doc.settings();
        for (label, id) in [
            ("skip", settings.skip.as_ref().map(|s| s.item)),
            ("fields", settings.fields.as_ref().map(|s| s.item)),
            ("separator", settings.separator.as_ref().map(|s| s.item)),
            ("date_format", settings.date_format.as_ref().map(|s| s.item)),
            ("account1", settings.account1.as_ref().map(|s| s.item)),
            ("account2", settings.account2.as_ref().map(|s| s.item)),
            ("currency", settings.currency.as_ref().map(|s| s.item)),
            (
                "balance_type",
                settings.balance_type.as_ref().map(|s| s.item),
            ),
        ] {
            if let Some(id) = id {
                assert!(
                    doc.item_text(id).is_some(),
                    "{name}: {label} names a missing item {id}"
                );
            }
        }
    }
}

#[test]
fn simple_fixtures_project_the_settings_they_state() {
    let doc = RulesDoc::parse(&fixture_text("simple/creditcard1.csv.rules"));
    let settings = doc.settings();
    assert_eq!(settings.skip.map(|s| s.value), Some(1));
    assert_eq!(
        settings.date_format.map(|s| s.value),
        Some("%Y-%m-%d".to_string())
    );
    assert_eq!(settings.currency.map(|s| s.value), Some("$".to_string()));
    assert_eq!(
        settings.account1.map(|s| s.value),
        Some("liabilities:creditcard:visa".to_string())
    );
    assert_eq!(
        settings.fields.map(|s| s.value),
        Some(vec![
            "date".to_string(),
            "description".to_string(),
            "amount-in".to_string(),
            "amount-out".to_string(),
        ])
    );
    // The file states no `source`, and "not stated" is not the same as a
    // default — so the projection says nothing rather than guessing.
    assert!(settings.source.is_none());
}

#[test]
fn edge_fixtures_keep_their_awkward_bytes() {
    let empty = RulesDoc::parse(&fixture_text("edge/empty.rules"));
    assert!(empty.items().is_empty());
    assert_eq!(empty.apply(&EditPlan::keep_all(&empty)).as_deref(), Ok(""));

    let comments = RulesDoc::parse(&fixture_text("edge/only-comments.rules"));
    assert_eq!(comments.items().len(), 1, "one unbroken trivia run");
    assert_eq!(comments.items()[0].kind, ItemKind::Trivia);

    let crlf_text = fixture_text("edge/crlf.rules");
    let crlf = RulesDoc::parse(&crlf_text);
    assert_eq!(crlf.newline(), Newline::CrLf);
    assert!(
        crlf.items()
            .iter()
            .all(|item| crlf_text[item.span.clone()].contains("\r\n")),
        "every paragraph keeps its CRLF terminators"
    );

    let bom_text = fixture_text("edge/bom.rules");
    let bom = RulesDoc::parse(&bom_text);
    assert!(bom_text.starts_with('\u{feff}'), "fixture has a BOM");
    assert!(
        bom.item_text(ItemId(0))
            .is_some_and(|text| text.starts_with('\u{feff}')),
        "the BOM stays in the first item, not in a preamble the model forgets"
    );

    let no_newline = RulesDoc::parse(&fixture_text("edge/no-final-newline.rules"));
    let last = no_newline.items().last().expect("items");
    assert!(!no_newline.text()[last.span.clone()].ends_with('\n'));
}

// ---------------------------------------------------------------------------
// Part spans — the contract step 4 splices against
// ---------------------------------------------------------------------------

/// The spans a typed item records, each with the text it claims to cover (or
/// `None` where the model keeps no copy to compare against).
///
/// Two things must hold for every one of these, and step 4's splicing is wrong
/// the moment either stops holding: the span lies inside the item's own body, or
/// an edit reaches into a neighbour; and it covers the text the model reports,
/// or an edit replaces the wrong bytes.
fn block_matchers(block: &IfBlock) -> impl Iterator<Item = &Matcher> {
    block.groups.iter().flat_map(|group| &group.matchers)
}

fn part_spans(kind: &ItemKind) -> Vec<(Span, Option<&str>)> {
    fn assignment_spans(assignment: &Assignment) -> Vec<(Span, Option<&str>)> {
        vec![
            (assignment.field_span.clone(), None),
            (assignment.sep_span.clone(), None),
            (assignment.value_span.clone(), None),
        ]
    }

    match kind {
        ItemKind::Trivia | ItemKind::Opaque(_) => Vec::new(),
        ItemKind::Directive(directive) => vec![
            (directive.name_span.clone(), None),
            (
                directive.value_span.clone(),
                match &directive.value {
                    DirectiveValue::Source { raw, .. } => Some(raw.as_str()),
                    DirectiveValue::Text(text) => Some(text.as_str()),
                    _ => None,
                },
            ),
        ],
        ItemKind::Include(include) => {
            vec![(include.target_span.clone(), Some(include.target.as_str()))]
        }
        ItemKind::Fields(fields) => fields
            .name_spans
            .iter()
            .cloned()
            .zip(fields.names.iter().map(|name| Some(name.as_str())))
            .chain(std::iter::once((fields.tail_span.clone(), None)))
            .collect(),
        ItemKind::Assignment(assignment) => assignment_spans(assignment),
        ItemKind::IfBlock(block) => std::iter::once((block.indent.clone(), None))
            .chain(block_matchers(block).flat_map(|matcher| {
                std::iter::once((matcher.pattern_span.clone(), Some(matcher.pattern.as_str())))
                    .chain(matcher.field_span.clone().map(|span| {
                        (
                            span,
                            match &matcher.scope {
                                MatchScope::Field(name) => Some(name.as_str()),
                                MatchScope::WholeRecord => None,
                            },
                        )
                    }))
            }))
            .chain(block.assignments.iter().flat_map(assignment_spans))
            .collect(),
    }
}

fn assert_part_spans(doc: &RulesDoc, name: &str) {
    for item in doc.items() {
        for (span, claimed) in part_spans(&item.kind) {
            assert!(
                item.body.start <= span.start && span.end <= item.body.end,
                "{name}: part span {span:?} escapes item {} body {:?}",
                item.id,
                item.body
            );
            if let Some(claimed) = claimed {
                assert_eq!(
                    &doc.text()[span.clone()],
                    claimed,
                    "{name}: part span {span:?} does not cover what it claims"
                );
            }
        }
    }
}

#[test]
fn every_typed_span_stays_inside_its_item_and_covers_what_it_claims() {
    for (name, text) in fixtures() {
        assert_part_spans(&RulesDoc::parse(&text), &name);
    }
}

// ---------------------------------------------------------------------------
// (5) Edit isolation — the analogue of `edit.rs`'s
//     `delete_keeps_other_transactions_byte_identical`
// ---------------------------------------------------------------------------

/// A plausible edit to `item`, or `None` for an item the edit policy refuses.
///
/// Spelled out here rather than driven from the crate so the corpus is edited
/// the way a client would edit it: change a value, rename a name, extend a
/// matcher. Nothing here adds or removes a leaf — the point is that a *small*
/// edit stays small, which is the property a re-rendering editor gets wrong.
fn edit_for(doc: &RulesDoc, item: &Item) -> Option<ItemBody> {
    let extended = |span: &Span| format!("{}X", &doc.text()[span.clone()]);
    match &item.kind {
        // `source` and `archive` are keep-only, for the security reason the
        // crate's `writable` documents.
        ItemKind::Directive(directive) => match directive.name {
            DirectiveName::Source | DirectiveName::Archive => None,
            name => Some(ItemBody::Directive {
                name,
                value: nudged(&directive.value),
            }),
        },
        ItemKind::Fields(fields) => Some(ItemBody::Fields {
            names: fields
                .names
                .iter()
                .enumerate()
                .map(|(at, name)| {
                    if at == 0 {
                        "edited".to_string()
                    } else {
                        name.clone()
                    }
                })
                .collect(),
        }),
        ItemKind::Assignment(assignment) => Some(ItemBody::Assignment {
            field: assignment.field,
            value: extended(&assignment.value_span),
        }),
        // Grouping is kept exactly as found: the point here is that a *small*
        // edit stays small, and re-grouping is not a small edit.
        ItemKind::IfBlock(block) => Some(ItemBody::IfBlock {
            groups: block
                .groups
                .iter()
                .enumerate()
                .map(|(group_at, group)| MatcherGroupSpec {
                    matchers: group
                        .matchers
                        .iter()
                        .enumerate()
                        .map(|(at, matcher)| MatcherSpec {
                            scope: matcher.scope.clone(),
                            pattern: if (group_at, at) == (0, 0) {
                                format!("{}X", matcher.pattern)
                            } else {
                                matcher.pattern.clone()
                            },
                        })
                        .collect(),
                })
                .collect(),
            assignments: block
                .assignments
                .iter()
                .map(|assignment| (assignment.field, extended(&assignment.value_span)))
                .collect(),
            // Kept as found for the same reason as the grouping: adding or
            // dropping a `skip`/`end` changes which rows import at all, which
            // is not the small edit this test measures.
            control: block.control.as_ref().map(|control| control.kind),
        }),
        ItemKind::Trivia | ItemKind::Include(_) | ItemKind::Opaque(_) => None,
    }
}

/// A different value of the same shape.
///
/// A flag and a `separator tab` word have nothing to nudge, so they come back
/// unchanged — the isolation property holds either way, and refusing to edit
/// them would just shrink the corpus this test covers.
fn nudged(value: &DirectiveValue) -> DirectiveValue {
    match value {
        DirectiveValue::Text(text) => DirectiveValue::Text(format!("{text}X")),
        DirectiveValue::Skip(count) => DirectiveValue::Skip(count.saturating_add(1)),
        DirectiveValue::DecimalMark('.') => DirectiveValue::DecimalMark(','),
        DirectiveValue::DecimalMark(_) => DirectiveValue::DecimalMark('.'),
        DirectiveValue::Separator(Separator::Char(';')) => {
            DirectiveValue::Separator(Separator::Char(','))
        }
        DirectiveValue::Separator(Separator::Char(_)) => {
            DirectiveValue::Separator(Separator::Char(';'))
        }
        DirectiveValue::BalanceType(BalanceType::Total) => {
            DirectiveValue::BalanceType(BalanceType::Simple)
        }
        DirectiveValue::BalanceType(_) => DirectiveValue::BalanceType(BalanceType::Total),
        other => other.clone(),
    }
}

/// A plan that keeps everything and rewrites exactly one item's body.
fn edit_plan(doc: &RulesDoc, id: ItemId, body: ItemBody) -> EditPlan {
    let mut plan = EditPlan::keep_all(doc);
    plan.order[id.0 as usize] = Slot::Replace(id, body);
    plan
}

/// Assert that `out` is `text` with exactly `item`'s span rewritten.
///
/// Returns whether the item's own bytes actually changed, so a caller can prove
/// the corpus exercised real rewrites rather than a renderer that happened to
/// reproduce every line.
fn assert_only_this_item_changed(name: &str, text: &str, item: &Item, out: &str) -> bool {
    let prefix = &text[..item.span.start];
    let suffix = &text[item.span.end..];
    assert!(
        out.len() >= prefix.len() + suffix.len(),
        "{name}: editing item {} swallowed a neighbour",
        item.id
    );
    assert_eq!(
        &out[..prefix.len()],
        prefix,
        "{name}: editing item {} disturbed an earlier item",
        item.id
    );
    assert_eq!(
        &out[out.len() - suffix.len()..],
        suffix,
        "{name}: editing item {} disturbed a later item",
        item.id
    );
    out[prefix.len()..out.len() - suffix.len()] != text[item.span.clone()]
}

#[test]
fn editing_each_item_leaves_every_other_item_byte_identical() {
    let mut rewritten = 0usize;
    for (name, text) in fixtures() {
        let doc = RulesDoc::parse(&text);
        for item in doc.items() {
            let Some(body) = edit_for(&doc, item) else {
                continue;
            };
            let plan = edit_plan(&doc, item.id, body);
            let out = doc
                .apply(&plan)
                .unwrap_or_else(|e| panic!("{name}: editing item {} should apply: {e}", item.id));

            if assert_only_this_item_changed(&name, &text, item, &out) {
                rewritten += 1;
            }
            assert_eq!(
                doc.verify(&plan, &out),
                Ok(()),
                "{name}: editing item {} must verify",
                item.id
            );
        }
    }
    // A renderer that silently re-emitted every line unchanged would satisfy
    // the isolation assertions above and prove nothing.
    assert!(
        rewritten >= 20,
        "the corpus should exercise real rewrites; only {rewritten} items changed"
    );
}

// ---------------------------------------------------------------------------
// Property tests — what actually proves the model
// ---------------------------------------------------------------------------

/// One plausible rules-file line, without its terminator.
fn line_shape() -> impl Strategy<Value = String> {
    prop_oneof![
        // Truly empty: ends a conditional block or table.
        "",
        // Whitespace-only: blank at top level, but a no-op *inside* a block.
        "[ \t]{1,4}",
        // Comment: a comment at top level, a regex inside a matcher run.
        "[;#*][ a-zA-Z0-9:]{0,24}",
        // Directive / field assignment.
        "(skip|fields|date-format|currency|separator|account1|balance-type)[ ]{1,3}[a-zA-Z0-9%/,:$= ]{0,24}",
        // Bare `if`, whose matchers are stacked on the following lines.
        "if",
        // Inline `if MATCHER`.
        "if [a-zA-Z0-9 %]{0,20}",
        // Conditional table header; the char after `if` is the separator.
        "if[,|;][a-zA-Z%,]{0,24}",
        // Matcher, table row, or plain junk at column 1.
        "[a-zA-Z0-9,%&!][a-zA-Z0-9 ,%]{0,24}",
        // Indented assignment line.
        "[ \t]{1,4}[a-zA-Z0-9]{0,12}[ ]{0,3}[a-zA-Z0-9:]{0,16}",
    ]
}

/// A whole plausible rules file, with a per-line CRLF/LF choice and an optional
/// missing final terminator.
fn rules_text() -> impl Strategy<Value = String> {
    (
        prop::collection::vec((line_shape(), any::<bool>()), 0..32),
        any::<bool>(),
    )
        .prop_map(|(lines, final_newline)| {
            let count = lines.len();
            lines
                .into_iter()
                .enumerate()
                .map(|(index, (line, crlf))| {
                    let terminator = if index + 1 == count && !final_newline {
                        ""
                    } else if crlf {
                        "\r\n"
                    } else {
                        "\n"
                    };
                    format!("{line}{terminator}")
                })
                .collect()
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The model's whole claim: parse, keep everything, and the bytes come back.
    /// This reaches the `Opaque` extents a hand-written corpus never will.
    #[test]
    fn parse_then_keep_all_is_the_identity(text in rules_text()) {
        let doc = RulesDoc::parse(&text);
        prop_assert!(tiles(&doc));
        let plan = EditPlan::keep_all(&doc);
        prop_assert_eq!(doc.apply(&plan), Ok(text.clone()));
        prop_assert_eq!(doc.verify(&plan, &text), Ok(()));
    }

    /// Swapping any adjacent pair emits the same parts, in the requested order,
    /// with no byte gained or lost — except the terminators a formerly-last part
    /// needs so that it neither fuses with its new successor nor swallows it.
    #[test]
    fn reorder_is_a_permutation(text in rules_text(), seed in 0usize..1_000_000) {
        let doc = RulesDoc::parse(&text);
        let count = doc.items().len();
        prop_assume!(count > 1);
        let at = seed % (count - 1);

        let mut order = ids(&doc);
        order.swap(at, at + 1);
        let out = doc.apply(&plan_for(&order, &[])).expect("a total plan applies");

        // At most two terminators are ever supplied, and only to the item that
        // used to end the file: a line terminator, and a conditional table's
        // blank line.
        let supplied = out.len() - text.len();
        prop_assert!(supplied <= 2 * doc.newline().as_str().len());
        prop_assert!(supplied == 0 || !text.ends_with('\n') || needs_a_blank_line(&doc, ids(&doc)[count - 1]));
        prop_assert_eq!(&out, &expected_reorder(&doc, &order));

        let mut before = item_texts(&doc);
        let mut after: Vec<&str> = order
            .iter()
            .map(|id| doc.item_text(*id).expect("known id"))
            .collect();
        before.sort_unstable();
        after.sort_unstable();
        prop_assert_eq!(before, after);
        prop_assert!(tiles(&RulesDoc::parse(&out)));
    }

    /// Classification never invents a span outside the item it belongs to, and
    /// never reports text its span does not cover. This reaches the shapes a
    /// hand-written corpus does not: an inline matcher glued to EOF, a `fields`
    /// list whose names are all empty, a value that is nothing but whitespace.
    #[test]
    fn typed_spans_stay_inside_their_items(text in rules_text()) {
        assert_part_spans(&RulesDoc::parse(&text), "generated");
    }

    /// Editing one item rewrites that item's span and not one byte outside it.
    ///
    /// The counterpart of `reorder_is_a_permutation` for the write path, and it
    /// reaches shapes the fixtures do not: a block whose only assignment is a
    /// bare field name, a `fields` list of empty names, an inline matcher glued
    /// to EOF.
    #[test]
    fn editing_one_item_leaves_the_others_byte_identical(text in rules_text(), seed in 0usize..1_000_000) {
        let doc = RulesDoc::parse(&text);
        let editable: Vec<ItemId> = doc
            .items()
            .iter()
            .filter(|item| edit_for(&doc, item).is_some())
            .map(|item| item.id)
            .collect();
        prop_assume!(!editable.is_empty());

        let item = &doc.items()[editable[seed % editable.len()].0 as usize];
        let body = edit_for(&doc, item).expect("the item was chosen because it is editable");
        let plan = edit_plan(&doc, item.id, body);

        // A generated file holds values this module will not write back — a
        // matcher scope name with a comma in it, a `\N` backreference. Refusing
        // is a correct outcome; emitting the wrong bytes is not.
        let Ok(out) = doc.apply(&plan) else { return Ok(()); };

        assert_only_this_item_changed("generated", &text, item, &out);
        prop_assert_eq!(doc.verify(&plan, &out), Ok(()));
    }

    /// Parsing is infallible for *anything*, including bytes no rules file would
    /// ever contain — and the round trip holds there too. `(?s)` so `.` can
    /// produce the newlines the line splitter is built around.
    #[test]
    fn parse_never_panics(text in "(?s).{0,4096}") {
        let doc = RulesDoc::parse(&text);
        prop_assert!(tiles(&doc));
        prop_assert_eq!(doc.apply(&EditPlan::keep_all(&doc)), Ok(text.clone()));
    }
}
