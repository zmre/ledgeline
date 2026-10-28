//! Applying `= QUERY` auto-posting rules to a transaction, as
//! `hledger print --auto` does.
//!
//! # hledger 1.52's behaviour, which this reproduces
//!
//! Verified by scratch journals (`plans/30-auto-postings-and-period-grammar.md`):
//!
//! - Rules apply in file order. Each one walks the transaction's postings AS
//!   THE EARLIER RULES LEFT THEM, so a later rule matches an earlier rule's
//!   generated postings; a rule never matches its own.
//! - Wherever the query matches a posting, the rule's postings are inserted
//!   immediately below it. A transaction-level term (`desc:`) matches every
//!   posting, so the rule fires once per posting.
//! - `*N` multiplies the matched amount exactly — `$333.33 * 0.1` is
//!   `$33.333` — and a TOTAL cost (`@@`) with it; a unit cost is unchanged.
//!   `*N CUR` also re-denominates. A bare `N` is written as is, with no
//!   commodity — the manual says it takes the matched one, the binary does
//!   not (verified), and this follows the binary.
//! - One amountless posting balances the rule's other generated postings of
//!   its own group: the regular ones, or the balanced-virtual ones. An
//!   amountless `(virtual)` posting has nothing to balance and is zero.
//! - `%account` in an account name is the matched posting's account.
//! - A generated posting carries the matched posting's dates and the tags
//!   `generated-posting` / `_generated-posting` (value `= QUERY`); a
//!   transaction a rule touched carries `modified-transaction` /
//!   `_modified-transaction`.
//!
//! A rule whose query is unsupported never fires ([`crate::query`]).

use crate::decimal::{Dec, DecError};
use crate::model::{
    AccountName, Amount, AutoAmount, AutoPosting, AutoPostingRule, CostKind, Posting, PostingType,
    Transaction,
};
use std::collections::BTreeMap;

/// The tag a generated posting carries, valued `= QUERY` (hledger's own).
pub const GENERATED_POSTING_TAG: &str = "generated-posting";
/// Its hidden twin.
pub const HIDDEN_GENERATED_POSTING_TAG: &str = "_generated-posting";
/// The tag a transaction a rule changed carries.
pub const MODIFIED_TRANSACTION_TAG: &str = "modified-transaction";
/// Its hidden twin.
pub const HIDDEN_MODIFIED_TRANSACTION_TAG: &str = "_modified-transaction";

/// `txn` with every rule in `rules` applied, in order. Unchanged (and not
/// tagged) when no rule matched.
///
/// # Errors
/// [`DecError`] when a multiplication or an inferred sum overflows.
pub fn apply(rules: &[AutoPostingRule], txn: &Transaction) -> Result<Transaction, DecError> {
    apply_counted(rules, txn).map(|(txn, _)| txn)
}

/// [`apply`], and how many times each rule fired on `txn` (one count per rule,
/// in order) — what [`hledger_refuses`] needs.
///
/// # Errors
/// As [`apply`].
pub fn apply_counted(
    rules: &[AutoPostingRule],
    txn: &Transaction,
) -> Result<(Transaction, Vec<usize>), DecError> {
    let mut postings = txn.postings.clone();
    let mut fired = vec![0; rules.len()];
    for (index, rule) in rules.iter().enumerate() {
        if rule.query.unsupported().is_some() {
            continue;
        }
        let mut out = Vec::with_capacity(postings.len());
        for posting in &postings {
            out.push(posting.clone());
            if rule.query.matches(txn, posting, &[]) {
                out.extend(generate(rule, posting)?);
                fired[index] += 1;
            }
        }
        postings = out;
    }
    if fired.iter().all(|count| *count == 0) {
        return Ok((txn.clone(), fired));
    }
    let mut tags = txn.tags.clone();
    for name in [MODIFIED_TRANSACTION_TAG, HIDDEN_MODIFIED_TRANSACTION_TAG] {
        if !tags.iter().any(|(tag, _)| tag == name) {
            tags.push((name.to_string(), String::new()));
        }
    }
    Ok((
        Transaction {
            postings,
            tags,
            ..txn.clone()
        },
        fired,
    ))
}

