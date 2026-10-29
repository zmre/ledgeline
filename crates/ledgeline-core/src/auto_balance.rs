//! Whether an `=` rule's postings can balance, judged from the rule alone.
//!
//! hledger 1.52 checks that every transaction balances AFTER `--auto` has
//! added a rule's postings, and refuses the whole journal when one does not
//! ("This transaction is unbalanced"). [`crate::auto::rule_findings`] reports
//! that per transaction once a rule has fired; [`unbalanced_group`] answers
//! the question a writer must ask BEFORE a rule is written, when it may match
//! nothing yet.
//!
//! Each balancing group — the regular postings, and the `[balanced virtual]`
//! ones — must balance on its own; `(virtual)` postings never need to. A group
//! balances by construction when it is empty, holds an amountless posting
//! (hledger infers it), its `*N` multipliers sum to zero, or its fixed amounts
//! leave zero or two commodities with a residual — two is hledger's implied
//! conversion. It can never balance when its multipliers alone sum to
//! anything else, or its fixed amounts alone leave one commodity, or three or
//! more. A mix of multipliers and fixed amounts, a re-denominating `*N CUR`
//! or a fixed amount with a cost balances or not depending on the posting it
//! matches, so it is left to the per-transaction check. Verified against
//! hledger 1.52 (2026-10-28) for every shape in the tests below.

use std::collections::BTreeMap;

use crate::decimal::Dec;
use crate::model::{AutoAmount, AutoPostingRule, Commodity, PostingType};

/// The groups that must each balance; `(virtual)` postings are in neither.
const BALANCED_GROUPS: [PostingType; 2] = [PostingType::Regular, PostingType::BalancedVirtual];

/// The first balancing group of `rule` whose postings cannot balance whatever
/// the rule matches — what hledger refuses the journal for as soon as the
/// rule fires — or `None` when every group balances, or may.
#[must_use]
pub fn unbalanced_group(rule: &AutoPostingRule) -> Option<PostingType> {
    BALANCED_GROUPS.into_iter().find(|group| {
        let amounts: Vec<&AutoAmount> = rule
            .postings
            .iter()
            .filter(|posting| posting.ptype == *group)
            .map(|posting| &posting.amount)
            .collect();
        never_balances(&amounts)
    })
}

/// Whether one group's postings are certain to leave a residual.
fn never_balances(amounts: &[&AutoAmount]) -> bool {
    if amounts
        .iter()
        .any(|amount| matches!(amount, AutoAmount::Inferred))
    {
        return false;
    }
    let factors: Option<Vec<Dec>> = amounts
        .iter()
        .map(|amount| match amount {
            AutoAmount::Scale {
                factor,
                commodity: None,
            } => Some(*factor),
            _ => None,
        })
        .collect();
    if let Some(factors) = factors {
        return total(factors).is_some_and(|sum| !sum.is_zero());
    }
    let fixed: Option<Vec<(&Commodity, Dec)>> = amounts
        .iter()
        .map(|amount| match amount {
            AutoAmount::Fixed(amount) if amount.cost.is_none() => {
                Some((&amount.commodity, amount.quantity))
            }
            _ => None,
        })
        .collect();
    fixed.is_some_and(|fixed| {
        let mut by_commodity: BTreeMap<&Commodity, Vec<Dec>> = BTreeMap::new();
        for (commodity, quantity) in fixed {
            by_commodity.entry(commodity).or_default().push(quantity);
        }
        let residuals: Option<Vec<Dec>> = by_commodity.into_values().map(total).collect();
        residuals.is_some_and(|residuals| {
            let open = residuals.iter().filter(|sum| !sum.is_zero()).count();
            open == 1 || open > 2
        })
    })
}

/// The exact sum, or `None` on overflow (undecided, so never refused). Seeded
/// with the first value rather than a zero at scale 0, which `Dec::add` would
/// rescale — see `parse::group_residual`.
fn total(values: Vec<Dec>) -> Option<Dec> {
    let mut values = values.into_iter();
    match values.next() {
        None => Some(Dec::zero()),
        Some(first) => values.try_fold(first, |sum, value| sum.add(value).ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group_of(block: &str) -> Option<PostingType> {
        let text = format!("{block}\n");
        let journal = crate::parse::parse_journal(&text, "rule.journal").expect("parses");
        let [rule] = journal.auto_rules.as_slice() else {
            panic!("one rule in {block}");
        };
        unbalanced_group(rule)
    }

    #[test]
    fn a_lone_percentage_posting_can_never_balance() {
        assert_eq!(
            group_of("= revenue:sales\n    liabilities:tax  *0.1"),
            Some(PostingType::Regular)
        );
    }

    #[test]
    fn a_lone_fixed_posting_can_never_balance() {
        assert_eq!(
            group_of("= revenue:sales\n    a  $5"),
            Some(PostingType::Regular)
        );
    }

    #[test]
    fn a_lone_balanced_virtual_posting_can_never_balance() {
        assert_eq!(
            group_of("= revenue:sales\n    [budget:food]  *-1"),
            Some(PostingType::BalancedVirtual)
        );
    }

    #[test]
    fn a_regular_leg_is_judged_apart_from_a_balanced_virtual_pair() {
        assert_eq!(
            group_of("= revenue:sales\n    [a]  *0.1\n    [b]\n    c  *1"),
            Some(PostingType::Regular)
        );
    }

    #[test]
    fn unbalanced_virtual_postings_need_no_balance() {
        assert_eq!(group_of("= revenue:sales\n    (budget:food)  *-1"), None);
    }

    #[test]
    fn legs_that_cancel_or_an_amountless_leg_balance() {
        for block in [
            "= revenue:sales\n    a  *0.1\n    b  *-0.1",
            "= revenue:sales\n    a  $5\n    b  $-5",
            "= revenue:sales\n    liabilities:tax  *0.1\n    assets:checking",
            "= revenue:sales\n    [budget:food]  *-1\n    [budget:x]  *1",
        ] {
            assert_eq!(group_of(block), None, "{block}");
        }
    }

    #[test]
    fn two_commodities_left_over_are_hledger_s_implied_conversion() {
        assert_eq!(
            group_of("= revenue:sales\n    a  10 AAA\n    b  $-50"),
            None
        );
    }

    #[test]
    fn a_shape_that_depends_on_the_matched_amount_is_left_to_the_transaction_check() {
        for block in [
            "= revenue:sales\n    a  *0.1\n    b  $-5",
            "= revenue:sales\n    a  *0.1 EUR\n    b  *-0.1",
        ] {
            assert_eq!(group_of(block), None, "{block}");
        }
    }
}
