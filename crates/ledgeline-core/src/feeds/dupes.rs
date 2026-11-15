//! Which normalized rows the journal already has (plan 43 §3.4, layers 2
//! and 4).
//!
//! - **Already:** the row's id is on some transaction. Silent, always: if the
//!   user edited that transaction since, their edit wins over a re-download,
//!   and sync reports no conflict.
//! - **Held:** no transaction has the id, but one on the same journal account
//!   looks like the same money moving — same amount, within three days,
//!   sharing a payee word — and carries no id from this fetch. That is what
//!   Capital One re-issuing its ids looks like, and what an OFX file imported
//!   by hand for a synced account looks like. Held rows are **never written**;
//!   the user picks "Import anyway" or "It's a duplicate".
//! - **Fresh:** everything else, for the import.

use super::normalize::NormRow;
use crate::Dec;
use crate::edit::render_amount;
use crate::model::{Journal, Posting, Transaction};
use crate::reimport::{ID_TAG, ids_of};
use crate::reports::periods::days_between;
use crate::reports::subscriptions::payee_of;
use std::collections::HashSet;

/// How far apart, in days, a held row and the transaction it resembles may be.
pub const WINDOW_DAYS: i64 = 3;

/// The shortest payee word that counts as shared. Two letters match far too
/// much ("of", "co").
pub const MIN_TOKEN_CHARS: usize = 3;

/// Every `id:` tag value on every transaction in `journal`.
#[must_use]
pub fn known_ids(journal: &Journal) -> HashSet<String> {
    journal
        .transactions
        .iter()
        .flat_map(|txn| ids_of(txn, ID_TAG))
        .map(str::to_string)
        .collect()
}

/// The transaction a held row resembles.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Existing {
    /// Its 1-based file-order index ([`Tindex`](crate::model::Tindex)), what
    /// the transaction editor addresses it by.
    pub index: usize,
    /// Its date, ISO.
    pub date: String,
    /// Its description, as written.
    pub description: String,
    /// The matching posting's amount, rendered as in the journal (`$-4.50`).
    pub amount: String,
}

/// A row that waits for the user.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Held {
    /// The row from the feed.
    pub row: NormRow,
    /// What it looks like a duplicate of.
    pub existing: Existing,
}

/// [`split`]'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    /// Ids of rows the journal already holds.
    pub already: Vec<String>,
    /// Rows to import, in input order.
    pub fresh: Vec<NormRow>,
    /// Possible duplicates, in input order. Never written.
    pub held: Vec<Held>,
}

/// A transaction a row could be held against, prepared once.
struct Candidate<'a> {
    txn: &'a Transaction,
    /// The value and rendering of each single-amount posting to the account.
    amounts: Vec<(Dec, String)>,
    tokens: HashSet<String>,
}

/// Split `rows` into already-held ids, fresh rows and held rows.
///
/// `known` is [`known_ids`] of `journal` (passed in so one build serves every
/// linked account). `fetched_ids` is
/// [`normalize::fetched_ids`](super::normalize::fetched_ids): every prefixed
/// id in this fetch before filtering, so a transaction an earlier sync wrote
/// from *this* data is never mistaken for a different one.
///
/// A row is held against an existing transaction when **all** of these hold:
///
/// - it has a posting to exactly `journal_account` carrying one amount equal
///   to the row's (by decimal value, so `-4.5` equals `-4.50`; any one
///   commodity, since a feed's `USD` is the journal's `$`);
/// - its date is within [`WINDOW_DAYS`] of the row's;
/// - none of its ids is in `fetched_ids`;
/// - its payee ([`payee_of`] its description) shares a word of at least
///   [`MIN_TOKEN_CHARS`] letters with the row's payee or description.
///
/// Each existing transaction is claimed by at most one row; a row takes the
/// nearest-dated unclaimed match, ties going to the earlier in the file.
#[must_use]
pub fn split(
    rows: Vec<NormRow>,
    journal: &Journal,
    journal_account: &str,
    known: &HashSet<String>,
    fetched_ids: &HashSet<String>,
) -> Split {
    let candidates: Vec<Candidate> = journal
        .transactions
        .iter()
        .filter(|txn| {
            ids_of(txn, ID_TAG)
                .iter()
                .all(|id| !fetched_ids.contains(*id))
        })
        .filter_map(|txn| {
            let amounts: Vec<(Dec, String)> = txn
                .postings
                .iter()
                .filter(|posting| posting.account.0 == journal_account)
                .filter_map(single_amount)
                .collect();
            (!amounts.is_empty()).then(|| Candidate {
                txn,
                amounts,
                tokens: tokens(payee_of(&txn.description)),
            })
        })
        .collect();

    let mut claimed = vec![false; candidates.len()];
    let mut out = Split::default();
    for row in rows {
        if known.contains(&row.id) {
            out.already.push(row.id);
            continue;
        }
        match best_match(&row, &candidates, &claimed) {
            Some((at, existing)) => {
                claimed[at] = true;
                out.held.push(Held { row, existing });
            }
            None => out.fresh.push(row),
        }
    }
    out
}

