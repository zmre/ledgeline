//! Declared/inferred account types — port of
//! `web/src/lib/domain/accountTypes.ts`.
//!
//! An account's EFFECTIVE type is its own declared `type:`, else the nearest
//! declared ancestor's, else inferred from the name. `Cash` is the subtype
//! `hledger cashflow` selects on.

use super::accounts::{RootCategory, ascii_or_lowercased, categorize};
use crate::model::{AccountDeclaration, Journal};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

/// A resolved account type. `Cash`, `Conversion` and `Gain` are the subtypes
/// hledger tracks beyond the five roots.
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
    /// `V` — a subtype of Equity (hledger `type:V` accounts also match
    /// `type:E`, and `bse` files them inside the Equity section).
    Conversion,
    /// `G` — a subtype of Revenue (hledger `type:G` accounts also match
    /// `type:R`).
    Gain,
}

/// Fold two types into the one that describes both, or `None` when they
/// genuinely disagree. Subtypes collapse into their parent type, mirroring
/// [`is_account_type`]'s hierarchy (`Cash`→`Asset`, `Gain`→`Revenue`,
/// `Conversion`→`Equity`).
fn unify(a: AccountType, b: AccountType) -> Option<AccountType> {
    use AccountType::{Asset, Cash, Conversion, Equity, Gain, Revenue};
    match (a, b) {
        _ if a == b => Some(a),
        (Cash, Asset) | (Asset, Cash) => Some(Asset),
        (Gain, Revenue) | (Revenue, Gain) => Some(Revenue),
        (Conversion, Equity) | (Equity, Conversion) => Some(Equity),
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

/// The `account` directive tag naming an account's TYPE — hledger's own, and
/// the only one of the five closed-vocabulary tags this codebase did not invent.
pub const ACCOUNT_TYPE_TAG: &str = "type";

/// The cash-like segments hledger's Cash-account heuristic looks for below an
/// asset root.
const CASH_SEGMENTS: [&str; 7] = [
    "cash", "bank", "checking", "chequing", "savings", "saving", "current",
];

/// hledger's Cash-account name heuristic, equivalent to the TS regex
/// `^assets?(:.+)?:(cash|bank|che(ck|que)ing|savings?|current)(:|$)`:
/// an `asset`/`assets`-rooted account with a cash-like segment anywhere below
/// the root.
///
/// Splitting and then comparing case-insensitively, rather than lowercasing the
/// whole name first, is what removes the per-call `String` (PERF-5e). It is the
/// same classification: nothing lowercases to or from `:`, so the segmentation
/// cannot move — see [`ascii_or_lowercased`] for the non-ASCII half.
fn matches_cash_name(account: &str) -> bool {
    let lowered = ascii_or_lowercased(account);
    let mut segments = lowered.split(':');
    let asset_rooted = segments.next().is_some_and(|root| {
        ["asset", "assets"]
            .iter()
            .any(|r| root.eq_ignore_ascii_case(r))
    });
    asset_rooted
        && segments.any(|segment| {
            CASH_SEGMENTS
                .iter()
                .any(|cash| segment.eq_ignore_ascii_case(cash))
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
        // Seeking past `account` itself and taking while the name is still
        // prefixed by it spans that block; the `:` test then rejects the
        // siblings that merely share the same opening letters (`activos:banco`
        // for `activo`) and would otherwise sort in among them. Same set, same
        // order, without the `format!("{account}:")` key this allocated on every
        // untyped account (PERF-5e).
        declared
            .range::<str, _>((Bound::Excluded(account), Bound::Unbounded))
            .take_while(|(declared_name, _)| declared_name.starts_with(account))
            .filter(|(declared_name, _)| declared_name.as_bytes().get(account.len()) == Some(&b':'))
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
/// account is matched by hledger's `type:R` query). `Conversion` counts as an
/// `Equity` likewise (verified: hledger 1.52's `type:E` query matches a
/// declared `type: V` account, and `bse` prints `equity:conversion` inside the
/// Equity section) — a balance sheet that dropped it would leak the whole
/// multi-commodity conversion residue into its `A − L − E` check line.
#[must_use]
pub fn is_account_type(
    account: &str,
    declared: &BTreeMap<String, AccountType>,
    category: AccountType,
) -> bool {
    is_category(resolve_account_type(account, declared), category)
}

/// Whether an already-resolved type is a member of `category`, subtypes
/// included. Shared by [`is_account_type`] and [`AccountTypes::is_type`] so the
/// two cannot drift.
fn is_category(resolved: Option<AccountType>, category: AccountType) -> bool {
    match resolved {
        Some(AccountType::Cash) => matches!(category, AccountType::Asset | AccountType::Cash),
        Some(AccountType::Gain) => matches!(category, AccountType::Revenue | AccountType::Gain),
        Some(AccountType::Conversion) => {
            matches!(category, AccountType::Equity | AccountType::Conversion)
        }
        Some(found) => found == category,
        None => false,
    }
}

/// [`resolve_account_type`] memoized over one immutable set of declarations.
///
/// Resolution is pure but not free: it probes a `BTreeMap` once per ancestor
/// and, when nothing in the ancestry is declared, infers from the name and then
/// scans for declared descendants. Reports call it once per POSTING — 840k times
/// at 200k transactions in `detect_subscriptions` alone — over only a couple of
/// hundred DISTINCT account names, so every answer after the first is a repeat
/// (PERF-5e).
///
/// Purity is intact: the cache observes nothing but its own declarations and the
/// names it is asked about, so for a given `declared` this is the same total
/// function as the free [`resolve_account_type`], only cheaper. No clock, no
/// I/O.
///
/// The memo is interior-mutable so this stays a `&self` API, shareable by
/// reference exactly where [`declared_types`]' map is passed today. That makes
/// it `Send` but NOT `Sync` — the right trade for a cache built per report. A
/// cache shared across threads (an HTTP snapshot serving concurrent requests)
/// wants the eager form instead: resolve every account name in the journal once
/// at build time into a plain immutable map.
#[derive(Debug, Clone)]
pub struct AccountTypes {
    declared: BTreeMap<String, AccountType>,
    memo: RefCell<HashMap<String, Option<AccountType>>>,
}

impl AccountTypes {
    /// Memoize over the types declared by `decls`.
    #[must_use]
    pub fn new(decls: &[AccountDecl]) -> Self {
        Self::from_declared(declared_types(decls))
    }

    /// Memoize over an already-built [`declared_types`] map.
    #[must_use]
    pub fn from_declared(declared: BTreeMap<String, AccountType>) -> Self {
        Self {
            declared,
            memo: RefCell::new(HashMap::new()),
        }
    }

    /// The declarations this was built from, for callers that still need to hand
    /// the raw map to a free function.
    #[must_use]
    pub fn declared(&self) -> &BTreeMap<String, AccountType> {
        &self.declared
    }

    /// [`resolve_account_type`], answered from the memo after the first ask.
    pub fn resolve(&self, account: &str) -> Option<AccountType> {
        if let Some(hit) = self.memo.borrow().get(account) {
            return *hit;
        }
        let resolved = resolve_account_type(account, &self.declared);
        self.memo.borrow_mut().insert(account.to_string(), resolved);
        resolved
    }

    /// [`is_account_type`], answered from the memo.
    pub fn is_type(&self, account: &str, category: AccountType) -> bool {
        is_category(self.resolve(account), category)
    }

    /// True when `account`'s effective type is Cash — the [`cash_predicate`]
    /// test, answered from the memo.
    pub fn is_cash(&self, account: &str) -> bool {
        self.resolve(account) == Some(AccountType::Cash)
    }
}

/// Cash predicate for the cash-flow report: an account's effective type is Cash.
/// With NO declared types this reduces to the pure name heuristic.
///
/// Backed by [`AccountTypes`], so the repeated asks a bucketed cash-flow report
/// makes about the same handful of accounts cost one hash lookup each.
pub fn cash_predicate(decls: &[AccountDecl]) -> impl Fn(&str) -> bool {
    let types = AccountTypes::new(decls);
    move |account: &str| types.is_cash(account)
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
                .find(|(key, _)| key == ACCOUNT_TYPE_TAG)
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

    /// hledger defines `type: V` as a SUBTYPE of equity — its `type:E` query
    /// matches a declared `type: V` account, and `bse` files it under Equity.
    /// A `Conversion` that matched no balance-sheet section leaked its whole
    /// residue into the `A − L − E` check line. The name is deliberately one the
    /// English heuristic cannot classify, so the declared type does all the work.
    #[test]
    fn conversion_is_an_equity_subtype() {
        let decls = vec![AccountDecl {
            name: "trading:usd-eur".into(),
            account_type: Some(AccountType::Conversion),
        }];
        let declared = declared_types(&decls);
        assert!(is_account_type(
            "trading:usd-eur",
            &declared,
            AccountType::Equity
        ));
        assert!(is_account_type(
            "trading:usd-eur",
            &declared,
            AccountType::Conversion
        ));
        // The subtype folds UP only: an `Equity` account is not a `Conversion`,
        // exactly as an `Asset` is not a `Cash`.
        assert!(!is_account_type(
            "trading:usd-eur",
            &declared,
            AccountType::Asset
        ));
        let equity = declared_types(&[AccountDecl {
            name: "patrimonio:inicio".into(),
            account_type: Some(AccountType::Equity),
        }]);
        assert!(!is_account_type(
            "patrimonio:inicio",
            &equity,
            AccountType::Conversion
        ));
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

        // Conversion is an Equity subtype, so a Conversion+Equity subtree is an
        // Equity subtree — the mixed declared equity of a journal that books
        // both `type: E` capital and a `type: V` conversion account.
        let conversion_and_equity = declared_types(&[
            AccountDecl {
                name: "patrimonio:cambio".into(),
                account_type: Some(AccountType::Conversion),
            },
            AccountDecl {
                name: "patrimonio:inicio".into(),
                account_type: Some(AccountType::Equity),
            },
        ]);
        assert_eq!(
            resolve_account_type("patrimonio", &conversion_and_equity),
            Some(AccountType::Equity)
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
        // Memoized: the second ask must give the same answer as the first.
        assert!(pred("assets:bank:checking"));
        assert!(!pred("assets:broker:taxable:aapl"));
    }

    /// The Cash heuristic compares segments case-insensitively instead of
    /// lowercasing the whole name, so the non-ASCII path has to keep doing the
    /// real Unicode lowering: U+212A KELVIN SIGN folds to a plain `k`, which
    /// `eq_ignore_ascii_case` alone would never see.
    #[test]
    fn cash_name_heuristic_keeps_unicode_lowering() {
        assert!(matches_cash_name("assets:BAN\u{212A}"));
        assert!(matches_cash_name("assets:BAN\u{212A}:checking"));
        assert!(!matches_cash_name("assets:BAN\u{212A}X"));
        assert!(!matches_cash_name("expenses:BAN\u{212A}"));
    }

    /// Every name in a chart that exercises all four resolution steps — own
    /// declaration, declared ancestor, name inference, declared descendant — plus
    /// the misses in between.
    fn resolution_probe_chart() -> (BTreeMap<String, AccountType>, Vec<&'static str>) {
        let declared = declared_types(&[
            AccountDecl {
                name: "assets".into(),
                account_type: Some(AccountType::Asset),
            },
            AccountDecl {
                name: "assets:bank:checking".into(),
                account_type: Some(AccountType::Cash),
            },
            AccountDecl {
                name: "cuenta:uno".into(),
                account_type: Some(AccountType::Cash),
            },
            AccountDecl {
                name: "cuenta:dos".into(),
                account_type: Some(AccountType::Cash),
            },
            // Sorts between `cuenta` and `cuenta:` — the sibling the `:` test
            // has to reject.
            AccountDecl {
                name: "cuenta-vieja".into(),
                account_type: Some(AccountType::Liability),
            },
            AccountDecl {
                name: "mixto:a".into(),
                account_type: Some(AccountType::Liability),
            },
            AccountDecl {
                name: "mixto:b".into(),
                account_type: Some(AccountType::Expense),
            },
        ]);
        let names = vec![
            "assets",
            "assets:bank",
            "assets:bank:checking",
            "assets:bank:checkingx",
            "cuenta",
            "cuenta:uno",
            "cuenta-vieja",
            "cuentax",
            "mixto",
            "mixto:a",
            "expenses:food",
            "liabilities:cc:visa",
            "misc",
            "",
        ];
        (declared, names)
    }

    /// The descendant fallback seeks past `account` itself rather than building
    /// an `account:` key, so a sibling sorting between the two must not be
    /// mistaken for a descendant — nor cut the scan short before the real ones.
    #[test]
    fn descendant_fallback_skips_siblings_that_sort_between_parent_and_children() {
        let (declared, _) = resolution_probe_chart();
        // `cuenta-vieja` (Liability) sorts before `cuenta:dos`; only the two
        // `cuenta:` children count, and they agree on Cash.
        assert_eq!(
            resolve_account_type("cuenta", &declared),
            Some(AccountType::Cash)
        );
        // `cuentax` has no descendants at all despite sharing a prefix.
        assert_eq!(resolve_account_type("cuentax", &declared), None);
        // Disagreeing descendants still collapse to `None`.
        assert_eq!(resolve_account_type("mixto", &declared), None);
    }

    /// The memo must be indistinguishable from the free function, on every
    /// resolution path and on repeat asks.
    #[test]
    fn account_types_memo_agrees_with_the_free_function() {
        let (declared, names) = resolution_probe_chart();
        let types = AccountTypes::from_declared(declared.clone());
        for pass in 0..2 {
            for name in &names {
                assert_eq!(
                    types.resolve(name),
                    resolve_account_type(name, &declared),
                    "pass {pass}, account {name:?}"
                );
                for category in [
                    AccountType::Asset,
                    AccountType::Liability,
                    AccountType::Equity,
                    AccountType::Revenue,
                    AccountType::Expense,
                    AccountType::Cash,
                    AccountType::Conversion,
                    AccountType::Gain,
                ] {
                    assert_eq!(
                        types.is_type(name, category),
                        is_account_type(name, &declared, category),
                        "pass {pass}, account {name:?}, category {category:?}"
                    );
                }
                assert_eq!(
                    types.is_cash(name),
                    resolve_account_type(name, &declared) == Some(AccountType::Cash),
                    "pass {pass}, is_cash {name:?}"
                );
            }
        }
        assert_eq!(types.declared(), &declared);
    }
}
