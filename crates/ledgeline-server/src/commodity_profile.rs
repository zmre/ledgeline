//! How one commodity divides across each "by category" dimension of the
//! Holdings pie — the pure merge of the journal's commodity tags over what
//! Yahoo Finance reports.
//!
//! # The rule
//!
//! **A tag wins, whole.** `commodity VTI  ; sector: Technology` makes VTI 100%
//! Technology in the Sector view, whatever Yahoo says, and nothing else about
//! VTI changes. Yahoo only ever fills a dimension the journal left untagged.
//!
//! # What a breakdown is
//!
//! Per dimension, a list of `(label, weight)` with weights in `(0, 1]` summing
//! to AT MOST one. The shortfall is "unclassified" — the SPA draws it as its
//! own muted slice — so an unknown is never silently spread across the known
//! labels, and a holding nothing is known about is 100% unclassified.
//!
//! | Dimension     | Tag          | From Yahoo                                                                 |
//! |---------------|--------------|----------------------------------------------------------------------------|
//! | Asset class   | `assetclass` | a fund's position split; else implied by the security type (a stock is equity) |
//! | Sector        | `sector`     | a stock's sector; a fund's sector weights × its equity share, the non-equity share by asset class |
//! | Industry      | `industry`   | a stock's industry (funds: none — Yahoo has no fund industry split)         |
//! | Security type | `type`       | `quoteType`                                                                 |
//! | Category      | `category`   | a fund's Morningstar category (`Large Blend`); stocks have none            |
//! | Risk          | `risk`       | a fund's Morningstar risk rating (Low … High); stocks have none             |
//!
//! `type` rather than a new tag name because it is ALREADY how journals label
//! a commodity's kind — `commodity NAWGX ; type:mutualfund` is in the
//! corpus this engine is tested against — so the view reads what those
//! journals already say.
//!
//! The sector rule is the one non-obvious row. A 60/40 balanced fund has
//! sector weights only for its 60% equity sleeve; spreading them over the whole
//! fund would call bonds "Technology". So the equity share is split by sector
//! and the rest is labelled by its asset class ("Bonds", "Cash"), which keeps
//! the Sector pie summing to the portfolio and says what the non-equity part is.

use crate::yahoo_profile::YahooProfile;
use serde::Serialize;

/// The pie's category dimensions, in the order the SPA offers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dimension {
    AssetClass,
    Sector,
    Industry,
    SecurityType,
    Category,
    Risk,
}

impl Dimension {
    /// The commodity tag that overrides this dimension.
    pub(crate) fn tag(self) -> &'static str {
        match self {
            Self::AssetClass => "assetclass",
            Self::Sector => "sector",
            Self::Industry => "industry",
            Self::SecurityType => "type",
            Self::Category => "category",
            Self::Risk => "risk",
        }
    }

    pub(crate) const ALL: [Self; 6] = [
        Self::AssetClass,
        Self::Sector,
        Self::Industry,
        Self::SecurityType,
        Self::Category,
        Self::Risk,
    ];
}

/// One label's share of a holding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Weight {
    pub(crate) label: String,
    pub(crate) weight: f64,
}

fn weight(label: impl Into<String>, weight: f64) -> Weight {
    Weight {
        label: label.into(),
        weight,
    }
}

/// Every dimension's breakdown for one commodity.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Breakdown {
    pub(crate) asset_class: Vec<Weight>,
    pub(crate) sector: Vec<Weight>,
    pub(crate) industry: Vec<Weight>,
    pub(crate) security_type: Vec<Weight>,
    pub(crate) category: Vec<Weight>,
    pub(crate) risk: Vec<Weight>,
}

/// Canonical asset-class labels. `Other assets`, not `Other`, because the pie
/// already has an `(other)` slice meaning "the folded tail", and the two must
/// never read as the same thing.
pub(crate) const EQUITY: &str = "Equity";
const BONDS: &str = "Bonds";
const CASH: &str = "Cash";
const OTHER_ASSETS: &str = "Other assets";
const CRYPTO: &str = "Crypto";

/// Whether `tags` already answer every dimension, in which case Yahoo has
/// nothing to add and is not asked.
pub(crate) fn fully_tagged(tags: &[(String, String)]) -> bool {
    Dimension::ALL
        .iter()
        .all(|dimension| tag_value(tags, dimension.tag()).is_some())
}

