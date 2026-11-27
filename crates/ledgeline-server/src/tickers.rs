//! **Which Yahoo ticker a journal commodity is looked up as**, and whether
//! what Yahoo answered is what the journal meant (plan 48 C2–C4).
//!
//! One mapping for every caller that asks Yahoo about a commodity — Update
//! prices, the automatic updater, the intraday refresh and the category
//! profiles — so a commodity is never priced as one instrument and
//! classified as another.
//!
//! # Resolution (C2, C3)
//!
//! 1. `yahoo: X` → `X`. Always wins.
//! 2. A commodity tagged `type: crypto` or `assetclass: crypto` → Yahoo's own
//!    pair in the base currency, `<SYMBOL>-<ISO>`: `BTC-USD` in a `$` ledger,
//!    `BTC-EUR` in a `€` one. Never converted through the journal's FX prices.
//!    A base with no single ISO code (`¥`) gets no pair: the bare symbol, and a
//!    note saying so.
//! 3. Anything else → the symbol itself. An untagged symbol never gets a
//!    suffix: `BTC` may be a deliberate holding of the Grayscale ETF.
//!
//! # The guard (C4)
//!
//! Every chart reply says what the instrument is
//! ([`crate::yahoo::Instrument`]). [`judge`] reads it:
//!
//! - a coin-tagged commodity answered by anything but a coin is REFUSED (no
//!   `P` line, no intraday quote) and reported;
//! - an untagged commodity whose symbol is a well-known coin's
//!   ([`WELL_KNOWN_COINS`]), answered by something that is not a coin, is
//!   priced as before — it may really be the ETF — and WARNED about, naming
//!   what it was priced as and how to say "the coin".

use ledgeline_core::holdings::{HoldingsScope, ScopeMode, currency_iso_code, valuation_base};
use ledgeline_core::model::{Commodity, Journal};
use std::collections::BTreeSet;

use crate::prices_api::WirePriceSymbol;
use crate::yahoo::Instrument;

/// The commodity tag naming a Yahoo ticker outright.
const YAHOO_TAG: &str = "yahoo";

/// The coin symbols most likely to collide with an unrelated listing on
/// Yahoo (`BTC`, `ETH` and `XRP` are ETFs there, `USDC` an OTC stock). An
/// untagged commodity on this list that Yahoo answers with a non-coin is
/// warned about. Short on purpose: a long tail of token symbols would warn
/// about ordinary stocks.
pub(crate) const WELL_KNOWN_COINS: &[&str] = &[
    "BTC", "ETH", "USDT", "USDC", "SOL", "XRP", "BNB", "DOGE", "ADA", "TRX", "AVAX", "LINK", "DOT",
    "LTC", "BCH", "XLM", "SHIB", "MATIC", "POL", "TON", "UNI",
];

/// `tags`' first non-blank value for `key`, trimmed.
fn tag<'a>(tags: &'a [(String, String)], key: &str) -> Option<&'a str> {
    tags.iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.trim())
        .find(|value| !value.is_empty())
}

/// Whether `tags` call the commodity a coin: `type: crypto` or
/// `assetclass: crypto` (`cryptocurrency` too, any case).
#[must_use]
pub(crate) fn is_crypto_tagged(tags: &[(String, String)]) -> bool {
    ["type", "assetclass"].iter().any(|key| {
        tag(tags, key).is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "crypto" | "cryptocurrency"
            )
        })
    })
}

/// Every tag on `symbol`'s `commodity` directives, in journal order.
fn tags_of<'a>(
    commodity_tags: &'a [(Commodity, Vec<(String, String)>)],
    symbol: &str,
) -> Vec<&'a (String, String)> {
    commodity_tags
        .iter()
        .filter(|(commodity, _)| commodity.0 == symbol)
        .flat_map(|(_, tags)| tags.iter())
        .collect()
}

/// Resolves commodities to Yahoo tickers against one journal's tags and one
/// base currency.
pub(crate) struct Tickers<'a> {
    commodity_tags: &'a [(Commodity, Vec<(String, String)>)],
    /// The valuation base, when known. Only a coin-tagged commodity with no
    /// `yahoo:` tag ever reads it.
    base: Option<String>,
}

impl<'a> Tickers<'a> {
    /// Resolve against `base`, which the caller already knows (from the
    /// holdings report it computed).
    pub(crate) fn with_base(journal: &'a Journal, base: &str) -> Self {
        Self {
            commodity_tags: &journal.commodity_tags,
            base: Some(base.to_string()),
        }
    }