/// The nearest unclaimed candidate `row` resembles, and its position.
fn best_match(
    row: &NormRow,
    candidates: &[Candidate],
    claimed: &[bool],
) -> Option<(usize, Existing)> {
    let amount = Dec::parse(&row.amount, '.').ok()?;
    let row_tokens: HashSet<String> = tokens(&row.payee)
        .into_iter()
        .chain(tokens(&row.description))
        .collect();
    candidates
        .iter()
        .enumerate()
        .filter(|(at, _)| !claimed[*at])
        .filter_map(|(at, candidate)| {
            let gap = days_between(&candidate.txn.date, &row.date).abs();
            if gap > WINDOW_DAYS || candidate.tokens.is_disjoint(&row_tokens) {
                return None;
            }
            let (_, rendered) = candidate.amounts.iter().find(|(q, _)| *q == amount)?;
            Some((gap, at, rendered))
        })
        .min_by_key(|(gap, at, _)| (*gap, *at))
        .map(|(_, at, rendered)| {
            let txn = candidates[at].txn;
            (
                at,
                Existing {
                    index: txn.index.0 as usize,
                    date: txn.date.clone(),
                    description: txn.description.clone(),
                    amount: rendered.clone(),
                },
            )
        })
}

/// A posting's amount when it has exactly one, ignoring auto-posting output.
fn single_amount(posting: &Posting) -> Option<(Dec, String)> {
    match (posting.amounts.as_slice(), &posting.generated) {
        ([amount], None) => Some((amount.quantity, render_amount(amount))),
        _ => None,
    }
}

/// The words of `text` for payee matching: lowercased, split on anything
/// that isn't a letter (so digits never form a word), at least
/// [`MIN_TOKEN_CHARS`] letters long.
fn tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|word| word.chars().count() >= MIN_TOKEN_CHARS)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    const ACCOUNT: &str = "assets:bank:checking";

    fn journal(text: &str) -> Journal {
        parse_journal(text, "journal").expect("fixture parses")
    }

    fn row(id: &str, date: &str, amount: &str, payee: &str) -> NormRow {
        NormRow {
            date: date.into(),
            amount: amount.into(),
            payee: payee.into(),
            description: payee.into(),
            memo: String::new(),
            id: id.into(),
            transacted: String::new(),
        }
    }

    fn split_one(text: &str, rows: Vec<NormRow>, fetched: &[&str]) -> Split {
        let journal = journal(text);
        let known = known_ids(&journal);
        let fetched: HashSet<String> = fetched.iter().map(|s| s.to_string()).collect();
        split(rows, &journal, ACCOUNT, &known, &fetched)
    }

    const COFFEE: &str = "\
2026-11-10 Blue Bottle Coffee  ; id:FIT0001
    assets:bank:checking   $-4.50
    expenses:coffee