/// The first non-blank value of `key`, trimmed.
fn tag_value<'a>(tags: &'a [(String, String)], key: &str) -> Option<&'a str> {
    tags.iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.trim())
        .find(|value| !value.is_empty())
}

/// Merge `tags` over `yahoo` into every dimension's breakdown.
pub(crate) fn resolve(tags: &[(String, String)], yahoo: Option<&YahooProfile>) -> Breakdown {
    let tag = |dimension: Dimension| tag_value(tags, dimension.tag());
    let single = |label: String| vec![weight(label, 1.0)];
    let fetched_single = |value: Option<String>| value.map(single).unwrap_or_default();

    // Security type first: the asset-class fallback reads it.
    let security_type = match tag(Dimension::SecurityType) {
        Some(value) => single(security_type_label(value)),
        None => fetched_single(
            yahoo
                .and_then(|profile| profile.quote_type.as_deref())
                .map(quote_type_label),
        ),
    };

    // Asset class. A fund's position split beats the implication of its type;
    // the implication (a stock is equity) beats nothing.
    let asset_class = match tag(Dimension::AssetClass) {
        Some(value) => single(asset_class_label(value)),
        None => {
            let mix = yahoo
                .and_then(|profile| profile.asset_mix)
                .map(mix_weights)
                .unwrap_or_default();
            if mix.is_empty() {
                security_type
                    .first()
                    .and_then(|entry| implied_asset_class(&entry.label))
                    .map(|label| single(label.to_string()))
                    .unwrap_or_default()
            } else {
                mix
            }
        }
    };

    let sector = match tag(Dimension::Sector) {
        Some(value) => single(value.to_string()),
        None => derived_sector(yahoo, &asset_class),
    };
    let industry = match tag(Dimension::Industry) {
        Some(value) => single(value.to_string()),
        None => fetched_single(yahoo.and_then(|profile| profile.industry.clone())),
    };
    let category = match tag(Dimension::Category) {
        Some(value) => single(value.to_string()),
        None => fetched_single(yahoo.and_then(|profile| profile.category.clone())),
    };
    let risk = match tag(Dimension::Risk) {
        Some(value) => single(value.to_string()),
        None => fetched_single(
            yahoo
                .and_then(|profile| profile.risk_rating)
                .and_then(risk_label)
                .map(str::to_string),
        ),
    };

    Breakdown {
        asset_class,
        sector,
        industry,
        security_type,
        category,
        risk,
    }
}

/// A fund's position split as asset-class weights: negatives (a leveraged
/// fund's borrowed cash) clamped to zero, the rest scaled to sum to one, zeros
/// dropped, in a fixed order.
fn mix_weights(mix: crate::yahoo_profile::AssetMix) -> Vec<Weight> {
    let parts = [
        (EQUITY, mix.equity),
        (BONDS, mix.bond),
        (CASH, mix.cash),
        (OTHER_ASSETS, mix.other),
    ]
    .map(|(label, value)| {
        (
            label,
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            },
        )
    });
    let total: f64 = parts.iter().map(|(_, value)| value).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    parts
        .into_iter()
        .filter(|(_, value)| *value > 0.0)
        .map(|(label, value)| weight(label, value / total))
        .collect()
}