    /// Resolve against the base Update prices would use, as of `as_of` —
    /// [`valuation_base`] over the whole journal, unscoped, exactly the
    /// report [`crate::prices_api::symbols_to_price`] computes. Computed only
    /// when some commodity actually needs it (a coin tag and no `yahoo:`
    /// tag), so a journal without coins pays nothing.
    pub(crate) fn for_journal(journal: &'a Journal, as_of: &str) -> Self {
        let needs_base = journal
            .commodity_tags
            .iter()
            .any(|(_, tags)| is_crypto_tagged(tags) && tag(tags, YAHOO_TAG).is_none());
        let base = needs_base
            .then(|| {
                let scope = HoldingsScope {
                    accounts: BTreeSet::new(),
                    mode: ScopeMode::Include,
                    as_of: as_of.to_string(),
                    gain_since: None,
                    value_in: None,
                    selection: Default::default(),
                    return_income: Default::default(),
                };
                valuation_base(
                    &journal.transactions,
                    &journal.prices,
                    &journal.accounts,
                    &scope,
                )
                .ok()
            })
            .flatten()
            .map(|base| base.0);
        Self {
            commodity_tags: &journal.commodity_tags,
            base,
        }
    }

    /// `symbol`'s ticker and what the guard needs to know about it.
    pub(crate) fn resolve(&self, symbol: &str) -> WirePriceSymbol {
        let tags: Vec<(String, String)> = tags_of(self.commodity_tags, symbol)
            .into_iter()
            .cloned()
            .collect();
        let crypto = is_crypto_tagged(&tags);
        let iso = self.base.as_deref().and_then(currency_iso_code);
        let pair = iso.map(|iso| format!("{symbol}-{iso}"));
        let explicit = tag(&tags, YAHOO_TAG).map(str::to_string);
        let (yahoo_ticker, note) = match (explicit.clone(), crypto, pair.clone()) {
            (Some(ticker), _, _) => (ticker, None),
            (None, true, Some(pair)) => (pair, None),
            (None, true, None) => (
                symbol.to_string(),
                Some(format!(
                    "{symbol} is tagged as a coin, but the base currency {} has no Yahoo \
                     pair Ledgeline can name, so it was looked up as {symbol}. Add a \
                     yahoo: tag naming the pair, for example yahoo: {symbol}-USD.",
                    self.base.as_deref().unwrap_or("(none)")
                )),
            ),
            (None, false, _) => (symbol.to_string(), None),
        };
        WirePriceSymbol {
            symbol: symbol.to_string(),
            yahoo_ticker,
            crypto,
            explicit: explicit.is_some(),
            coin_pair: pair.unwrap_or_else(|| format!("{symbol}-USD")),
            note,
        }
    }
}

/// What to do with a chart reply for `symbol`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Record it.
    Accept,
    /// Record it, and say this.
    Warn(String),
    /// Do not record it; say this instead.
    Refuse(String),
}

/// The guard (C4): is `instrument` — what Yahoo answered `symbol`'s ticker
/// with — what the journal meant? A reply that does not say what it is
/// (`None`, or no `instrumentType`) is accepted: there is nothing to judge.
#[must_use]
pub(crate) fn judge(symbol: &WirePriceSymbol, instrument: Option<&Instrument>) -> Verdict {
    let Some(instrument) = instrument.filter(|instrument| instrument.instrument_type.is_some())
    else {
        return Verdict::Accept;
    };
    if instrument.is_crypto() {
        return Verdict::Accept;
    }
    let name = &symbol.symbol;
    if symbol.crypto {
        return Verdict::Refuse(format!(
            "{name} is tagged as a coin, but Yahoo's {} is {}, not a coin, so its price was \
             not recorded. Give commodity {name} the coin's pair, for example yahoo: {}.",
            symbol.yahoo_ticker,
            instrument.describe(),
            symbol.coin_pair,
        ));
    }
    if !symbol.explicit && WELL_KNOWN_COINS.contains(&name.as_str()) {
        return Verdict::Warn(format!(
            "{name} priced as {}. If you hold the coin, add type: crypto or yahoo: {} to \
             commodity {name}.",
            instrument.describe(),
            symbol.coin_pair,
        ));
    }
    Verdict::Accept
}

