//! The budget editor against real journals: every committed `fixtures/budget`
//! file, edited every way the editor offers, with the two properties that matter
//! asserted after each one.
//!
//! The unit tests in `periodic.rs` pin the mechanics on small hand-written
//! inputs. This suite exists to answer a different question — *does it hold on
//! the files we actually ship?* — and to state the two invariants the whole
//! module is built to keep, as executable claims rather than as prose:
//!
//! 1. **Nothing else moved.** Every line of the file that the edit did not name
//!    comes back byte-identical. This is checked over the WHOLE file, not just
//!    the `~` blocks, so a splice that wandered into a transaction is caught.
//! 2. **It is still a journal, and it still means what was asked.** The result
//!    re-parses, its rules still balance (the parser refuses them otherwise), and
//!    the edited goal reads back as exactly the quantity requested.
//!
//! Property 2 is the one `PeriodicDoc::verify` deliberately does NOT make — it
//! is a text-shape model and does not parse amounts. Here it is made against a
//! real parse, which is the same check `budget_api` performs before it writes.

mod common;

use ledgeline_core::Dec;
use ledgeline_core::model::{
    Amount, AmountStyle, Commodity, CommoditySide, PeriodExpr, PeriodicTransaction, PostingType,
};
use ledgeline_core::parse_journal;
use ledgeline_core::periodic::{
    BlockBalance, GoalRequest, PeriodicDoc, PeriodicEdit, PeriodicPlan, plan,
};

/// Every `.journal` under `fixtures/budget`, as `(name, text)`.
fn budget_fixtures() -> Vec<(String, String)> {
    let dir = common::fixtures_dir().join("budget");
    let mut found: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "journal"))
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(entry.path())
                .unwrap_or_else(|e| panic!("read {name}: {e}"));
            (name, text)
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "fixtures/budget holds no journals");
    found
}

/// The file's parsed `~` rules.
fn rules(text: &str, name: &str) -> Vec<PeriodicTransaction> {
    parse_journal(text, name)
        .unwrap_or_else(|e| panic!("parse {name}: {e}"))
        .periodic_transactions
}

/// `$n` in the style the budget fixtures are written in.
fn usd(quantity: Dec) -> Amount {
    Amount {
        commodity: Commodity("$".into()),
        quantity,
        style: AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: 0,
        },
        cost: None,
    }
}

/// Property 1, over the whole file: every line the plan did not name is present,
/// unchanged, in the same relative order.
///
/// Implemented as a subsequence check rather than a set difference: a line that
/// survived but MOVED is a change to the file, and a set comparison would call
/// it identical.
fn untouched_lines_survive_in_order(before: &str, after: &str, changed: &[String]) {
    let mut remaining = after.lines();
    for line in before.lines() {
        if changed.iter().any(|c| c == line) {
            continue;
        }
        let found = remaining.by_ref().any(|candidate| candidate == line);
        assert!(
            found,
            "line {line:?} was not preserved in order\n--- after ---\n{after}"
        );
    }
}

/// The exact source lines a plan rewrites or removes — the only ones property 1
/// exempts.
///
/// A delete that takes down its whole rule (because it was the rule's last goal)
/// removes the `~` header and any comment lines with it, so the exemption is the
/// block's whole extent rather than the one named line.
fn changed_lines(doc: &PeriodicDoc, plan: &PeriodicPlan) -> Vec<String> {
    let deleted: Vec<usize> = plan
        .edits
        .iter()
        .filter(|edit| matches!(edit, PeriodicEdit::Delete { .. }))
        .filter_map(PeriodicEdit::index)
        .collect();
    let mut changed: Vec<String> = Vec::new();
    for index in plan.edits.iter().filter_map(PeriodicEdit::index) {
        let line = &doc.lines()[index];
        let block = &doc.blocks()[line.block];
        let dropped = block.lines.iter().all(|at| deleted.contains(at));
        let span = if dropped && deleted.contains(&index) {
            block.full.clone()
        } else {
            line.span.clone()
        };
        changed.extend(doc.text()[span].lines().map(str::to_string));
    }
    changed
}