/// Whether hledger 1.52 refuses a transaction on which `rule` fired `fired`
/// times. hledger infers an amountless auto posting at the TRANSACTION level,
/// so a rule with an amountless regular (or balanced-virtual) posting that
/// fires twice leaves two amountless postings: "there can't be more than one
/// real posting with no amount" (verified 2026-10-28). Ledgeline balances each
/// firing on its own — the reading that `*-N` on that posting spells out, and
/// so the fix to suggest.
#[must_use]
pub fn hledger_refuses(rule: &AutoPostingRule, fired: usize) -> bool {
    fired > 1
        && rule.postings.iter().any(|posting| {
            posting.amount == AutoAmount::Inferred && posting.ptype != PostingType::Virtual
        })
}

/// The postings `rule` adds below `matched`.
fn generate(rule: &AutoPostingRule, matched: &Posting) -> Result<Vec<Posting>, DecError> {
    let marker = format!("= {}", rule.query.raw());
    let mut generated = rule
        .postings
        .iter()
        .map(|auto| {
            let mut tags = auto.tags.clone();
            tags.push((GENERATED_POSTING_TAG.to_string(), marker.clone()));
            tags.push((HIDDEN_GENERATED_POSTING_TAG.to_string(), marker.clone()));
            Ok(Posting {
                status: auto.status,
                ptype: auto.ptype,
                account: AccountName(auto.account.replace("%account", &matched.account.0)),
                amounts: amounts(auto, matched)?,
                balance_assertion: None,
                date: matched.date.clone(),
                date2: matched.date2.clone(),
                comment: auto.comment.clone(),
                tags,
            })
        })
        .collect::<Result<Vec<_>, DecError>>()?;
    // Fill each amountless posting from its balance group.
    for (at, auto) in rule.postings.iter().enumerate() {
        if auto.amount != AutoAmount::Inferred {
            continue;
        }
        let group = |ptype: PostingType| match auto.ptype {
            PostingType::Virtual => false,
            own => ptype == own,
        };
        let mut sums: BTreeMap<String, (Amount, Dec)> = BTreeMap::new();
        for (other, posting) in generated.iter().enumerate() {
            if other == at
                || !group(posting.ptype)
                || rule.postings[other].amount == AutoAmount::Inferred
            {
                continue;
            }
            for amount in &posting.amounts {
                let entry = sums
                    .entry(amount.commodity.0.clone())
                    .or_insert_with(|| (amount.clone(), Dec::new(0, 0)));
                entry.1 = entry.1.add(amount.quantity)?;
            }
        }
        let balancing = sums
            .into_values()
            .map(|(template, sum)| {
                Ok(Amount {
                    quantity: sum.neg()?,
                    cost: None,
                    ..template
                })
            })
            .collect::<Result<Vec<_>, DecError>>()?;
        generated[at].amounts = if balancing.is_empty() {
            zero_like(matched)
        } else {
            balancing
        };
    }
    Ok(generated)
}

/// What one rule posting writes for `matched`, before inference.
fn amounts(auto: &AutoPosting, matched: &Posting) -> Result<Vec<Amount>, DecError> {
    match &auto.amount {
        AutoAmount::Fixed(amount) => Ok(vec![amount.clone()]),
        AutoAmount::Scale { factor, commodity } => matched
            .amounts
            .iter()
            .map(|amount| {
                let quantity = amount.quantity.mul(*factor)?;
                Ok(match commodity {
                    Some(template) => Amount {
                        quantity,
                        cost: None,
                        ..template.clone()
                    },
                    None => Amount {
                        quantity,
                        cost: amount
                            .cost
                            .as_ref()
                            .map(|cost| -> Result<_, DecError> {
                                let mut cost = cost.as_ref().clone();
                                if cost.kind == CostKind::Total {
                                    cost.amount.quantity = cost.amount.quantity.mul(*factor)?;
                                }
                                Ok(Box::new(cost))
                            })
                            .transpose()?,
                        ..amount.clone()
                    },
                })
            })
            .collect(),
        AutoAmount::Inferred => Ok(Vec::new()),
    }
}

