//! Which Holdings tab an account belongs on — the `holdings:` account tag.
//!
//! The Holdings page has two tabs: Stocks (securities, keyed by commodity) and
//! Other (everything else you own, keyed by account). The default split is
//! mechanical — an asset account holding a non-currency commodity is a security
//! — and `holdings:` overrides it in either direction.
//!
//! This is a CODE, not prose, for the reason `type:` and `issection:` are codes:
//! a classification that decides *membership* must never match English words,
//! because the failure mode is a tab that reads empty and a chart of accounts may
//! be in any language. Three values, closed set.
//!
//! Why an override is needed at all: booking an asset as its own commodity
//! (`1 HOUSE @ $150,000` plus `P` directives) is the only way a dollar journal
//! makes a house *revalue*, and revaluation is the entire point of "change in
//! value over time". Without the tag, the price history that makes a house
//! interesting is exactly what would file it under Stocks.
//!
//! An unknown value falls back to the mechanical default, exactly as an absent
//! one does, and is REPORTED as an `account-tag` diagnostic — see
//! [`journal_to_tag_diagnostics`](crate::wire::journal_to_tag_diagnostics).
//!
//! It used to be refused outright, following `issection:` rather than `type:`,
//! on the argument that a silent fallback leaves the user who wrote
//! `holdings: real-estate` staring at their house on the Stocks tab with nothing
//! on screen to say why. The "nothing on screen" half was the real point and the
//! diagnostic keeps it — naming the account, the value and the alternatives.
//! Refusing was the wrong lever for it: `compute_holdings` feeds BOTH Holdings
//! tabs, the Insights tab and the drawer's own `stock-*` findings, so one typo
//! in one `account` directive blanked all four — including, through
//! `wire::journal_to_stock_diagnostics`'s `let Ok(..) else` fallback, the very
//! drawer that was supposed to explain it.

use std::collections::BTreeMap;

use crate::model::AccountDeclaration;

/// The account-directive tag that decides WHICH TAB an account appears on.
pub const HOLDINGS_TAG: &str = "holdings";

/// The account-directive tag that decides an account's ROLE WITHIN its holding.
pub const VALUATION_TAG: &str = "valuation";

/// The Holdings tab an account has been explicitly assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldingsClass {
    /// `holdings: stocks` — the Stocks tab only, never Other, even when the
    /// account holds nothing but currency.
    Stocks,
    /// `holdings: other` — the Other tab, whatever the account holds. Suppressed
    /// from Stocks so a tagged security is never counted on both tabs.
    Other,
    /// `holdings: none` — neither tab. This hides clutter (a petty receivable, a
    /// suspense account), never money: the account stays on the balance sheet.
    None,
}

/// Parse a `holdings:` tag value; `None` for anything outside the vocabulary.
///
/// Case- and whitespace-insensitive, and the singular spellings are accepted
/// alongside the plural ones: `stock` and `stocks` are the same instruction, and
/// the vocabulary is small enough that admitting both costs nothing. Anything
/// else is a typo, and [`declared_holdings_classes`] ignores it while the
/// `account-tag` diagnostic names it.
#[must_use]
pub fn parse_holdings_tag(value: &str) -> Option<HoldingsClass> {
    match value.trim().to_lowercase().as_str() {
        "stock" | "stocks" => Some(HoldingsClass::Stocks),
        "other" => Some(HoldingsClass::Other),
        "none" => Some(HoldingsClass::None),
        _ => None,
    }
}

/// The `holdings:` class declared per account by the journal's `account`
/// directives. Untagged accounts are absent.
///
/// An EMPTY value reads as no declaration at all, exactly as
/// [`account_sections_from`](crate::reports::account_sections_from) and
/// `declared_groups_from` treat one: `; holdings:` with nothing after it names
/// no tab, and three tag readers answering differently about the same syntax
/// would be its own trap. An UNRECOGNISED value reads the same way — see the
/// module docs for why that is a diagnostic rather than a refusal.
#[must_use]
pub fn declared_holdings_classes(
    accounts: &[AccountDeclaration],
) -> BTreeMap<String, HoldingsClass> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == HOLDINGS_TAG)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
                .and_then(parse_holdings_tag)
                .map(|class| (decl.name.0.clone(), class))
        })
        .collect()
}

