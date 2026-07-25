//! Declared/inferred account types — port of
//! `web/src/lib/domain/accountTypes.ts`.
//!
//! An account's EFFECTIVE type is its own declared `type:`, else the nearest
//! declared ancestor's, else inferred from the name. `Cash` is the subtype
//! `hledger cashflow` selects on.

use super::accounts::{RootCategory, categorize};
use crate::model::{AccountDeclaration, Journal};
use std::collections::BTreeMap;

/// A resolved account type. `Cash`/`Conversion` are the two subtypes hledger
/// tracks beyond the five roots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountType {
    /// `A`
    Asset,
    /// `L`
    Liability,
    /// `E`
    Equity,
    /// `R`
    Revenue,
    /// `X`
    Expense,
    /// `C` — a subtype of Asset; what `hledger cashflow` selects on.
    Cash,
    /// `V`
    Conversion,
}

/// One account's declared type as read from `account` directives (`None` when
/// no `type:` tag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDecl {
    /// Full account name.
    pub name: String,
    /// Declared type, or `None`.
    pub account_type: Option<AccountType>,
}

/// hledger's Cash-account name heuristic, equivalent to the TS regex
/// `^assets?(:.+)?:(cash|bank|che(ck|que)ing|savings?|current)(:|$)`:
/// an `asset`/`assets`-rooted account with a cash-like segment anywhere below
/// the root.
fn matches_cash_name(account: &str) -> bool {
    let lower = account.to_lowercase();
    let mut segments = lower.split(':');
    match segments.next() {
        Some("asset" | "assets") => {}
        _ => return false,
    }
    segments.any(|segment| {
        matches!(
            segment,
            "cash" | "bank" | "checking" | "chequing" | "savings" | "saving" | "current"
        )
    })
}

/// Parse a `type:` tag value (single letter `A/L/E/R/X/C/V` or a full word),
/// case-insensitively; `None` when unrecognized.
#[must_use]
pub fn parse_account_type_tag(value: &str) -> Option<AccountType> {
    let v = value.trim().to_lowercase();
    if v.chars().count() == 1 {
        match v.as_str() {
            "a" => Some(AccountType::Asset),
            "l" => Some(AccountType::Liability),
            "e" => Some(AccountType::Equity),
            "r" => Some(AccountType::Revenue),
            "x" => Some(AccountType::Expense),
            "c" => Some(AccountType::Cash),
            "v" => Some(AccountType::Conversion),
            _ => None,
        }
    } else {
        match v.as_str() {
            "asset" => Some(AccountType::Asset),
            "liability" => Some(AccountType::Liability),
            "equity" => Some(AccountType::Equity),
            "revenue" | "income" => Some(AccountType::Revenue),
            "expense" => Some(AccountType::Expense),
            "cash" => Some(AccountType::Cash),
            "conversion" => Some(AccountType::Conversion),
            _ => None,
        }
    }
}

/// hledger's name-based type inference — the fallback when nothing in the
/// ancestry is declared. `None` when no convention matches.
#[must_use]
pub fn infer_account_type(account: &str) -> Option<AccountType> {
    if matches_cash_name(account) {
        return Some(AccountType::Cash);
    }
    match categorize(account) {
        RootCategory::Asset => Some(AccountType::Asset),
        RootCategory::Liability => Some(AccountType::Liability),
        RootCategory::Equity => Some(AccountType::Equity),
        RootCategory::Revenue => Some(AccountType::Revenue),
        RootCategory::Expense => Some(AccountType::Expense),
        RootCategory::Other => None,
    }
}

/// Declared (non-`None`) types keyed by account name.
#[must_use]
pub fn declared_types(decls: &[AccountDecl]) -> BTreeMap<String, AccountType> {
    decls
        .iter()
        .filter_map(|decl| decl.account_type.map(|ty| (decl.name.clone(), ty)))
        .collect()
}

/// Effective type of `account`: own declared → nearest declared ancestor → name
/// inference → a declared DESCENDANT's type (`None` when untyped).
///
/// The descendant step is last for a reason. It exists for the parent rows a
/// depth-clamped report invents: clamping `activo:banco` to depth 1 yields
/// `activo`, which is declared nowhere and means nothing to the English name
/// heuristic, so without it that row — and therefore the whole section total
/// rolled up from depth-1 roots — would be dropped. Running it AFTER name
/// inference keeps every conventional chart of accounts behaving exactly as
/// before: `assets` still infers `Asset` from its name rather than picking up
/// `Cash` from some `assets:bank:checking ; type: C` child.
#[must_use]
pub fn resolve_account_type(
    account: &str,
    declared: &BTreeMap<String, AccountType>,
) -> Option<AccountType> {
    let mut name = account;
    loop {
        if let Some(ty) = declared.get(name) {
            return Some(*ty);
        }
        match name.rfind(':') {
            Some(cut) => name = &name[..cut],
            None => break,
        }
    }
    infer_account_type(account).or_else(|| {
        let prefix = format!("{account}:");
        declared
            .iter()
            .find(|(declared_name, _)| declared_name.starts_with(&prefix))
            .map(|(_, ty)| *ty)
    })
}

