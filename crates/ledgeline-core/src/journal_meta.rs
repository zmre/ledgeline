//! Whole-journal display metadata served on `/api/journal` (plan 26 B1): the
//! one style rule reports format with, the one "dominant commodity" rule, and
//! the journal's size and date span. Computed on the engine so no client has to
//! walk every transaction to learn them.
//!
//! Both rules are ports of the TypeScript the SPA used to run over its full
//! copy of the journal, and `fixtures/journal-meta/cases.json` — captured from
//! that TypeScript before it was deleted — pins them.

use crate::model::{Amount, AmountStyle, Commodity, Journal};
use std::collections::{BTreeMap, HashMap};

/// The display style reports use per commodity.
///
/// **The rule:** the style of the commodity's FIRST posting amount (file order;
/// costs and balance assertions are not posting amounts), with its precision
/// raised to the most decimal places any posting amount of that commodity was
/// written with — raised, never lowered. When the commodity has a declared
/// style ([`Journal::commodity_styles`], the same table `/api/journal`'s
/// `commodityStyles` serves) that declared style IS the first-seen style, its
/// precision a floor exactly as the SPA's normalizer treats it.
///
/// Why raise: the first-seen style is right for rendering one posting but wrong
/// for an aggregated total — if the first `AAPL` was written `10 AAPL`, a
/// 19.5-share total would round to `20 AAPL`. Other amounts' declared
/// precisions are deliberately not consulted: cost/conversion postings carry
/// inflated precisions that hledger's own display does not use.
///
/// Commodities that are declared but never posted are absent, as before; a
/// caller asking for one falls back to the declared style itself.
#[must_use]
pub fn report_styles(journal: &Journal) -> BTreeMap<Commodity, AmountStyle> {
    let declared: HashMap<&Commodity, &AmountStyle> = journal
        .commodity_styles
        .iter()
        .map(|(commodity, style)| (commodity, style))
        .collect();
    let mut first_seen: BTreeMap<Commodity, AmountStyle> = BTreeMap::new();
    let mut max_places: HashMap<&Commodity, u32> = HashMap::new();
    for amount in posting_amounts(journal) {
        let places = amount.quantity.places;
        max_places
            .entry(&amount.commodity)
            .and_modify(|max| *max = (*max).max(places))
            .or_insert(places);
        first_seen
            .entry(amount.commodity.clone())
            .or_insert_with(|| match declared.get(&amount.commodity) {
                Some(style) => (*style).clone(),
                None => amount.style.clone(),
            });
    }
    first_seen
        .into_iter()
        .map(|(commodity, style)| {
            let max = max_places.get(&commodity).copied().unwrap_or(0);
            let precision = style.precision.max(max);
            (commodity, AmountStyle { precision, ..style })
        })
        .collect()
}

/// The journal's most-used commodity — the ONE rule for "which commodity is
/// this journal in" (the edit form's default, a chart's fallback when a view
/// has nothing of its own to rank).
///
/// **The rule:** count posting amounts per commodity, skipping the empty
/// (commodity-less) symbol; the highest count wins, a tie going to the
/// commodity seen first in file order. `$` for a journal with no posted
/// commodity at all.
///
/// Posting count rather than value, because values in different commodities
/// cannot be compared without prices, and rather than "first seen", because one
/// opening-balance posting in a foreign currency must not repaint the journal.
#[must_use]
pub fn dominant_commodity(journal: &Journal) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    let mut slot: HashMap<&str, usize> = HashMap::new();
    for amount in posting_amounts(journal) {
        let symbol = amount.commodity.0.as_str();
        if symbol.is_empty() {
            continue;
        }
        let index = *slot.entry(symbol).or_insert_with(|| {
            counts.push((symbol, 0));
            counts.len() - 1
        });
        counts[index].1 += 1;
    }
    // `max_by_key` keeps the LAST maximum; fold so the FIRST-seen wins a tie.
    counts
        .iter()
        .fold(None::<(&str, usize)>, |best, &(symbol, count)| match best {
            Some((_, best_count)) if best_count >= count => best,
            _ => Some((symbol, count)),
        })
        .map_or_else(|| "$".to_string(), |(symbol, _)| symbol.to_string())
}

/// The earliest and latest primary transaction dates, `None` for an empty
/// journal. Dates are ISO `YYYY-MM-DD`, so string order is date order.
#[must_use]
pub fn date_span(journal: &Journal) -> Option<(&str, &str)> {
    let mut dates = journal.transactions.iter().map(|txn| txn.date.as_str());
    let first = dates.next()?;
    Some(dates.fold((first, first), |(lo, hi), date| {
        (lo.min(date), hi.max(date))
    }))
}

/// Every amount the journal WROTE. A posting an `=` rule generated is left
/// out: its exact product (`$333.33 * 0.1` is `$33.333`) is not a precision
/// anyone typed, and hledger's own reports keep `$-33.33` for it (verified
/// 2026-10-28, `bal --auto`).
fn posting_amounts(journal: &Journal) -> impl Iterator<Item = &Amount> {
    journal
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .filter(|posting| posting.generated.is_none())
        .flat_map(|posting| &posting.amounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    fn journal(text: &str) -> Journal {
        parse_journal(text, "t.journal").expect("parses")
    }

    #[test]
    fn dominant_is_the_most_posted_commodity_first_seen_on_a_tie() {
        let j = journal(
            "2026-01-01 a\n    x  10 EUR\n    y  -10 EUR\n\n2026-01-02 b\n    x  $1\n    y  $-1\n",
        );
        assert_eq!(dominant_commodity(&j), "EUR");
        let j =
            journal("2026-01-01 a\n    x  10 EUR\n    y\n\n2026-01-02 b\n    x  $1\n    y  $-1\n");
        // `y`'s elided amount is inferred as EUR, so EUR still has two.
        assert_eq!(dominant_commodity(&j), "EUR");
    }

    #[test]
    fn dominant_defaults_to_dollars_and_skips_bare_numbers() {
        assert_eq!(dominant_commodity(&journal("")), "$");
        assert_eq!(
            dominant_commodity(&journal("2026-01-01 a\n    x  10\n    y  -10\n")),
            "$"
        );
    }

    #[test]
    fn report_style_precision_is_raised_to_the_widest_amount() {
        let j = journal(
            "2026-01-01 a\n    x  10 AAPL\n    y  -10 AAPL\n\n2026-01-02 b\n    x  9.5 AAPL\n    y  -9.5 AAPL\n",
        );
        let styles = report_styles(&j);
        assert_eq!(styles[&Commodity("AAPL".into())].precision, 1);
    }

    #[test]
    fn date_span_is_min_and_max_not_file_order() {
        let j = journal("2026-03-01 a\n    x  $1\n    y\n\n2026-01-15 b\n    x  $1\n    y\n");
        assert_eq!(date_span(&j), Some(("2026-01-15", "2026-03-01")));
        assert_eq!(date_span(&journal("")), None);
    }
}