/// The Sector breakdown when no `sector:` tag gives one: the equity share
/// split by Yahoo's sector data, every other asset class labelled as itself
/// (see the module docs). With no asset class known, Yahoo's sectors are taken
/// as the whole holding.
fn derived_sector(yahoo: Option<&YahooProfile>, asset_class: &[Weight]) -> Vec<Weight> {
    let sectors: Vec<Weight> = match yahoo {
        Some(YahooProfile {
            sector: Some(sector),
            ..
        }) => vec![weight(sector.clone(), 1.0)],
        Some(profile) => {
            let total: f64 = profile.sector_weights.iter().map(|(_, w)| w).sum();
            if total > 0.0 {
                profile
                    .sector_weights
                    .iter()
                    .map(|(label, w)| weight(label.clone(), w / total))
                    .collect()
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    };
    if asset_class.is_empty() {
        return sectors;
    }
    let equity_share: f64 = asset_class
        .iter()
        .filter(|entry| entry.label == EQUITY)
        .map(|entry| entry.weight)
        .sum();
    let equity = sectors
        .into_iter()
        .map(|entry| weight(entry.label, entry.weight * equity_share))
        .filter(|entry| entry.weight > 0.0);
    let rest = asset_class
        .iter()
        .filter(|entry| entry.label != EQUITY)
        .cloned();
    equity.chain(rest).collect()
}

/// A Yahoo `quoteType` as a reader would say it: sentence case (`INDEX` →
/// `Index`), except the types whose reading is not their code.
fn quote_type_label(quote_type: &str) -> String {
    match quote_type.to_ascii_uppercase().as_str() {
        "EQUITY" => "Stock".to_string(),
        "ETF" => "ETF".to_string(),
        "MUTUALFUND" => "Mutual fund".to_string(),
        "MONEYMARKET" => "Money market".to_string(),
        "CRYPTOCURRENCY" => "Crypto".to_string(),
        _ => sentence_case(quote_type),
    }
}

/// A `type:` tag's value, normalized where it names a kind Yahoo also reports
/// (so `type:mutualfund` and a fetched `MUTUALFUND` are one slice), else kept
/// as written.
fn security_type_label(value: &str) -> String {
    match squash(value).as_str() {
        "stock" | "stocks" | "equity" | "commonstock" | "share" | "shares" => "Stock".to_string(),
        "etf" => "ETF".to_string(),
        "mutualfund" | "fund" | "mf" => "Mutual fund".to_string(),
        "moneymarket" | "mmf" => "Money market".to_string(),
        "bond" | "bonds" => "Bond".to_string(),
        "crypto" | "cryptocurrency" => "Crypto".to_string(),
        "currency" => "Currency".to_string(),
        "index" => "Index".to_string(),
        _ => value.to_string(),
    }
}

/// An `assetclass:` tag's value, normalized onto the four classes Yahoo
/// reports where it plainly means one of them, else kept as written (real
/// estate, commodities, private equity — the tag is open-ended on purpose).
fn asset_class_label(value: &str) -> String {
    match squash(value).as_str() {
        "equity" | "equities" | "stock" | "stocks" => EQUITY.to_string(),
        "bond" | "bonds" | "fixedincome" => BONDS.to_string(),
        "cash" | "moneymarket" | "cashequivalents" => CASH.to_string(),
        "other" | "otherassets" => OTHER_ASSETS.to_string(),
        "crypto" | "cryptocurrency" => CRYPTO.to_string(),
        _ => value.to_string(),
    }
}

/// The asset class a security type implies on its own, when nothing better is
/// known. Funds imply nothing: a fund can hold anything.
fn implied_asset_class(security_type: &str) -> Option<&'static str> {
    match security_type {
        "Stock" => Some(EQUITY),
        "Bond" => Some(BONDS),
        "Money market" | "Currency" => Some(CASH),
        "Crypto" => Some(CRYPTO),
        _ => None,
    }
}

/// Morningstar's five risk bands, as Morningstar words them.
fn risk_label(rating: u8) -> Option<&'static str> {
    match rating {
        1 => Some("Low"),
        2 => Some("Below average"),
        3 => Some("Average"),
        4 => Some("Above average"),
        5 => Some("High"),
        _ => None,
    }
}