/// A zero in `matched`'s first commodity — what an amountless posting with
/// nothing to balance reads as (`$0`, as hledger prints it).
fn zero_like(matched: &Posting) -> Vec<Amount> {
    matched
        .amounts
        .first()
        .map(|like| Amount {
            quantity: Dec::new(0, 0),
            cost: None,
            ..like.clone()
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    /// `(account, amount as "commodity mantissa/places")` per posting of the
    /// first transaction of `journal` once its rules are applied.
    fn applied(journal: &str) -> Vec<(String, String)> {
        let journal = parse_journal(journal, "a.journal").expect("parses");
        let txn = apply(&journal.auto_rules, &journal.transactions[0]).expect("applies");
        txn.postings
            .iter()
            .map(|posting| {
                let amounts: Vec<String> = posting
                    .amounts
                    .iter()
                    .map(|amount| format!("{}{}", amount.commodity.0, show(amount.quantity)))
                    .collect();
                (posting.account.0.clone(), amounts.join("+"))
            })
            .collect()
    }

    /// `-33.333`, `1200` — a decimal as written, trailing zeros dropped.
    fn show(quantity: Dec) -> String {
        let q = quantity.normalized();
        let digits = q.mantissa.unsigned_abs().to_string();
        let places = q.places as usize;
        let sign = if q.mantissa < 0 { "-" } else { "" };
        if places == 0 {
            return format!("{sign}{digits}");
        }
        let padded = format!("{digits:0>width$}", width = places + 1);
        let (whole, fraction) = padded.split_at(padded.len() - places);
        format!("{sign}{whole}.{fraction}")
    }

    fn rows(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect()
    }

    /// The user's own rules (2026-10-28), on a forecast-shaped transaction:
    /// `hledger print --auto` inserts the pair below the matched posting, the
    /// amountless leg balancing the multiplied one.
    #[test]
    fn a_commission_rule_inserts_its_pair_below_the_sale_as_hledger_does() {
        let got = applied(
            "= revenue:sales tag:_generated-transaction\n\
             \x20   expenses:people-costs:sales-commission:residual  *-0.03\n\
             \x20   assets:mercury:checking\n\
             \n\
             2027-04-01 Lucid Software  ; _generated-transaction: ~ every 1st Apr\n\
             \x20   revenue:sales            $-40000\n\
             \x20   assets:mercury:checking\n",
        );
        assert_eq!(
            got,
            rows(&[
                ("revenue:sales", "$-40000"),
                ("expenses:people-costs:sales-commission:residual", "$1200"),
                ("assets:mercury:checking", "$-1200"),
                ("assets:mercury:checking", "$40000"),
            ])
        );
    }

    /// The forecast-only tag keeps a rule off a recorded transaction.
    #[test]
    fn a_forecast_only_rule_leaves_a_recorded_transaction_alone() {
        let journal = parse_journal(
            "= revenue:sales tag:_generated-transaction\n    (x)  *0.1\n\n\
             2026-10-01 real\n    revenue:sales  $-100\n    assets:checking\n",
            "r.journal",
        )
        .expect("parses");
        let txn = &journal.transactions[0];
        assert_eq!(&apply(&journal.auto_rules, txn).expect("applies"), txn);
    }

    /// `hledger print --auto` over this exact journal (2026-10-27): rules in
    /// order, the later one matching the earlier one's output, multipliers
    /// exact, a commodity replaced, an amountless regular leg balancing its
    /// regular group and an amountless balanced-virtual one its own.
    #[test]
    fn chained_rules_multipliers_and_groups_match_hledger() {
        let got = applied(
            "= revenue:sales\n\
             \x20   (tax)  *0.1 USD\n\
             \x20   [budget:a]   $10\n\
             \x20   [budget:b]\n\
             \n\
             = acct:^tax$\n\
             \x20   (seen)  *1\n\
             \n\
             2026-10-01 sale\n\
             \x20   revenue:sales     $-333.33\n\
             \x20   assets:checking\n",
        );
        assert_eq!(
            got,
            rows(&[
                ("revenue:sales", "$-333.33"),
                ("tax", "USD-33.333"),
                ("seen", "USD-33.333"),
                ("budget:a", "$10"),
                ("budget:b", "$-10"),
                ("assets:checking", "$333.33"),
            ])
        );
    }

    /// `%account`, a bare number staying commodity-less, and a rule firing
    /// once per posting on a transaction-level query — `hledger print --auto`
    /// over this journal (2026-10-28).
    #[test]
    fn percent_account_bare_numbers_and_per_posting_firing() {
        let got = applied(
            "= desc:fee\n    (%account:fee)  2\n\n\
             2026-01-01 fee\n    expenses:bank  €5\n    assets:checking\n",
        );
        assert_eq!(
            got,
            rows(&[
                ("expenses:bank", "€5"),
                ("expenses:bank:fee", "2"),
                ("assets:checking", "€-5"),
                ("assets:checking:fee", "2"),
            ])
        );
    }

    /// Generated postings and the touched transaction carry hledger's tags.
    #[test]
    fn generated_postings_and_their_transaction_are_tagged() {
        let journal = parse_journal(
            "= food\n    (budget:food)  *-1\n\n2026-01-01 x\n    expenses:food  $5\n    assets:cash\n",
            "t.journal",
        )
        .expect("parses");
        let txn = apply(&journal.auto_rules, &journal.transactions[0]).expect("applies");
        let generated = &txn.postings[1];
        assert!(
            generated
                .tags
                .contains(&("generated-posting".to_string(), "= food".to_string()))
        );
        assert!(
            generated
                .tags
                .contains(&("_generated-posting".to_string(), "= food".to_string()))
        );
        assert!(
            txn.tags
                .iter()
                .any(|(tag, _)| tag == "modified-transaction")
        );
        assert!(
            !txn.postings[0]
                .tags
                .iter()
                .any(|(tag, _)| tag == "generated-posting")
        );
    }

    /// A rule this engine cannot evaluate is carried but never fires.
    #[test]
    fn an_unsupported_rule_never_fires() {
        let journal = parse_journal(
            "= expr:'food and not lunch'\n    (x)  1\n\n2026-01-01 x\n    expenses:food  $5\n    assets:cash\n",
            "u.journal",
        )
        .expect("parses");
        assert_eq!(journal.auto_rules.len(), 1);
        assert!(journal.auto_rules[0].query.unsupported().is_some());
        let txn = &journal.transactions[0];
        assert_eq!(&apply(&journal.auto_rules, txn).expect("applies"), txn);
    }

    /// The user's payroll-tax rule over per-person CHILD accounts, all in one
    /// transaction: each child's posting gets its own 10% and its own
    /// balancing leg. Those are the figures `hledger print --auto` prints for
    /// this journal once the leg is spelled `*-0.1` (2026-10-28) — with the
    /// leg left amountless hledger refuses the transaction instead, which is
    /// what `hledger_refuses` reports.
    #[test]
    fn a_parent_account_rule_taxes_each_child_account_on_its_own() {
        let payroll = "\n2027-01-01 payroll  ; _generated-transaction: ~ monthly\n\
             \x20   expenses:people-costs:wages-salaries:george   $6000\n\
             \x20   expenses:people-costs:wages-salaries:john     $5000\n\
             \x20   assets:mercury:checking\n";
        let want = rows(&[
            ("expenses:people-costs:wages-salaries:george", "$6000"),
            ("expenses:people-costs:payroll-tax", "$600"),
            ("assets:mercury:checking", "$-600"),
            ("expenses:people-costs:wages-salaries:john", "$5000"),
            ("expenses:people-costs:payroll-tax", "$500"),
            ("assets:mercury:checking", "$-500"),
            ("assets:mercury:checking", "$-11000"),
        ]);
        for leg in ["assets:mercury:checking", "assets:mercury:checking  *-0.1"] {
            let rule = format!(
                "= expenses:people-costs:wages-salaries tag:_generated-transaction\n    \
                 expenses:people-costs:payroll-tax  *0.1\n    {leg}\n"
            );
            let text = format!("{rule}{payroll}");
            assert_eq!(applied(&text), want, "{leg}");
            let journal = parse_journal(&text, "p.journal").expect("parses");
            let (_, fired) =
                apply_counted(&journal.auto_rules, &journal.transactions[0]).expect("applies");
            assert_eq!(fired, [2], "{leg}");
            assert_eq!(
                hledger_refuses(&journal.auto_rules[0], fired[0]),
                !leg.contains('*'),
                "hledger refuses only the amountless spelling: {leg}"
            );
        }
    }
}