";

    #[test]
    fn tokens_are_letters_only_and_long_enough() {
        let words = tokens("POS 1234 BLUE-BOTTLE #42 co Café");
        assert_eq!(
            words,
            HashSet::from(["pos", "blue", "bottle", "café"].map(String::from))
        );
    }

    #[test]
    fn known_ids_cover_every_id_tag() {
        let journal = journal(
            "2026-01-01 A  ; id:X, id:Y\n    a  1\n    b\n\n2026-01-02 B  ; id:Z\n    a  1\n    b\n",
        );
        assert_eq!(
            known_ids(&journal),
            HashSet::from(["X", "Y", "Z"].map(String::from))
        );
    }

    #[test]
    fn a_known_id_is_already_even_when_everything_else_changed() {
        let out = split_one(
            "2026-11-10 Edited by hand  ; id:s-1\n    assets:bank:checking  $-99\n    expenses:x\n",
            vec![row("s-1", "2026-11-10", "-4.50", "Blue Bottle")],
            &["s-1"],
        );
        assert_eq!(out.already, vec!["s-1"]);
        assert!(out.fresh.is_empty() && out.held.is_empty());
    }

    #[test]
    fn a_lookalike_under_another_id_is_held() {
        let out = split_one(
            COFFEE,
            vec![row("s-NEW", "2026-11-12", "-4.5", "BLUE BOTTLE #42")],
            &["s-NEW"],
        );
        assert_eq!(out.held.len(), 1);
        let held = &out.held[0];
        assert_eq!(held.row.id, "s-NEW");
        assert_eq!(
            held.existing,
            Existing {
                index: 1,
                date: "2026-11-10".into(),
                description: "Blue Bottle Coffee".into(),
                amount: "$-4.50".into(),
            }
        );
        assert!(out.fresh.is_empty());
    }

    #[test]
    fn the_window_is_three_days_either_side() {
        for (date, held) in [
            ("2026-11-07", true),
            ("2026-11-13", true),
            ("2026-11-06", false),
            ("2026-11-14", false),
        ] {
            let out = split_one(COFFEE, vec![row("s-1", date, "-4.50", "Blue Bottle")], &[]);
            assert_eq!(out.held.len() == 1, held, "{date}");
            assert_eq!(out.fresh.len() == 1, !held, "{date}");
        }
    }

    #[test]
    fn amount_must_be_equal() {
        let out = split_one(
            COFFEE,
            vec![
                row("s-1", "2026-11-10", "-4.51", "Blue Bottle"),
                row("s-2", "2026-11-10", "4.50", "Blue Bottle"),
            ],
            &[],
        );
        assert_eq!(out.fresh.len(), 2);
    }

    #[test]
    fn the_posting_must_be_to_exactly_the_account() {
        let journal_text = COFFEE.replace("assets:bank:checking", "assets:bank:checking:sub");
        let out = split_one(
            &journal_text,
            vec![row("s-1", "2026-11-10", "-4.50", "Blue Bottle")],
            &[],
        );
        assert_eq!(out.fresh.len(), 1);
    }

    #[test]
    fn payees_must_share_a_word_of_three_letters() {
        let out = split_one(
            COFFEE,
            vec![
                row("s-1", "2026-11-10", "-4.50", "BB #4242 co"),
                row("s-2", "2026-11-10", "-4.50", "Something Else"),
            ],
            &[],
        );
        assert_eq!(out.fresh.len(), 2);
        // The row's description counts as well as its payee.
        let by_description = NormRow {
            description: "POS BOTTLE 1234".into(),
            ..row("s-3", "2026-11-10", "-4.50", "Unrelated")
        };
        assert_eq!(split_one(COFFEE, vec![by_description], &[]).held.len(), 1);
    }

    #[test]
    fn only_the_payee_part_of_the_description_counts() {
        let journal_text = COFFEE.replace("Blue Bottle Coffee", "Shop | bottle deposit");
        let out = split_one(
            &journal_text,
            vec![row("s-1", "2026-11-10", "-4.50", "Bottle")],
            &[],
        );
        assert_eq!(out.fresh.len(), 1);
    }

    #[test]
    fn a_transaction_carrying_an_id_from_this_fetch_is_not_a_lookalike() {
        // The earlier sync wrote s-1 from this very data; s-2 is a second,
        // genuinely different coffee.
        let journal_text = COFFEE.replace("id:FIT0001", "id:s-1");
        let out = split_one(
            &journal_text,
            vec![row("s-2", "2026-11-10", "-4.50", "Blue Bottle")],
            &["s-1", "s-2"],
        );
        assert_eq!(out.fresh.len(), 1);
        // …while an id from outside this fetch doesn't protect it.
        let out = split_one(
            &journal_text,
            vec![row("s-2", "2026-11-10", "-4.50", "Blue Bottle")],
            &["s-2"],
        );
        assert_eq!(out.held.len(), 1);
    }

    #[test]
    fn untagged_history_can_be_a_lookalike() {
        let journal_text =
            "2026-11-10 Blue Bottle\n    assets:bank:checking  -4.50\n    expenses:coffee\n";
        let out = split_one(
            journal_text,
            vec![row("s-1", "2026-11-11", "-4.50", "Blue Bottle")],
            &["s-1"],
        );
        assert_eq!(out.held.len(), 1);
        assert_eq!(out.held[0].existing.amount, "-4.50");
    }

    #[test]
    fn one_existing_transaction_claims_at_most_one_row() {
        let out = split_one(
            COFFEE,
            vec![
                row("s-1", "2026-11-10", "-4.50", "Blue Bottle"),
                row("s-2", "2026-11-10", "-4.50", "Blue Bottle"),
            ],
            &[],
        );
        assert_eq!(out.held.len(), 1);
        assert_eq!(out.held[0].row.id, "s-1");
        assert_eq!(out.fresh.len(), 1);
        assert_eq!(out.fresh[0].id, "s-2");
    }

    #[test]
    fn the_nearest_date_wins() {
        let two = "\
2026-11-08 Blue Bottle  ; id:A
    assets:bank:checking   -4.50
    expenses:coffee

2026-11-11 Blue Bottle  ; id:B
    assets:bank:checking   -4.50
    expenses:coffee
";
        let out = split_one(
            two,
            vec![row("s-1", "2026-11-10", "-4.50", "Blue Bottle")],
            &[],
        );
        assert_eq!(out.held[0].existing.index, 2);
    }

    #[test]
    fn split_keeps_input_order() {
        let out = split_one(
            "",
            vec![
                row("s-2", "2026-11-12", "-1", "B"),
                row("s-1", "2026-11-10", "-1", "A"),
            ],
            &[],
        );
        assert_eq!(
            out.fresh.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["s-2", "s-1"]
        );
    }
}