/// What an account contributes to its holding's valuation.
///
/// A DIFFERENT tag from [`HoldingsClass`] on purpose, and the separation is the
/// one this codebase already makes twice: `type:` decides which statement
/// section an account is in and `bsgroup:` decides its line within that section;
/// `issection:` decides the box and `isgroup:` the line inside it. Membership and
/// role are never the same tag. `holdings:` says which TAB; `valuation:` says
/// what this account MEANS once it is on one, and overloading the first with the
/// second would make "move this to the Other tab" and "this is a paper gain"
/// the same sentence.
///
/// It exists because a very common way to model an illiquid asset carries the
/// cost/market split in the ACCOUNT TREE rather than in commodity costs:
///
/// ```journal
/// account assets:home:cost        ; type:A   ; what you actually paid
/// account assets:home:unrealized  ; type:A, valuation: unrealized
/// ```
///
/// Without the tag both sub-accounts are just dollars, so cost equals value and
/// the holding reports a change of zero — the tab's entire subject, reading as
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValuationRole {
    /// `valuation: cost` — money actually put in. The default for every untagged
    /// account, so it rarely needs writing.
    Cost,
    /// `valuation: unrealized` (or `depreciation`, `adjustment`, `mark`) — a
    /// valuation adjustment rather than money in. Counted in the holding's VALUE
    /// and excluded from its COST, which is what makes the difference between
    /// them the gain or loss.
    ///
    /// The variant is named for the ROLE rather than for any one of its spellings
    /// because the same mechanic serves two quite different stories: a house
    /// marked up to market, and a car written down by accumulated depreciation.
    /// Calling it `Unrealized` made the depreciation case read as a mistake.
    Adjustment,
}

/// Parse a `valuation:` tag value; `None` for anything outside the vocabulary.
///
/// Four spellings share the adjustment role. They are not synonyms for tidiness:
/// `unrealized` is the natural word for a house marked to market, `depreciation`
/// for a car written down, and `adjustment` for anything else that moves value
/// without moving basis. Forcing one word on all three would make two of them
/// read as errors in the journal that declares them.
#[must_use]
pub fn parse_valuation_tag(value: &str) -> Option<ValuationRole> {
    match value.trim().to_lowercase().as_str() {
        "cost" | "basis" => Some(ValuationRole::Cost),
        "unrealized" | "unrealised" | "mark" | "depreciation" | "adjustment" => {
            Some(ValuationRole::Adjustment)
        }
        _ => None,
    }
}

/// The `valuation:` role declared per account. Untagged accounts are absent and
/// read as [`ValuationRole::Cost`].
///
/// An empty or unrecognised value reads as no declaration, so the account keeps
/// the [`ValuationRole::Cost`] default — [`declared_holdings_classes`]'
/// reasoning, and with the sharpest consequence of the four tags: a misspelt
/// role reverts the account to `cost`, which makes a holding's unrealized gain
/// vanish into its basis and reports a real gain as zero. That is precisely why
/// the `account-tag` diagnostic exists; the gain is wrong either way if the tag
/// cannot be read, and a warning that names the account beats a blank tab.
#[must_use]
pub fn declared_valuation_roles(
    accounts: &[AccountDeclaration],
) -> BTreeMap<String, ValuationRole> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == VALUATION_TAG)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
                .and_then(parse_valuation_tag)
                .map(|role| (decl.name.0.clone(), role))
        })
        .collect()
}

/// The effective role of `account`: its own declared tag, else the nearest
/// declared ancestor's, else [`ValuationRole::Cost`].
#[must_use]
pub fn resolve_valuation_role(
    account: &str,
    declared: &BTreeMap<String, ValuationRole>,
) -> ValuationRole {
    let mut name = account;
    loop {
        if let Some(role) = declared.get(name) {
            return *role;
        }
        match name.rfind(':') {
            Some(cut) => name = &name[..cut],
            None => return ValuationRole::Cost,
        }
    }
}