/// True when `account`'s effective type belongs to `category`.
///
/// Use this rather than [`crate::reports::accounts::categorize`] for any report
/// that groups accounts: names are a fallback, not the source of truth, so a
/// chart of accounts that books costs under `cogs:` — or in a language other
/// than English — still lands in the right section.
///
/// `Cash` counts as an `Asset`, which is what it is: hledger treats it as an
/// Asset subtype that `cashflow` selects on, and a balance sheet that dropped
/// every declared `type: C` account would be missing most of its assets.
#[must_use]
pub fn is_account_type(
    account: &str,
    declared: &BTreeMap<String, AccountType>,
    category: AccountType,
) -> bool {
    match resolve_account_type(account, declared) {
        Some(AccountType::Cash) => matches!(category, AccountType::Asset | AccountType::Cash),
        Some(found) => found == category,
        None => false,
    }
}

/// Cash predicate for the cash-flow report: an account's effective type is Cash.
/// With NO declared types this reduces to the pure name heuristic.
pub fn cash_predicate(decls: &[AccountDecl]) -> impl Fn(&str) -> bool {
    let declared = declared_types(decls);
    move |account: &str| resolve_account_type(account, &declared) == Some(AccountType::Cash)
}

/// Read the declared `type:` per account from a parsed journal's `account`
/// directives — the engine's equivalent of `normalizeAccounts` over `/accounts`.
/// (Only explicitly-declared accounts carry a non-`None` type, which is all
/// [`cash_predicate`] consults.)
#[must_use]
pub fn account_decls(journal: &Journal) -> Vec<AccountDecl> {
    account_decls_from(&journal.accounts)
}

/// [`account_decls`] over a bare slice, for callers holding the declarations
/// without the whole [`Journal`] (the holdings engine).
#[must_use]
pub fn account_decls_from(accounts: &[AccountDeclaration]) -> Vec<AccountDecl> {
    accounts
        .iter()
        .map(|decl| {
            let account_type = decl
                .tags
                .iter()
                .find(|(key, _)| key == "type")
                .and_then(|(_, value)| parse_account_type_tag(value));
            AccountDecl {
                name: decl.name.0.clone(),
                account_type,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_type_tags() {
        assert_eq!(parse_account_type_tag("C"), Some(AccountType::Cash));
        assert_eq!(parse_account_type_tag("a"), Some(AccountType::Asset));
        assert_eq!(parse_account_type_tag("Cash"), Some(AccountType::Cash));
        assert_eq!(parse_account_type_tag("income"), Some(AccountType::Revenue));
        assert_eq!(parse_account_type_tag("  L "), Some(AccountType::Liability));
        assert_eq!(parse_account_type_tag("Z"), None);
        assert_eq!(parse_account_type_tag("nonsense"), None);
    }

    #[test]
    fn resolves_declared_then_ancestor_then_name() {
        let decls = vec![
            AccountDecl {
                name: "assets".into(),
                account_type: Some(AccountType::Asset),
            },
            AccountDecl {
                name: "assets:bank:checking".into(),
                account_type: Some(AccountType::Cash),
            },
        ];
        let declared = declared_types(&decls);
        // Own declaration wins even though the name says "bank" (Cash-ish).
        assert_eq!(
            resolve_account_type("assets:bank:checking", &declared),
            Some(AccountType::Cash)
        );
        // Nearest declared ancestor: assets ; type: A overrides the bank name.
        assert_eq!(
            resolve_account_type("assets:bankofamerica", &declared),
            Some(AccountType::Asset)
        );
        // No declaration in ancestry → name inference.
        assert_eq!(
            resolve_account_type("expenses:food", &BTreeMap::new()),
            Some(AccountType::Expense)
        );
    }

    #[test]
    fn cash_name_heuristic_matches_hledger() {
        assert!(matches_cash_name("assets:bank:checking"));
        assert!(matches_cash_name("assets:bank:wise:eur"));
        assert!(matches_cash_name("assets:broker:taxable:cash"));
        assert!(matches_cash_name("asset:savings"));
        assert!(matches_cash_name("ASSETS:BANK"));
        assert!(!matches_cash_name("assets:broker:taxable:aapl"));
        assert!(!matches_cash_name("assets"));
        assert!(!matches_cash_name("expenses:bank"));
        assert!(!matches_cash_name("liabilities:cc:visa"));
    }

    #[test]
    fn cash_predicate_falls_back_to_names_without_declarations() {
        let pred = cash_predicate(&[]);
        assert!(pred("assets:bank:checking"));
        assert!(!pred("assets:broker:taxable:aapl"));
    }
}
