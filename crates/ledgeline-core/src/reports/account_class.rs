//! Per-account classification served to clients: each account's resolved TYPE
//! and balance-sheet TERM, resolved once on the engine so no client has to
//! re-implement [`resolve_account_type`] or the `bsterm:` rules (plan 26 B1).
//!
//! # What `term` means here
//!
//! The term is the account's own `bsterm:` tag, else its nearest declared
//! ancestor's ([`super::account_groups::declared_bs_terms`]). When nothing in the
//! ancestry declares one and the account resolves to Cash or Liability, the
//! name heuristic [`infer_bs_term`] may GUESS non-current; such a term carries
//! `term_guessed`, so a view that drops non-current balances can say it did so
//! on a guess and not on a tag.
//!
//! There is deliberately no group-derived default. The grouped balance sheet's
//! [`resolve_bs_term`](super::account_groups::resolve_bs_term) defaults the
//! Investments group to non-current, but that needs the group, which this
//! per-account view does not compute. `None` means "undeclared", and a caller
//! dealing in cash and liabilities (where Investments cannot arise) reads it as
//! current.

use super::account_groups::{BsTerm, declared_bs_terms, nearest_declared};
use super::account_types::{AccountType, AccountTypes, account_decls_from};
use crate::model::Journal;
use crate::wire::journal_to_accountnames;
use std::collections::BTreeMap;

/// One account's resolved classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountClass {
    /// The effective type ([`resolve_account_type`](super::resolve_account_type)),
    /// `None` when untyped.
    pub account_type: Option<AccountType>,
    /// Whether `account_type` came from a `type:` declaration on the account or
    /// an ancestor (as opposed to name inference or the declared-descendant
    /// step). A client classifying a name the engine never saw — an account a
    /// user is typing into a form — inherits a DECLARED ancestor's type exactly
    /// as the engine would, and must otherwise fall back to name inference.
    pub type_declared: bool,
    /// Declared (or inherited) `bsterm:`, else a name GUESS for a Cash or
    /// Liability account, else `None`.
    pub term: Option<BsTerm>,
    /// True when `term` is a guess by [`infer_bs_term`], never by a tag.
    pub term_guessed: bool,
}

/// Classify every account name the journal knows (`/accountnames`: posted-to,
/// declared, and every implied ancestor), keyed by full name.
#[must_use]
pub fn account_classes(journal: &Journal) -> BTreeMap<String, AccountClass> {
    let types = AccountTypes::new(&account_decls_from(&journal.accounts));
    let terms = declared_bs_terms(&journal.accounts);
    journal_to_accountnames(journal)
        .into_iter()
        .map(|name| {
            let class = classify(&name, &types, &terms);
            (name, class)
        })
        .collect()
}

fn classify(account: &str, types: &AccountTypes, terms: &BTreeMap<String, BsTerm>) -> AccountClass {
    let account_type = types.resolve(account);
    let type_declared = nearest_declared(types.declared(), account).is_some();
    let declared_term = nearest_declared(terms, account).copied();
    let guessed = match (declared_term, account_type) {
        (None, Some(kind @ (AccountType::Cash | AccountType::Liability))) => {
            infer_bs_term(account, kind)
        }
        _ => None,
    };
    AccountClass {
        account_type,
        type_declared,
        term: declared_term.or(guessed),
        term_guessed: declared_term.is_none() && guessed.is_some(),
    }
}

// ---------- the name heuristic ----------
//
// Everything below GUESSES a term from the account's name, and it exists under
// protest. The standing rule is that membership is decided by a declared tag and
// never by an English word (docs/reports/balance-sheet.md, "Why grouping never
// matches account names"), because a chart of accounts may be in any language.
// The guess runs only as the LAST resort, after the account and every ancestor
// have been asked for a `bsterm:` tag, and only for the cash view's two kinds.
// The grouped balance sheet does not use it.
//
// The word list is short on purpose. It comes from a survey of 108 public
// plain-text-accounting repositories (2,057 distinct account paths) plus the
// GnuCash, QuickBooks and Xero default charts, and the selection rule was
// PRECISION, not coverage: a false positive silently drops a real debt out of
// the user's short-term picture, which is worse than missing one. So words that
// are *usually* long-term are excluded — `loan`, `car`/`auto`/`vehicle`, `note`,
// `bond`, `lease`, `deposit`, `property`, `capital`, `fixed`, `principal`, and
// every short abbreviation (`ltd` is a company suffix, `sep` is September). A
// miss is one tag away; a balance the reader never saw is not.
//
// Ported from the SPA's former `domain/accountTerms.ts` `inferBsTerm`; its outputs
// are pinned by `fixtures/account-types/bs-term-cases.json`, captured from that
// implementation before it was deleted.

/// Explicit statements of intent: the tag's own vocabulary, written into the name.
const EXPLICIT_LONG_TERM: [&str; 2] = ["noncurrent", "longterm"];

/// Liability words that essentially only ever name a long-dated debt.
/// `mortage` is the observed misspelling; `student` because the corpus never
/// spelled `studentloan` as one word.
const LONG_TERM_LIABILITY_WORDS: [&str; 5] = ["mortgage", "mortage", "heloc", "student", "pension"];

