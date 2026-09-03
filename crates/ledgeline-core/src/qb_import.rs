//! Resolving QuickBooks account names against a journal's own plain aliases,
//! and choosing a transaction description, for the QuickBooks Journal write
//! pipeline (WP-17 Phase B; see `plans/17-quickbooks-journal-import.md`).
//!
//! # The narrow alias exception
//!
//! `docs/imports.md` states, deliberately: "Ledgeline reads aliases; it does
//! not apply them" — because reproducing hledger's regex alias dialect would be
//! a near-miss silent-wrong-answer generator. That policy is about *regex*
//! aliases. A **plain** (non-regex) alias applied to an **exact QuickBooks
//! account string** needs no regex engine at all: it's string equality, plus
//! hledger's own plain-alias rule that a plain alias also matches a prefix
//! ending at `:` ([`crate::hledger_conf::conf_argument`]'s module docs:
//! "verified: it rewrites `a` and `a:sub` and leaves `abc` alone"). This is the
//! one place in the codebase Ledgeline computes an aliased name itself rather
//! than forwarding to hledger.
//!
//! The prefix rule cascades on purpose: an alias on `1520 Computer & Office
//! Equipment` also rewrites `1520 Computer & Office Equipment:1521 Computer &
//! Equipment - Accum Depr`, preserving the `:1521 …` suffix on the new name.
//! Confirmed with the user, and it is the same behaviour real hledger gives the
//! same alias against the same account names — `docs/imports.md`'s "Column
//! interpolation composes with it" paragraph already relies on exactly this for
//! every other import. Ten of the real export's eighteen accounts carry a
//! colon, so this is the common case, not an edge case.
//!
//! A `/regex/` alias is never eligible: the account it might have matched is
//! simply reported unmapped (see [`unmapped_accounts`]), never guessed at.
//!
//! # Which aliases count
//!
//! [`plain_aliases`] starts from [`Journal::aliases_in_force`], so an alias an
//! `end aliases` line has closed is excluded exactly as it is from the aliases
//! the CSV import path forwards ([`crate::aliases::forward`]) — the user wrote
//! down where that mapping stops, and this is the same "in force" a new mapping
//! would need to be. Matched in file order, first match wins, exactly how
//! hledger composes several `--alias` options.

use crate::model::{AliasDirective, Journal};
use crate::qb_journal::QbTransaction;

/// The plain (non-regex), in-force aliases of `journal` — the set
/// [`resolve_account`] and [`unmapped_accounts`] are matched against.
#[must_use]
pub fn plain_aliases(journal: &Journal) -> Vec<&AliasDirective> {
    journal
        .aliases_in_force()
        .filter(|alias| !alias.regex)
        .collect()
}

/// Resolve one QuickBooks account name against `aliases`.
///
/// `None` when nothing matches — never a guess and never a default account.
/// Otherwise the hledger account name: `alias.replacement` on an exact match,
/// or `{replacement}:{rest}` when `account` is `{alias.pattern}:{rest}` for
/// some non-empty `rest` (the `:`-bounded prefix rule; see the module docs for
/// why it cascades on purpose).
#[must_use]
pub fn resolve_account(account: &str, aliases: &[&AliasDirective]) -> Option<String> {
    aliases.iter().find_map(|alias| {
        if account == alias.pattern {
            return Some(alias.replacement.clone());
        }
        account
            .strip_prefix(alias.pattern.as_str())
            .and_then(|rest| rest.strip_prefix(':'))
            .map(|rest| format!("{}:{rest}", alias.replacement))
    })
}

/// Every distinct QuickBooks account across `transactions` that `aliases` does
/// not map, in first-seen order.
///
/// This is what blocks the write (see the WP-17 plan's "any unmapped account
/// blocks the write and must be reported to the caller by name") — the list is
/// meant to be shown to a person and asked for aliases, never defaulted.
#[must_use]
pub fn unmapped_accounts(
    aliases: &[&AliasDirective],
    transactions: &[QbTransaction],
) -> Vec<String> {
    let mut unmapped: Vec<String> = Vec::new();
    for transaction in transactions {
        for posting in &transaction.postings {
            if resolve_account(&posting.account, aliases).is_none()
                && !unmapped.iter().any(|seen| seen == &posting.account)
            {
                unmapped.push(posting.account.clone());
            }
        }
    }
    unmapped
}

