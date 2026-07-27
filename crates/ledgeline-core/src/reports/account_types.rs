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
    /// `G` — a subtype of Revenue (hledger `type:G` accounts also match
    /// `type:R`).
    Gain,
}

/// Fold two types into the one that describes both, or `None` when they
/// genuinely disagree. Subtypes collapse into their parent type, mirroring
/// [`is_account_type`]'s hierarchy (`Cash`→`Asset`, `Gain`→`Revenue`).
fn unify(a: AccountType, b: AccountType) -> Option<AccountType> {
    use AccountType::{Asset, Cash, Gain, Revenue};
    match (a, b) {
        _ if a == b => Some(a),
        (Cash, Asset) | (Asset, Cash) => Some(Asset),
        (Gain, Revenue) | (Revenue, Gain) => Some(Revenue),
        _ => None,
    }
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

/// Parse a `type:` tag value, case-insensitively; `None` when unrecognized.
///
/// Accepts everything hledger 1.52 does — the single letters `A L E R X C V G`
/// and the singular words `Asset Liability Equity Revenue Expense Cash
/// Conversion` — plus a deliberate superset, because hledger's response to an
/// unrecognized code is a hard parse ERROR while ours is a silent `None`:
///
/// ```text
/// $ hledger -f j.journal bal            # account cogs:infra ; type: expenses
/// hledger: Error: j.journal:1:1: invalid account type code expenses, should be
///   one of A, L, E, R, X, C, V, G, Asset, Liability, ..., Gain
/// ```
///
/// A silent `None` falls back to name inference, so `type: expenses` on a
/// `cogs:` account used to declare NOTHING and vanish from the income statement
/// — the "reports read zero" symptom with a fresh cause (RPT-1). Until the
/// parser can reject the directive outright we accept the plurals (and
/// `income`, which hledger also rejects) rather than silently misfile them:
/// every journal hledger ACCEPTS is classified identically, and the ones it
/// rejects get the obviously-intended meaning instead of disappearing.
///
/// Note `Gains` is accepted by hledger 1.52 but the singular `Gain` is not,
/// despite its own error message listing `Gain`. We take both.
#[must_use]
pub fn parse_account_type_tag(value: &str) -> Option<AccountType> {
    match value.trim().to_lowercase().as_str() {
        // Single letters — exactly hledger's set.
        "a" => Some(AccountType::Asset),
        "l" => Some(AccountType::Liability),
        "e" => Some(AccountType::Equity),
        "r" => Some(AccountType::Revenue),
        "x" => Some(AccountType::Expense),
        "c" => Some(AccountType::Cash),
        "v" => Some(AccountType::Conversion),
        "g" => Some(AccountType::Gain),
        // Words. The singulars are hledger's; the plurals and `income` are ours
        // (hledger errors on them) — see the doc comment.
        "asset" | "assets" => Some(AccountType::Asset),
        "liability" | "liabilities" => Some(AccountType::Liability),
        "equity" | "equities" => Some(AccountType::Equity),
        "revenue" | "revenues" | "income" | "incomes" => Some(AccountType::Revenue),
        "expense" | "expenses" => Some(AccountType::Expense),
        "cash" => Some(AccountType::Cash),
        "conversion" | "conversions" => Some(AccountType::Conversion),
        "gain" | "gains" => Some(AccountType::Gain),
        _ => None,
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
/// heuristic, so without it that row would be dropped. Running it AFTER name
/// inference keeps every conventional chart of accounts behaving exactly as
/// before: `assets` still infers `Asset` from its name rather than picking up
/// `Cash` from some `assets:bank:checking ; type: C` child.
///
/// That step considers ALL declared descendants and requires them to agree
/// (up to the subtype hierarchy — see [`unify`]). It used to take the lexically
/// FIRST one, which meant a mixed subtree was classified by whichever child
/// happened to sort first, and merely RENAMING a child silently reclassified
/// every ancestor. A genuinely mixed subtree has no single type, so it gets
/// `None` and is simply not a member of any section — its typed descendants
/// still are, and section totals are summed from those (RPT-1).
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
        // Declared descendants are contiguous in the BTreeMap under `account:`.
        let prefix = format!("{account}:");
        declared
            .range(prefix.clone()..)
            .take_while(|(declared_name, _)| declared_name.starts_with(&prefix))
            .map(|(_, ty)| *ty)
            .try_fold(None::<AccountType>, |acc, ty| match acc {
                None => Some(Some(ty)),
                Some(seen) => unify(seen, ty).map(Some),
            })
            .flatten()
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
/// `Gain` counts as a `Revenue` for the same reason (verified: a `type:G`
/// account is matched by hledger's `type:R` query).
#[must_use]
pub fn is_account_type(
    account: &str,
    declared: &BTreeMap<String, AccountType>,
    category: AccountType,
) -> bool {
    match resolve_account_type(account, declared) {
        Some(AccountType::Cash) => matches!(category, AccountType::Asset | AccountType::Cash),
        Some(AccountType::Gain) => matches!(category, AccountType::Revenue | AccountType::Gain),
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

    /// Every single letter and singular word hledger 1.52 accepts, verified
    /// against the CLI. hledger REJECTS the word `Gain` (its own error message
    /// notwithstanding) but accepts the letter `G`.
    #[test]
    fn parses_every_hledger_type_code() {
        for (tag, want) in [
            ("A", AccountType::Asset),
            ("L", AccountType::Liability),
            ("E", AccountType::Equity),
            ("R", AccountType::Revenue),
            ("X", AccountType::Expense),
            ("C", AccountType::Cash),
            ("V", AccountType::Conversion),
            ("G", AccountType::Gain),
            ("Asset", AccountType::Asset),
            ("Liability", AccountType::Liability),
            ("Equity", AccountType::Equity),
            ("Revenue", AccountType::Revenue),
            ("Expense", AccountType::Expense),
            ("Cash", AccountType::Cash),
            ("Conversion", AccountType::Conversion),
            ("Gains", AccountType::Gain),
        ] {
            assert_eq!(parse_account_type_tag(tag), Some(want), "tag {tag}");
            assert_eq!(
                parse_account_type_tag(&tag.to_lowercase()),
                Some(want),
                "tag {tag} lowercased"
            );
        }
    }

    /// The plurals hledger errors on. Ours must not silently return `None` —
    /// that is the "declares nothing, falls back to name inference, vanishes
    /// from the report" bug (RPT-1).
    #[test]
    fn parses_plurals_hledger_rejects_rather_than_silently_dropping_them() {
        for (tag, want) in [
            ("expenses", AccountType::Expense),
            ("assets", AccountType::Asset),
            ("liabilities", AccountType::Liability),
            ("equities", AccountType::Equity),
            ("revenues", AccountType::Revenue),
            ("incomes", AccountType::Revenue),
            ("conversions", AccountType::Conversion),
        ] {
            assert_eq!(parse_account_type_tag(tag), Some(want), "tag {tag}");
        }
        // Still nothing for genuinely unrecognized values.
        for tag in ["", "  ", "Ass", "Exp", "Cashs", "Z", "nonsense"] {
            assert_eq!(parse_account_type_tag(tag), None, "tag {tag:?}");
        }
    }

    /// A `type: X` account under a root the English heuristic cannot classify
    /// used to declare nothing when spelled `expenses`, and so vanished.
    #[test]
    fn plural_expense_tag_classifies_a_cogs_account() {
        let decls = vec![AccountDecl {
            name: "cogs:infra".into(),
            account_type: parse_account_type_tag("expenses"),
        }];
        let declared = declared_types(&decls);
        assert!(is_account_type(
            "cogs:infra",
            &declared,
            AccountType::Expense
        ));
    }

    #[test]
    fn gain_is_a_revenue_subtype() {
        let decls = vec![AccountDecl {
            name: "biz:fx".into(),
            account_type: Some(AccountType::Gain),
        }];
        let declared = declared_types(&decls);
        assert!(is_account_type("biz:fx", &declared, AccountType::Revenue));
        assert!(is_account_type("biz:fx", &declared, AccountType::Gain));
        assert!(!is_account_type("biz:fx", &declared, AccountType::Expense));
    }

    #[test]
    fn descendant_fallback_requires_declared_descendants_to_agree() {
        let mixed = declared_types(&[
            AccountDecl {
                name: "Personal:Card".into(),
                account_type: Some(AccountType::Liability),
            },
            AccountDecl {
                name: "Personal:Checking".into(),
                account_type: Some(AccountType::Cash),
            },
        ]);
        // Lexically first is `Personal:Card` (Liability). A mixed subtree has no
        // single type, so the parent is a member of NO section.
        assert_eq!(resolve_account_type("Personal", &mixed), None);

        // Renaming a child must not reclassify the parent.
        let renamed = declared_types(&[
            AccountDecl {
                name: "Personal:Aard".into(),
                account_type: Some(AccountType::Cash),
            },
            AccountDecl {
                name: "Personal:Zard".into(),
                account_type: Some(AccountType::Liability),
            },
        ]);
        assert_eq!(resolve_account_type("Personal", &renamed), None);
    }

    #[test]
    fn descendant_fallback_unifies_subtypes_and_agreeing_types() {
        // All Cash → Cash.
        let all_cash = declared_types(&[
            AccountDecl {
                name: "cuenta:uno".into(),
                account_type: Some(AccountType::Cash),
            },
            AccountDecl {
                name: "cuenta:dos".into(),
                account_type: Some(AccountType::Cash),
            },
        ]);
        assert_eq!(
            resolve_account_type("cuenta", &all_cash),
            Some(AccountType::Cash)
        );

        // Cash is an Asset subtype, so a Cash+Asset subtree is an Asset subtree.
        let cash_and_asset = declared_types(&[
            AccountDecl {
                name: "activo:banco".into(),
                account_type: Some(AccountType::Asset),
            },
            AccountDecl {
                name: "activo:efectivo".into(),
                account_type: Some(AccountType::Cash),
            },
        ]);
        assert_eq!(
            resolve_account_type("activo", &cash_and_asset),
            Some(AccountType::Asset)
        );

        // Only DESCENDANTS count — a sibling prefix must not leak in.
        let sibling = declared_types(&[AccountDecl {
            name: "activos:banco".into(),
            account_type: Some(AccountType::Asset),
        }]);
        assert_eq!(resolve_account_type("activo", &sibling), None);
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