/// The effective class of `account`: its own declared tag, else the nearest
/// declared ancestor's. `None` = untagged, and the caller falls back to the
/// mechanical default.
///
/// Deliberately shorter than
/// [`resolve_account_type`](crate::reports::resolve_account_type): no name
/// inference and no declared-DESCENDANT fallback. `type:` infers because hledger
/// journals are expected to have types whether or not anyone declared them;
/// `holdings:` is a pure opt-in override, so inferring one would be inventing an
/// instruction the user never gave.
#[must_use]
pub fn resolve_holdings_class(
    account: &str,
    declared: &BTreeMap<String, HoldingsClass>,
) -> Option<HoldingsClass> {
    let mut name = account;
    loop {
        if let Some(class) = declared.get(name) {
            return Some(*class);
        }
        name = &name[..name.rfind(':')?];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountName, SourcePos};

    fn decl(name: &str, tags: &[(&str, &str)]) -> AccountDeclaration {
        AccountDeclaration {
            name: AccountName(name.to_string()),
            tags: tags
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            comment: String::new(),
            position: SourcePos { line: 1, column: 1 },
        }
    }

    fn classes(decls: &[AccountDeclaration]) -> BTreeMap<String, HoldingsClass> {
        declared_holdings_classes(decls)
    }

    #[test]
    fn parses_the_closed_vocabulary_case_insensitively() {
        assert_eq!(parse_holdings_tag("other"), Some(HoldingsClass::Other));
        assert_eq!(parse_holdings_tag("  Other "), Some(HoldingsClass::Other));
        assert_eq!(parse_holdings_tag("STOCKS"), Some(HoldingsClass::Stocks));
        assert_eq!(parse_holdings_tag("stock"), Some(HoldingsClass::Stocks));
        assert_eq!(parse_holdings_tag("none"), Some(HoldingsClass::None));
    }

    /// An unknown value is IGNORED here rather than refused, so the Holdings and
    /// Insights tabs still render from the mechanical default.
    ///
    /// The user is not left guessing: `wire::journal_to_tag_diagnostics` names
    /// the account, the value and the three alternatives as an `account-tag`
    /// warning. That half of the contract is pinned in
    /// `tests/tag_diagnostics.rs`, which is where the vocabularies meet the
    /// wire; all this side owes is the fallback.
    #[test]
    fn an_unknown_value_is_ignored_not_refused() {
        assert!(
            classes(&[decl("assets:house", &[("holdings", "real-estate")])]).is_empty(),
            "an unrecognised code must read as no declaration"
        );
        // ...and it must not be coerced into one of the real classes on the way.
        assert_eq!(parse_holdings_tag("real-estate"), None);
    }

    /// `; holdings:` with nothing after it names no tab — the same reading
    /// `issection:` and `bsgroup:` give an empty value.
    #[test]
    fn an_empty_value_is_no_declaration() {
        assert!(classes(&[decl("assets:house", &[("holdings", "  ")])]).is_empty());
    }

    #[test]
    fn the_class_inherits_from_the_nearest_declared_ancestor() {
        let declared = classes(&[
            decl("assets:property", &[("holdings", "other")]),
            decl("assets:property:rental", &[("holdings", "stocks")]),
        ]);

        // Own tag wins over the ancestor's.
        assert_eq!(
            resolve_holdings_class("assets:property:rental", &declared),
            Some(HoldingsClass::Stocks)
        );
        // Undeclared descendants inherit the nearest declared ancestor.
        assert_eq!(
            resolve_holdings_class("assets:property:house:land", &declared),
            Some(HoldingsClass::Other)
        );
        assert_eq!(
            resolve_holdings_class("assets:property:rental:unit1", &declared),
            Some(HoldingsClass::Stocks)
        );
        // Nothing on the path is declared.
        assert_eq!(resolve_holdings_class("assets:broker", &declared), None);
    }

    /// A prefix that is not a path SEGMENT must not match: `assets:propertyx` is
    /// not under `assets:property`.
    #[test]
    fn a_partial_segment_is_not_an_ancestor() {
        let declared = classes(&[decl("assets:property", &[("holdings", "other")])]);
        assert_eq!(resolve_holdings_class("assets:propertyx", &declared), None);
    }
}
