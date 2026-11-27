//! Intraday valuation (plan 47 D10/D11): today's point priced with a delayed
//! market quote instead of the journal's last close.
//!
//! The quotes themselves live in the server's memory, never in a journal. All
//! this module does is turn the ones dated today into `P` directives that a
//! caller APPENDS to the journal's own price list for one computation, so
//! every Holdings number — table, totals, gains, the chart's last point — is
//! the ordinary engine run over a slightly longer price list. Appending is
//! what makes a quote win: the price lookups break a same-date tie in favour
//! of the LAST directive declared, so a live quote overrides a close already
//! recorded for today, and nothing dated before today can see it at all.

use std::path::PathBuf;

use crate::decimal::Dec;
use crate::model::{Amount, Commodity, PriceDirective};

use super::commodities::is_us_dollar;

/// One cached quote, as the caller has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveQuote {
    /// The journal's commodity symbol (not the source's ticker).
    pub symbol: String,
    /// Per-unit price, in `currency`.
    pub price: Dec,
    /// The exchange's calendar date for the quote, `YYYY-MM-DD`.
    pub date: String,
    /// The quote's ISO currency (`USD`), when the source said.
    pub currency: Option<String>,
}

/// Where a live directive says it came from. Never a real file: nothing that
/// reads `source_file` may ever be pointed at one to write.
const LIVE_SOURCE: &str = "<live quote>";

/// The `P` directives that price `quotes` dated `today` — to be appended
/// after `prices` (see the module docs).
///
/// Each symbol is priced in the commodity the journal ALREADY prices it in,
/// from its latest `P` directive dated on or before `today`, exactly the way
/// the Update prices button records a close. A symbol the journal has never
/// priced is skipped: there is no established commodity to put it in, and
/// guessing one could change what the whole report is valued in.
///
/// A quote whose currency visibly disagrees with that commodity is skipped
/// too, rather than written down as the wrong money: `USD` fits any spelling
/// of the dollar ([`is_us_dollar`]), anything else must name the commodity
/// exactly. A quote with no currency is trusted, as the button trusts it.
#[must_use]
pub fn live_price_directives(
    prices: &[PriceDirective],
    quotes: &[LiveQuote],
    today: &str,
) -> Vec<PriceDirective> {
    quotes
        .iter()
        .filter(|quote| quote.date == today)
        .filter_map(|quote| {
            let template = latest_price(prices, &quote.symbol, today)?;
            let target = &template.price.commodity;
            currency_fits(quote.currency.as_deref(), &target.0).then(|| PriceDirective {
                date: today.to_string(),
                commodity: Commodity(quote.symbol.clone()),
                price: Amount {
                    commodity: target.clone(),
                    quantity: quote.price,
                    style: template.price.style.clone(),
                    cost: None,
                },
                source_file: PathBuf::from(LIVE_SOURCE),
            })
        })
        .collect()
}

/// The latest directive pricing `symbol` dated `<= today`; on a date tie the
/// one declared last, matching the engine's own lookups.
fn latest_price<'a>(
    prices: &'a [PriceDirective],
    symbol: &str,
    today: &str,
) -> Option<&'a PriceDirective> {
    prices
        .iter()
        .filter(|price| price.commodity.0 == symbol && price.date.as_str() <= today)
        .fold(None, |best: Option<&PriceDirective>, price| match best {
            Some(best) if best.date > price.date => Some(best),
            _ => Some(price),
        })
}

fn currency_fits(currency: Option<&str>, target: &str) -> bool {
    match currency {
        None => true,
        Some(currency) => {
            currency == target || (currency.eq_ignore_ascii_case("USD") && is_us_dollar(target))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::test_helpers::pd;

    fn quote(symbol: &str, cents: i128, date: &str, currency: Option<&str>) -> LiveQuote {
        LiveQuote {
            symbol: symbol.to_string(),
            price: Dec::new(cents, 2),
            date: date.to_string(),
            currency: currency.map(str::to_string),
        }
    }

    #[test]
    fn a_quote_dated_today_is_priced_in_the_journals_own_commodity() {
        let prices = vec![pd("2026-11-20", "VTI", 30000, "$")];
        let live = live_price_directives(
            &prices,
            &[quote("VTI", 29500, "2026-11-26", Some("USD"))],
            "2026-11-26",
        );
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].date, "2026-11-26");
        assert_eq!(live[0].commodity.0, "VTI");
        assert_eq!(live[0].price.commodity.0, "$");
        assert_eq!(live[0].price.quantity, Dec::new(29500, 2));
    }

    #[test]
    fn yesterdays_quote_and_a_never_priced_symbol_are_skipped() {
        let prices = vec![pd("2026-11-20", "VTI", 30000, "$")];
        let live = live_price_directives(
            &prices,
            &[
                quote("VTI", 29500, "2026-11-25", Some("USD")),
                quote("AAPL", 20000, "2026-11-26", Some("USD")),
            ],
            "2026-11-26",
        );
        assert!(live.is_empty());
    }

    #[test]
    fn a_quote_in_the_wrong_currency_is_never_written_down_as_the_journals() {
        let prices = vec![
            pd("2026-11-20", "VOD", 7000, "GBP"),
            pd("2026-11-20", "SAP", 20000, "EUR"),
        ];
        let live = live_price_directives(
            &prices,
            &[
                quote("VOD", 7100, "2026-11-26", Some("GBp")),
                quote("SAP", 20100, "2026-11-26", Some("EUR")),
            ],
            "2026-11-26",
        );
        let symbols: Vec<&str> = live.iter().map(|p| p.commodity.0.as_str()).collect();
        assert_eq!(symbols, ["SAP"]);
    }

    #[test]
    fn the_latest_directive_decides_the_commodity() {
        let prices = vec![
            pd("2026-01-02", "VTI", 25000, "EUR"),
            pd("2026-11-20", "VTI", 30000, "$"),
            pd("2026-12-31", "VTI", 1, "GBP"), // after today: not a template
        ];
        let live = live_price_directives(
            &prices,
            &[quote("VTI", 29500, "2026-11-26", None)],
            "2026-11-26",
        );
        assert_eq!(live[0].price.commodity.0, "$");
    }
}