/// Property 2: the result parses, and `expect` agrees with the rules it holds.
fn reparses(after: &str, name: &str) -> Vec<PeriodicTransaction> {
    parse_journal(after, name)
        .unwrap_or_else(|e| panic!("{name} no longer parses after an edit: {e}\n{after}"))
        .periodic_transactions
}

/// Setting a goal to a new number changes that number, keeps every other line,
/// and leaves a journal that still parses and still balances.
#[test]
fn setting_a_goal_moves_one_number_and_nothing_else() {
    for (name, text) in budget_fixtures() {
        let doc = PeriodicDoc::parse(&text);
        let parsed = rules(&text, &name);
        let mut edited_any = false;

        for line in doc.lines() {
            if doc.line_lock(line.index).is_some() {
                continue;
            }
            let wanted = Dec::new(4321, 2);
            let request = GoalRequest::Set {
                index: line.index,
                quantity: wanted,
            };
            let Ok(plan) = plan(&doc, &parsed, &request) else {
                // An ambiguous counter-leg is a legitimate refusal, not a
                // failure — `periodic.rs` pins that behaviour directly.
                continue;
            };
            let after = doc.apply(&plan).expect("apply");
            doc.verify(&plan, &after).expect("verify");

            untouched_lines_survive_in_order(&text, &after, &changed_lines(&doc, &plan));

            let now = reparses(&after, &name);
            let posting = &now[line.block].postings[line.at];
            assert_eq!(
                posting.amounts[0].quantity, wanted,
                "{name}: goal {} did not read back as the requested amount",
                line.account
            );
            assert_eq!(
                posting.account.0, line.account,
                "{name}: the account moved during an amount edit"
            );
            edited_any = true;
        }
        assert!(edited_any, "{name}: no goal in this fixture was editable");
    }
}

/// Adding a goal to an existing rule leaves every existing line alone and adds
/// exactly one posting to exactly one rule.
#[test]
fn adding_a_goal_adds_one_posting_to_one_rule() {
    for (name, text) in budget_fixtures() {
        let doc = PeriodicDoc::parse(&text);
        let parsed = rules(&text, &name);
        for block in doc.blocks() {
            if block.lock.is_some() {
                continue;
            }
            let request = GoalRequest::Add {
                block: block.index,
                account: "expenses:newcategory".into(),
                amount: usd(Dec::new(2500, 2)),
            };
            let Ok(plan) = plan(&doc, &parsed, &request) else {
                continue;
            };
            let after = doc.apply(&plan).expect("apply");
            doc.verify(&plan, &after).expect("verify");
            untouched_lines_survive_in_order(&text, &after, &changed_lines(&doc, &plan));

            let now = reparses(&after, &name);
            assert_eq!(now.len(), parsed.len(), "{name}: the rule count changed");
            for (at, (before, after)) in parsed.iter().zip(&now).enumerate() {
                let expected = before.postings.len() + usize::from(at == block.index);
                assert_eq!(
                    after.postings.len(),
                    expected,
                    "{name}: rule {at} gained or lost postings it should not have"
                );
            }
            let added = now[block.index]
                .postings
                .iter()
                .find(|posting| posting.account.0 == "expenses:newcategory")
                .unwrap_or_else(|| panic!("{name}: the new goal is missing"));
            assert_eq!(added.amounts[0].quantity, Dec::new(2500, 2));
        }
    }
}

