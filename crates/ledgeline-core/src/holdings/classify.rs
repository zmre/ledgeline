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
//! An unknown value is REFUSED, following `issection:` rather than `type:`
//! (`reports/mod.rs:97-113`). Both are closed vocabularies that decide
//! membership, and the argument recorded there transfers: a silent fallback
//! drops the account back to the mechanical default, so the user who wrote
//! `holdings: real-estate` to move their house sees it sitting on the Stocks tab
//! exactly as before, with nothing on screen to say why. A misspelt code in a
//! three-word vocabulary is a typo, and naming the alternatives is the only
//! answer that leads anywhere.

use std::collections::BTreeMap;

use crate::model::AccountDeclaration;
use crate::reports::ReportError;

/// The account-directive tag this module reads.
pub const HOLDINGS_TAG: &str = "holdings";

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
/// else is a typo, and [`declared_holdings_classes`] refuses it.
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
/// would be its own trap.
///
/// # Errors
/// Returns [`ReportError::UnknownHoldingsClass`] for a value outside the closed
/// vocabulary.
pub fn declared_holdings_classes(
    accounts: &[AccountDeclaration],
) -> Result<BTreeMap<String, HoldingsClass>, ReportError> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == HOLDINGS_TAG)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
                .map(|value| {
                    parse_holdings_tag(value)
                        .map(|class| (decl.name.0.clone(), class))
                        .ok_or_else(|| ReportError::UnknownHoldingsClass {
                            account: decl.name.0.clone(),
                            value: value.to_string(),
                        })
                })
        })
        .collect()
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
        declared_holdings_classes(decls).expect("declared classes parse")
    }

    #[test]
    fn parses_the_closed_vocabulary_case_insensitively() {
        assert_eq!(parse_holdings_tag("other"), Some(HoldingsClass::Other));
        assert_eq!(parse_holdings_tag("  Other "), Some(HoldingsClass::Other));
        assert_eq!(parse_holdings_tag("STOCKS"), Some(HoldingsClass::Stocks));
        assert_eq!(parse_holdings_tag("stock"), Some(HoldingsClass::Stocks));
        assert_eq!(parse_holdings_tag("none"), Some(HoldingsClass::None));
    }

    /// The `issection:` rule, not the `type:` rule: a membership code that falls
    /// back silently leaves the user staring at the tab they tried to move the
    /// account off.
    #[test]
    fn an_unknown_value_is_refused_by_name() {
        let err =
            declared_holdings_classes(&[decl("assets:house", &[("holdings", "real-estate")])])
                .expect_err("an unknown code is refused");
        assert_eq!(
            err,
            ReportError::UnknownHoldingsClass {
                account: "assets:house".to_string(),
                value: "real-estate".to_string(),
            }
        );
        // The message has to name the alternatives, or it leads nowhere.
        let text = err.to_string();
        assert!(text.contains("stocks"), "{text}");
        assert!(text.contains("other"), "{text}");
        assert!(text.contains("none"), "{text}");
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