/// hledger's description for one QuickBooks transaction.
///
/// `name` (the payee) when the report carried a non-empty one — that is the
/// column QuickBooks itself calls the payee, and every generated-report row
/// repeats it, so it is preferred outright. Otherwise `transaction_type` is
/// used, widened with the first posting's memo when there is one: the report
/// only ever uses one of six transaction types (`Deposit`, `Expense`, `Journal
/// Entry`, `Transfer`, `Bill`, `Credit Card Expense`), so on its own it would
/// give every un-payee'd transaction of a kind the identical description — a
/// real ten-line manual Journal Entry has no `Name` at all and would otherwise
/// read as bare "Journal Entry" no matter which of the file's several journal
/// entries it was.
///
/// Run through [`journal_safe`] before it is returned — see that function's
/// own docs for why a raw payee name cannot always be written verbatim.
#[must_use]
pub fn description_for(transaction: &QbTransaction) -> String {
    let raw = if let Some(name) = transaction.name.as_deref().filter(|name| !name.is_empty()) {
        name.to_string()
    } else {
        match transaction
            .postings
            .first()
            .and_then(|posting| posting.memo.as_deref())
            .filter(|memo| !memo.is_empty())
        {
            Some(memo) => format!("{}: {memo}", transaction.transaction_type),
            None => transaction.transaction_type.clone(),
        }
    };
    journal_safe(&raw)
}