/// Removing a goal removes exactly it — or, when it was a rule's last, the whole
/// rule, because a bare `~` header is not a construct to leave behind.
#[test]
fn removing_a_goal_removes_it_and_leaves_the_rest() {
    for (name, text) in budget_fixtures() {
        let doc = PeriodicDoc::parse(&text);
        let parsed = rules(&text, &name);
        for line in doc.lines() {
            let request = GoalRequest::Remove { index: line.index };
            let Ok(plan) = plan(&doc, &parsed, &request) else {
                continue;
            };
            let after = doc.apply(&plan).expect("apply");
            doc.verify(&plan, &after).expect("verify");
            untouched_lines_survive_in_order(&text, &after, &changed_lines(&doc, &plan));

            let now = reparses(&after, &name);
            let sole = doc.blocks()[line.block].lines.len() == 1;
            assert_eq!(
                now.len(),
                parsed.len() - usize::from(sole),
                "{name}: removing the last goal of a rule must remove the rule"
            );
            if !sole {
                assert!(
                    now[line.block]
                        .postings
                        .iter()
                        .all(|posting| posting.account.0 != line.account),
                    "{name}: {} survived its own removal",
                    line.account
                );
            }
        }
    }
}

/// A new rule appended at EOF cannot change what any existing byte means: the
/// original text is still a prefix of the result, and every rule that was there
/// re-parses identically.
#[test]
fn a_new_rule_is_appended_without_disturbing_the_file() {
    for (name, text) in budget_fixtures() {
        let doc = PeriodicDoc::parse(&text);
        let parsed = rules(&text, &name);
        let request = GoalRequest::AddBlock {
            period: PeriodExpr::Yearly,
            description: "annual budget".into(),
            account: "income:interest".into(),
            amount: usd(Dec::new(-120_000, 2)),
        };
        // The branch under test is "no rule states this recurrence under this
        // name yet". Asserted rather than assumed: a fixture that later grows a
        // `~ yearly  annual budget` rule must fail HERE, rather than quietly
        // exercising the joining branch under a test named for the other one.
        assert!(
            !doc.blocks()
                .iter()
                .any(|block| block.period == Some(PeriodExpr::Yearly)
                    && block.description == "annual budget"),
            "{name}: this fixture now states the rule this test appends"
        );
        let plan = plan(&doc, &parsed, &request).expect("plan");
        let after = doc.apply(&plan).expect("apply");
        doc.verify(&plan, &after).expect("verify");
        assert!(
            after.starts_with(&text),
            "{name}: an EOF append rewrote earlier bytes"
        );

        let now = reparses(&after, &name);
        assert_eq!(now.len(), parsed.len() + 1);
        for (before, after) in parsed.iter().zip(&now) {
            assert_eq!(
                (before.period, &before.description, before.postings.len()),
                (after.period, &after.description, after.postings.len()),
                "{name}: an existing rule changed"
            );
        }
        let added = now.last().expect("the appended rule");
        assert_eq!(added.period, PeriodExpr::Yearly);
        assert_eq!(added.description, "annual budget");
        assert_eq!(added.postings[0].account.0, "income:interest");
        assert_eq!(added.postings[0].ptype, PostingType::Virtual);
        assert_eq!(added.postings[0].amounts[0].quantity, Dec::new(-120_000, 2));
    }
}