/// Whether `symbol` is a coin: tagged as one, or Yahoo said so the last time
/// its ticker was fetched (`remembered`). What gives it weekend closes
/// (plan 48 C9).
#[must_use]
pub(crate) fn is_coin(symbol: &WirePriceSymbol, remembered: Option<&Instrument>) -> bool {
    symbol.crypto || remembered.is_some_and(Instrument::is_crypto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::parse::parse_journal;

    fn journal(text: &str) -> Journal {
        parse_journal(text, "test.journal").expect("a journal")
    }

    const COINS: &str = "\
commodity 1.00000000 BTC  ; type: crypto
commodity 1.00 USDC  ; assetclass: Crypto
commodity 1.00 UNI  ; type: crypto, yahoo: UNI7083-USD
commodity 1.0000 VTI
commodity 1.0000 ETH

2026-01-05 buy
    assets:broker    1 BTC @ $60000.00
    assets:cash
";

    fn instrument(kind: &str, name: &str) -> Instrument {
        Instrument {
            instrument_type: Some(kind.to_string()),
            name: Some(name.to_string()),
            exchange: Some("NYSEArca".to_string()),
        }
    }

    #[test]
    fn a_yahoo_tag_wins_a_coin_tag_gets_the_base_pair_anything_else_is_itself() {
        let journal = journal(COINS);
        let tickers = Tickers::with_base(&journal, "$");
        assert_eq!(tickers.resolve("UNI").yahoo_ticker, "UNI7083-USD");
        assert_eq!(tickers.resolve("BTC").yahoo_ticker, "BTC-USD");
        assert_eq!(tickers.resolve("USDC").yahoo_ticker, "USDC-USD");
        assert_eq!(tickers.resolve("VTI").yahoo_ticker, "VTI");
        assert_eq!(
            tickers.resolve("ETH").yahoo_ticker,
            "ETH",
            "never a suffix untagged"
        );
        assert_eq!(tickers.resolve("NOPE").yahoo_ticker, "NOPE");
    }

    #[test]
    fn a_euro_ledger_asks_for_the_native_euro_pair() {
        let journal = journal(COINS);
        assert_eq!(
            Tickers::with_base(&journal, "€")
                .resolve("BTC")
                .yahoo_ticker,
            "BTC-EUR"
        );
        assert_eq!(
            Tickers::with_base(&journal, "EUR")
                .resolve("BTC")
                .yahoo_ticker,
            "BTC-EUR"
        );
    }

    #[test]
    fn an_unmappable_base_falls_back_to_the_symbol_and_says_so() {
        let journal = journal(COINS);
        let btc = Tickers::with_base(&journal, "¥").resolve("BTC");
        assert_eq!(btc.yahoo_ticker, "BTC");
        assert!(
            btc.note
                .as_deref()
                .is_some_and(|note| note.contains("yahoo: BTC-USD"))
        );
    }

    #[test]
    fn for_journal_finds_the_base_itself_only_when_a_coin_needs_it() {
        let journal = journal(COINS);
        let tickers = Tickers::for_journal(&journal, "2026-11-27");
        assert_eq!(tickers.base.as_deref(), Some("$"));
        assert_eq!(tickers.resolve("BTC").yahoo_ticker, "BTC-USD");
        let plain = self::journal("commodity 1.00 VTI\n");
        assert_eq!(Tickers::for_journal(&plain, "2026-11-27").base, None);
    }

    #[test]
    fn the_guard_warns_about_an_untagged_coin_symbol_priced_as_an_etf() {
        let journal = journal(COINS);
        let eth = Tickers::with_base(&journal, "$").resolve("ETH");
        let etf = instrument("ETF", "Grayscale Ethereum Mini Trust ETF");
        match judge(&eth, Some(&etf)) {
            Verdict::Warn(message) => {
                assert!(
                    message.contains("ETH priced as Grayscale Ethereum Mini Trust ETF (NYSEArca)"),
                    "{message}"
                );
                assert!(message.contains("yahoo: ETH-USD"), "{message}");
            }
            other => panic!("expected a warning, got {other:?}"),
        }
    }

    #[test]
    fn the_guard_refuses_a_coin_tag_answered_by_a_non_coin() {
        let journal = journal(COINS);
        let btc = Tickers::with_base(&journal, "$").resolve("BTC");
        let etf = instrument("ETF", "Grayscale Bitcoin Mini Trust ETF");
        assert!(matches!(judge(&btc, Some(&etf)), Verdict::Refuse(_)));
        let coin = instrument("CRYPTOCURRENCY", "Bitcoin USD");
        assert_eq!(judge(&btc, Some(&coin)), Verdict::Accept);
    }

    #[test]
    fn the_guard_leaves_ordinary_stocks_explicit_tags_and_silent_replies_alone() {
        let journal =
            journal("commodity 1.00 BTC  ; yahoo: BTC\ncommodity 1.00 VTI\ncommodity 1.00 SOL\n");
        let tickers = Tickers::with_base(&journal, "$");
        let etf = instrument("ETF", "Some ETF");
        assert_eq!(judge(&tickers.resolve("VTI"), Some(&etf)), Verdict::Accept);
        assert_eq!(
            judge(&tickers.resolve("BTC"), Some(&etf)),
            Verdict::Accept,
            "a yahoo: tag naming the ETF is deliberate"
        );
        assert_eq!(judge(&tickers.resolve("SOL"), None), Verdict::Accept);
        let untyped = Instrument {
            instrument_type: None,
            ..instrument("ETF", "x")
        };
        assert_eq!(
            judge(&tickers.resolve("SOL"), Some(&untyped)),
            Verdict::Accept
        );
    }

    #[test]
    fn a_coin_is_tagged_or_remembered_as_one() {
        let journal = journal(COINS);
        let tickers = Tickers::with_base(&journal, "$");
        assert!(is_coin(&tickers.resolve("BTC"), None));
        assert!(!is_coin(&tickers.resolve("VTI"), None));
        let coin = instrument("CRYPTOCURRENCY", "Ethereum USD");
        assert!(is_coin(&tickers.resolve("ETH"), Some(&coin)));
    }
}