/// Lower-case with spaces, hyphens and underscores removed, for matching
/// `Mutual Fund`, `mutual-fund` and `mutualfund` as one word.
fn squash(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn sentence_case(value: &str) -> String {
    let lower = value.to_lowercase();
    let mut chars = lower.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yahoo_profile::AssetMix;

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn labels(weights: &[Weight]) -> Vec<(&str, f64)> {
        weights
            .iter()
            .map(|w| (w.label.as_str(), (w.weight * 10_000.0).round() / 10_000.0))
            .collect()
    }

    fn sum(weights: &[Weight]) -> f64 {
        weights.iter().map(|w| w.weight).sum()
    }

    fn stock() -> YahooProfile {
        YahooProfile {
            quote_type: Some("EQUITY".to_string()),
            sector: Some("Technology".to_string()),
            industry: Some("Consumer Electronics".to_string()),
            ..YahooProfile::default()
        }
    }

    fn balanced_fund() -> YahooProfile {
        YahooProfile {
            quote_type: Some("MUTUALFUND".to_string()),
            category: Some("Moderate Allocation".to_string()),
            asset_mix: Some(AssetMix {
                equity: 0.6,
                bond: 0.35,
                cash: 0.05,
                other: 0.0,
            }),
            sector_weights: vec![
                ("Technology".to_string(), 0.5),
                ("Healthcare".to_string(), 0.5),
            ],
            risk_rating: Some(3),
            ..YahooProfile::default()
        }
    }

    #[test]
    fn a_stock_from_yahoo_is_all_equity_and_its_own_sector() {
        let resolved = resolve(&[], Some(&stock()));
        let b = &resolved;
        assert_eq!(labels(&b.asset_class), vec![("Equity", 1.0)]);
        assert_eq!(labels(&b.sector), vec![("Technology", 1.0)]);
        assert_eq!(labels(&b.industry), vec![("Consumer Electronics", 1.0)]);
        assert_eq!(labels(&b.security_type), vec![("Stock", 1.0)]);
        assert!(b.category.is_empty(), "stocks have no category");
        assert!(b.risk.is_empty());
    }

    #[test]
    fn a_balanced_funds_sectors_cover_only_its_equity_share() {
        let b = resolve(&[], Some(&balanced_fund()));
        assert_eq!(
            labels(&b.asset_class),
            vec![("Equity", 0.6), ("Bonds", 0.35), ("Cash", 0.05)]
        );
        assert_eq!(
            labels(&b.sector),
            vec![
                ("Technology", 0.3),
                ("Healthcare", 0.3),
                ("Bonds", 0.35),
                ("Cash", 0.05)
            ]
        );
        assert!(
            (sum(&b.sector) - 1.0).abs() < 1e-9,
            "sums to the whole fund"
        );
        assert!(b.industry.is_empty(), "funds have no industry split");
        assert_eq!(labels(&b.category), vec![("Moderate Allocation", 1.0)]);
        assert_eq!(labels(&b.risk), vec![("Average", 1.0)]);
        assert_eq!(labels(&b.security_type), vec![("Mutual fund", 1.0)]);
    }

    #[test]
    fn a_mix_is_normalized_and_negative_cash_is_clamped() {
        let profile = YahooProfile {
            asset_mix: Some(AssetMix {
                equity: 1.5,
                bond: 0.0,
                cash: -0.5,
                other: 0.5,
            }),
            ..YahooProfile::default()
        };
        let b = resolve(&[], Some(&profile));
        assert_eq!(
            labels(&b.asset_class),
            vec![("Equity", 0.75), ("Other assets", 0.25)]
        );
    }

    #[test]
    fn sector_weights_that_do_not_sum_to_one_are_scaled() {
        let profile = YahooProfile {
            sector_weights: vec![("Energy".to_string(), 0.3), ("Utilities".to_string(), 0.1)],
            ..YahooProfile::default()
        };
        let b = resolve(&[], Some(&profile));
        assert_eq!(
            labels(&b.sector),
            vec![("Energy", 0.75), ("Utilities", 0.25)]
        );
    }

    #[test]
    fn a_bond_fund_is_fully_answered_by_its_asset_class_in_the_sector_view() {
        let profile = YahooProfile {
            quote_type: Some("ETF".to_string()),
            asset_mix: Some(AssetMix {
                equity: 0.0,
                bond: 0.98,
                cash: 0.02,
                other: 0.0,
            }),
            ..YahooProfile::default()
        };
        let b = resolve(&[], Some(&profile));
        assert_eq!(labels(&b.sector), vec![("Bonds", 0.98), ("Cash", 0.02)]);
    }

    #[test]
    fn an_equity_fund_with_no_sector_data_leaves_its_equity_unclassified() {
        let profile = YahooProfile {
            asset_mix: Some(AssetMix {
                equity: 0.9,
                bond: 0.0,
                cash: 0.1,
                other: 0.0,
            }),
            ..YahooProfile::default()
        };
        let b = resolve(&[], Some(&profile));
        assert_eq!(labels(&b.sector), vec![("Cash", 0.1)]);
    }

    #[test]
    fn a_tag_overrides_its_dimension_whole_and_nothing_else() {
        let resolved = resolve(
            &tags(&[("sector", "Diversified"), ("category", " Target Date ")]),
            Some(&balanced_fund()),
        );
        let b = &resolved;
        assert_eq!(labels(&b.sector), vec![("Diversified", 1.0)]);
        assert_eq!(labels(&b.category), vec![("Target Date", 1.0)]);
        // Untagged dimensions still come from Yahoo.
        assert_eq!(b.asset_class.len(), 3);
    }

    #[test]
    fn an_assetclass_tag_reshapes_the_yahoo_sector_view() {
        // Told it is all bonds, a stock's Technology sector no longer applies.
        let b = resolve(&tags(&[("assetclass", "fixed income")]), Some(&stock()));
        assert_eq!(labels(&b.asset_class), vec![("Bonds", 1.0)]);
        assert_eq!(labels(&b.sector), vec![("Bonds", 1.0)]);
    }

    #[test]
    fn tags_alone_classify_with_no_yahoo_at_all() {
        let resolved = resolve(
            &tags(&[("type", "mutualfund"), ("assetclass", "Real Estate")]),
            None,
        );
        let b = &resolved;
        assert_eq!(labels(&b.security_type), vec![("Mutual fund", 1.0)]);
        assert_eq!(labels(&b.asset_class), vec![("Real Estate", 1.0)]);
        // A non-equity class answers the Sector view as itself, from the tag.
        assert_eq!(labels(&b.sector), vec![("Real Estate", 1.0)]);
    }

    #[test]
    fn a_type_tag_implies_an_asset_class_and_counts_as_a_tag() {
        let resolved = resolve(&tags(&[("type", "Stock")]), None);
        assert_eq!(labels(&resolved.asset_class), vec![("Equity", 1.0)]);
        let crypto = resolve(&tags(&[("type", "crypto")]), None);
        assert_eq!(labels(&crypto.asset_class), vec![("Crypto", 1.0)]);
    }

    #[test]
    fn a_type_tag_beats_yahoo_quote_type() {
        let b = resolve(&tags(&[("type", "etf")]), Some(&stock()));
        assert_eq!(labels(&b.security_type), vec![("ETF", 1.0)]);
        // ETF implies nothing, and Yahoo gave no mix: asset class unknown.
        assert!(b.asset_class.is_empty());
    }

    #[test]
    fn nothing_known_is_all_empty_with_no_source() {
        let resolved = resolve(&tags(&[("name", "Private fund")]), None);
        assert_eq!(resolved, Breakdown::default());
    }

    #[test]
    fn blank_tag_values_are_ignored() {
        let b = resolve(&tags(&[("sector", "  ")]), Some(&stock()));
        assert_eq!(labels(&b.sector), vec![("Technology", 1.0)]);
    }

    #[test]
    fn unknown_labels_are_kept_as_written() {
        assert_eq!(security_type_label("REIT"), "REIT");
        assert_eq!(asset_class_label("Private equity"), "Private equity");
        assert_eq!(asset_class_label("Stocks"), "Equity");
        assert_eq!(quote_type_label("ECNQUOTE"), "Ecnquote");
    }

    #[test]
    fn fully_tagged_needs_every_dimension() {
        let all = tags(&[
            ("assetclass", "equity"),
            ("sector", "Tech"),
            ("industry", "Chips"),
            ("type", "stock"),
            ("category", "Growth"),
            ("risk", "High"),
        ]);
        assert!(fully_tagged(&all));
        assert!(!fully_tagged(&all[1..]));
    }

    #[test]
    fn every_breakdown_sums_to_at_most_one() {
        for profile in [stock(), balanced_fund(), YahooProfile::default()] {
            let b = resolve(&[], Some(&profile));
            for weights in [
                &b.asset_class,
                &b.sector,
                &b.industry,
                &b.security_type,
                &b.category,
                &b.risk,
            ] {
                assert!(sum(weights) <= 1.0 + 1e-9);
                assert!(weights.iter().all(|w| w.weight > 0.0));
            }
        }
    }

    #[test]
    fn a_quote_type_reads_in_sentence_case_unless_its_code_is_not_its_name() {
        assert_eq!(quote_type_label("INDEX"), "Index");
        assert_eq!(quote_type_label("CURRENCY"), "Currency");
        assert_eq!(quote_type_label("EQUITY"), "Stock");
        assert_eq!(quote_type_label("etf"), "ETF");
        assert_eq!(quote_type_label("MUTUALFUND"), "Mutual fund");
    }
}