/// The other branch of the same decision: when a rule ALREADY states this
/// recurrence under this name, the goal joins it and no second rule is opened.
///
/// The companion to `a_new_rule_is_appended_without_disturbing_the_file`, and
/// the invariant is made at its strongest here — splice the one inserted line
/// back out of the result and what is left must be the original file, byte for
/// byte. That covers the header, the alignment of every other goal, the
/// comments and the blank lines in one claim, without naming any of them.
#[test]
fn a_goal_joins_an_existing_rule_without_disturbing_the_file() {
    for (name, text) in budget_fixtures() {
        let doc = PeriodicDoc::parse(&text);
        let parsed = rules(&text, &name);
        for block in doc.blocks().iter().filter(|block| block.lock.is_none()) {
            let period = block.period.expect("an unlocked rule has a period");
            let request = GoalRequest::AddBlock {
                period,
                description: block.description.clone(),
                account: "expenses:newcategory".into(),
                amount: usd(Dec::new(2500, 2)),
            };
            // Which rule this SHOULD join, restated independently of the engine:
            // the first, in file order, that states the same recurrence under the
            // same name and that we are willing to rewrite.
            let target = doc
                .blocks()
                .iter()
                .position(|other| {
                    other.lock.is_none()
                        && other.period == Some(period)
                        && collapsed(&other.description) == collapsed(&block.description)
                })
                .expect("a block matches itself");

            let plan = plan(&doc, &parsed, &request).expect("plan");
            let after = doc.apply(&plan).expect("apply");
            doc.verify(&plan, &after).expect("verify");
            assert_eq!(
                without_goal(&after, "expenses:newcategory"),
                text,
                "{name}: rule {} changed more than the line it gained",
                block.index
            );

            let now = reparses(&after, &name);
            assert_eq!(
                now.len(),
                parsed.len(),
                "{name}: joining a rule must not open another one"
            );
            for (at, (before, after)) in parsed.iter().zip(&now).enumerate() {
                let expected = before.postings.len() + usize::from(at == target);
                assert_eq!(
                    after.postings.len(),
                    expected,
                    "{name}: rule {at} gained or lost postings it should not have"
                );
                assert_eq!(
                    (before.period, &before.description),
                    (after.period, &after.description),
                    "{name}: rule {at} was renamed"
                );
            }
            let added = now[target]
                .postings
                .last()
                .expect("the joined rule has postings");
            assert_eq!(
                added.account.0, "expenses:newcategory",
                "{name}: the new goal is not last in its rule"
            );
            assert_eq!(added.amounts[0].quantity, Dec::new(2500, 2));
        }
    }
}

/// A description as the engine compares one: trimmed, and every run of
/// whitespace a single space. Restated here rather than imported, so the test
/// says what it expects instead of agreeing with itself.
fn collapsed(description: &str) -> String {
    description.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `after` with `account`'s goal line — terminator and all — spliced back out.
fn without_goal(after: &str, account: &str) -> String {
    let doc = PeriodicDoc::parse(after);
    let line = doc
        .lines()
        .iter()
        .find(|line| line.account == account)
        .unwrap_or_else(|| panic!("{account} was not written:\n{after}"));
    format!("{}{}", &after[..line.full.start], &after[line.full.end..])
}

/// The scan and the parser must agree about which `~` block is which, in every
/// fixture. A drift here would land an edit on a different rule than the one the
/// user was looking at, and no later check would notice.
#[test]
fn every_fixtures_blocks_line_up_with_its_parsed_rules() {
    for (name, text) in budget_fixtures() {
        let doc = PeriodicDoc::parse(&text);
        let parsed = rules(&text, &name);
        assert_eq!(
            doc.blocks().len(),
            parsed.len(),
            "{name}: the scan and the parse disagree about how many rules there are"
        );
        for (block, rule) in doc.blocks().iter().zip(&parsed) {
            assert_eq!(block.period, Some(rule.period), "{name}: rule period");
            assert_eq!(block.description, rule.description, "{name}: rule name");
            assert_eq!(
                block.lines.len(),
                rule.postings.len(),
                "{name}: rule {} posting count",
                block.index
            );
            for (at, posting) in rule.postings.iter().enumerate() {
                let line = &doc.lines()[block.lines[at]];
                assert_eq!(line.account, posting.account.0, "{name}: account");
                assert_eq!(line.ptype, posting.ptype, "{name}: posting type");
            }
            // Every budget fixture uses the `(account)` idiom, so every block
            // should be unconstrained. If that ever stops being true the editor
            // still works — but the fixtures no longer cover the case they were
            // written to cover, which is worth failing over.
            assert_eq!(
                block.balance,
                BlockBalance::Free,
                "{name}: rule {} is no longer the all-virtual idiom",
                block.index
            );
        }
    }
}