/// Phrases that INVERT the reading, checked first: "Long-term debt, current
/// maturities" is a real FASB line item and a CURRENT liability.
const CURRENT_PORTION_PHRASES: [&str; 3] = ["currentmaturities", "currentportion", "duewithin"];

/// Every word in the path (lowercased, split on anything outside `[a-z0-9]`),
/// plus every adjacent pair joined — so one entry covers `noncurrent`,
/// `non-current` and `non current`.
fn path_words(account: &str) -> Vec<String> {
    let lowered = account.to_lowercase();
    let words: Vec<&str> = lowered
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|word| !word.is_empty())
        .collect();
    let pairs = words
        .windows(2)
        .map(|pair| format!("{}{}", pair[0], pair[1]));
    words
        .iter()
        .map(|word| (*word).to_string())
        .chain(pairs)
        .collect()
}

/// Guess whether `account` is non-current from its NAME, for an account whose
/// effective type is already known. `None` means "no opinion" — the answer for
/// almost every account, and one callers must treat as current.
///
/// `kind` gates the vocabulary: the liability words are consulted only for a
/// [`AccountType::Liability`], so `expenses:pension-contributions` and a savings
/// account called `assets:student` are untouched. Any other kind (in practice
/// Cash: declaring `type: C` is the owner saying it is a cash equivalent) gets
/// only the explicit words.
#[must_use]
pub fn infer_bs_term(account: &str, kind: AccountType) -> Option<BsTerm> {
    let words = path_words(account);
    let has = |word: &str| words.iter().any(|found| found == word);
    if CURRENT_PORTION_PHRASES.iter().any(|phrase| has(phrase)) {
        return None;
    }
    if EXPLICIT_LONG_TERM.iter().any(|word| has(word)) {
        return Some(BsTerm::NonCurrent);
    }
    (kind == AccountType::Liability && LONG_TERM_LIABILITY_WORDS.iter().any(|word| has(word)))
        .then_some(BsTerm::NonCurrent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    #[test]
    fn explicit_words_apply_to_either_kind() {
        for kind in [AccountType::Cash, AccountType::Liability] {
            assert_eq!(
                infer_bs_term("assets:bank:long-term-cd", kind),
                Some(BsTerm::NonCurrent)
            );
            assert_eq!(
                infer_bs_term("x:non current:y", kind),
                Some(BsTerm::NonCurrent)
            );
        }
    }

    #[test]
    fn current_portion_phrases_win() {
        assert_eq!(
            infer_bs_term(
                "liabilities:long-term-debt:current-maturities",
                AccountType::Liability
            ),
            None
        );
        assert_eq!(
            infer_bs_term(
                "liabilities:mortgage:due-within-one-year",
                AccountType::Liability
            ),
            None
        );
    }

    #[test]
    fn liability_words_only_for_liabilities() {
        assert_eq!(
            infer_bs_term("liabilities:mortgage", AccountType::Liability),
            Some(BsTerm::NonCurrent)
        );
        assert_eq!(
            infer_bs_term("assets:mortgage-escrow", AccountType::Cash),
            None
        );
        assert_eq!(
            infer_bs_term("liabilities:mortgageable", AccountType::Liability),
            None
        );
        assert_eq!(
            infer_bs_term("liabilities:cc:visa", AccountType::Liability),
            None
        );
    }

    #[test]
    fn classes_carry_declared_terms_and_guesses() {
        let text = "\
account assets:bank        ; type: C
account liabilities:house  ; type: L, bsterm: noncurrent
account activo             ; type: A

2026-01-01 x
    assets:bank                 $1
    liabilities:house:escrow   $-1

2026-01-02 y
    liabilities:mortgage        $1
    activo:caja                $-1
";
        let journal = parse_journal(text, "t.journal").expect("parses");
        let classes = account_classes(&journal);

        let bank = classes["assets:bank"];
        assert_eq!(bank.account_type, Some(AccountType::Cash));
        assert!(bank.type_declared);
        assert_eq!((bank.term, bank.term_guessed), (None, false));

        // Inherited from the tagged parent, so not a guess.
        let escrow = classes["liabilities:house:escrow"];
        assert_eq!(
            (escrow.term, escrow.term_guessed),
            (Some(BsTerm::NonCurrent), false)
        );
        assert!(
            escrow.type_declared,
            "inherits liabilities:house's declared type"
        );

        // Untagged, inferred liability, guessed long-term from the word.
        let mortgage = classes["liabilities:mortgage"];
        assert_eq!(mortgage.account_type, Some(AccountType::Liability));
        assert!(!mortgage.type_declared);
        assert_eq!(
            (mortgage.term, mortgage.term_guessed),
            (Some(BsTerm::NonCurrent), true)
        );

        // Ancestors are classified too; a declared ancestor's type is inherited.
        assert!(classes.contains_key("liabilities"));
        let caja = classes["activo:caja"];
        assert_eq!(caja.account_type, Some(AccountType::Asset));
        assert!(caja.type_declared);
    }
}