/// `text`, made safe to write literally into hledger's line-based grammar —
/// wherever it lands: a transaction's header (its description), or a
/// posting's `class:`/`customer:`/`vendor:` comment tag.
///
/// Found necessary against a real, much larger export: a payee name
/// containing a `;` (not unusual — a law firm's "Smith; Jones LLP"-style
/// name is exactly the shape a real vendor list turned up) reached
/// [`JournalEditor::add_transaction`](crate::edit::JournalEditor::add_transaction)'s
/// round-trip guard as `EditError::RoundTripMismatch`, correctly refusing —
/// but naming no transaction, because nothing that far downstream still
/// knows which one it was.
///
/// Two characters this format has no escape for, ever, so both are replaced
/// rather than the whole transaction refused over one punctuation mark most
/// people reviewing their books would not even notice changed:
///
/// - A newline would split one journal line into two, corrupting the
///   structure of every line below it. `parse.rs` has no continuation syntax
///   that could ever put it back together as one field.
/// - A `;` starts a comment with no way to write a literal one —
///   `parse::split_comment` is a plain `line.find(';')` — so the FIRST one
///   anywhere in a transaction's header line ends its description right
///   there and starts reading the rest as a tag comment instead, silently
///   changing what was written.
///
/// Safe to apply unconditionally, everywhere free text from the export is
/// about to be written: replacing a `;` that would have landed inside an
/// *already-started* comment (harmless there — `split_comment` only ever
/// looks for the first one) changes nothing a reader would notice either.
#[must_use]
pub fn journal_safe(text: &str) -> String {
    text.replace(['\n', '\r'], " ").replace(';', ",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;
    use crate::qb_journal::QbPosting;

    fn journal(text: &str) -> Journal {
        parse_journal(text, "t.journal").expect("the fixture parses")
    }

    fn posting(account: &str) -> QbPosting {
        QbPosting {
            account: account.to_string(),
            amount: crate::decimal::Dec::zero(),
            memo: None,
            class: None,
            customer: None,
            vendor: None,
        }
    }

    fn transaction(id: &str, postings: Vec<QbPosting>) -> QbTransaction {
        QbTransaction {
            id: id.to_string(),
            date: "2026-01-17".to_string(),
            transaction_type: "Journal Entry".to_string(),
            num: None,
            name: None,
            postings,
        }
    }

    #[test]
    fn an_exact_match_resolves() {
        let journal = journal("alias 1520 Computer & Office Equipment = assets:office\n");
        let aliases = plain_aliases(&journal);
        assert_eq!(
            resolve_account("1520 Computer & Office Equipment", &aliases).as_deref(),
            Some("assets:office")
        );
    }

    #[test]
    fn a_colon_bounded_prefix_cascades_and_preserves_the_suffix() {
        let journal = journal("alias 1520 Computer & Office Equipment = assets:office\n");
        let aliases = plain_aliases(&journal);
        assert_eq!(
            resolve_account(
                "1520 Computer & Office Equipment:1521 Computer & Equipment - Accum Depr",
                &aliases
            )
            .as_deref(),
            Some("assets:office:1521 Computer & Equipment - Accum Depr")
        );
    }

    #[test]
    fn a_prefix_not_bounded_by_a_colon_does_not_match() {
        // hledger's own rule: `a` rewrites `a` and `a:sub`, and leaves `abc` alone.
        let journal = journal("alias 1520 = assets:office\n");
        let aliases = plain_aliases(&journal);
        assert_eq!(
            resolve_account("1520 Computer & Office Equipment", &aliases),
            None
        );
    }

    #[test]
    fn a_regex_alias_is_never_eligible() {
        let journal = journal("alias /^1520.*/ = assets:office\n");
        let aliases = plain_aliases(&journal);
        assert!(aliases.is_empty(), "the regex alias must be filtered out");
        assert_eq!(
            resolve_account("1520 Computer & Office Equipment", &aliases),
            None
        );
    }

    #[test]
    fn an_alias_closed_by_end_aliases_is_excluded() {
        let journal = journal(
            "alias 1520 Computer & Office Equipment = assets:office\nend aliases\n2026-01-01 x\n    a  1\n    b  -1\n",
        );
        let aliases = plain_aliases(&journal);
        assert!(aliases.is_empty(), "the ended alias must not be in force");
        assert_eq!(
            resolve_account("1520 Computer & Office Equipment", &aliases),
            None
        );
    }

    #[test]
    fn the_first_matching_alias_in_file_order_wins() {
        let journal = journal(
            "alias 1520 Computer & Office Equipment = assets:first\n\
             alias 1520 Computer & Office Equipment = assets:second\n",
        );
        let aliases = plain_aliases(&journal);
        assert_eq!(
            resolve_account("1520 Computer & Office Equipment", &aliases).as_deref(),
            Some("assets:first")
        );
    }

    #[test]
    fn unmapped_accounts_are_reported_once_each_in_first_seen_order() {
        let journal = journal("alias Checking = assets:checking\n");
        let aliases = plain_aliases(&journal);
        let transactions = vec![
            transaction("1", vec![posting("Checking"), posting("Supplies")]),
            transaction("2", vec![posting("Supplies"), posting("Postage")]),
        ];
        assert_eq!(
            unmapped_accounts(&aliases, &transactions),
            vec!["Supplies".to_string(), "Postage".to_string()]
        );
    }

    #[test]
    fn mapping_every_account_leaves_nothing_unmapped() {
        let journal =
            journal("alias Checking = assets:checking\nalias Supplies = expenses:supplies\n");
        let aliases = plain_aliases(&journal);
        let transactions = vec![transaction(
            "1",
            vec![posting("Checking"), posting("Supplies")],
        )];
        assert_eq!(
            unmapped_accounts(&aliases, &transactions),
            Vec::<String>::new()
        );
    }

    #[test]
    fn description_prefers_the_payee_name() {
        let mut txn = transaction("1", vec![posting("Checking")]);
        txn.name = Some("Ridgeline Partners, LP".to_string());
        assert_eq!(description_for(&txn), "Ridgeline Partners, LP");
    }

    #[test]
    fn description_falls_back_to_type_and_first_memo_when_there_is_no_name() {
        let mut txn = transaction("1", vec![posting("Checking")]);
        txn.postings[0].memo = Some("Opening Balance Entry".to_string());
        assert_eq!(
            description_for(&txn),
            "Journal Entry: Opening Balance Entry"
        );
    }

    #[test]
    fn description_falls_back_to_bare_type_with_no_name_and_no_memo() {
        let txn = transaction("1", vec![posting("Checking")]);
        assert_eq!(description_for(&txn), "Journal Entry");
    }

    #[test]
    fn an_empty_name_is_treated_as_absent() {
        let mut txn = transaction("1", vec![posting("Checking")]);
        txn.name = Some(String::new());
        txn.postings[0].memo = Some("Opening Balance Entry".to_string());
        assert_eq!(
            description_for(&txn),
            "Journal Entry: Opening Balance Entry"
        );
    }

    #[test]
    fn journal_safe_replaces_a_semicolon_which_hledger_has_no_escape_for() {
        // `parse::split_comment` is a plain `line.find(';')` — the FIRST one
        // anywhere in a header ends the description there, silently.
        assert_eq!(journal_safe("Smith; Jones LLP"), "Smith, Jones LLP");
    }

    #[test]
    fn journal_safe_collapses_embedded_newlines() {
        // A raw newline would split one journal line into two.
        assert_eq!(journal_safe("Acme\nCorp"), "Acme Corp");
        assert_eq!(journal_safe("Acme\r\nCorp"), "Acme  Corp");
    }

    #[test]
    fn journal_safe_leaves_ordinary_text_untouched() {
        assert_eq!(
            journal_safe("Ridgeline Partners, LP"),
            "Ridgeline Partners, LP"
        );
    }

    #[test]
    fn a_semicolon_in_the_payee_name_does_not_reach_the_description_verbatim() {
        // The exact failure mode reported against a real export: a payee
        // name with a semicolon reached the journal-writing round-trip guard
        // as an unnamed EditError::RoundTripMismatch.
        let mut txn = transaction("1", vec![posting("Checking")]);
        txn.name = Some("Smith; Jones LLP".to_string());
        assert_eq!(description_for(&txn), "Smith, Jones LLP");
    }
}
