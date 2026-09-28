//! Currency-vs-stock classification — a journal-aware refinement of
//! `web/src/lib/holdings/commodities.ts`.
//!
//! A "stock" is any commodity that is NOT a currency. Everything that is not a
//! currency (`AAPL`, `VTI`, `GLD`, …) is a stock the holdings engine tracks with
//! an average-cost pool.
//!
//! # Why the journal has a say
//!
//! Three-letter ISO-4217 codes collide with tickers: `BND` is the Brunei dollar
//! and Vanguard's bond ETF, `ALL` the Albanian lek and Allstate. A fixed list
//! alone would treat a BND holding as cash and drop it from the portfolio. So an
//! ISO code is a currency only when the journal USES it as money — see
//! [`Currencies`]. The glyphs (`$`, `€`, `US$`, …) name nothing else and are
//! always currencies.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::Transaction;
use crate::reports::{AccountType, resolve_account_type};

/// Which commodities one journal treats as currency.
///
/// An ISO-4217 code counts when the journal uses it as money somewhere: as the
/// commodity of a cost (`@ 5 CHF`), or posted to an account that holds or moves
/// money — a cash, liability, revenue or expense account. A code that only ever
/// sits in a non-cash asset account and is priced there (`10 BND @ $72` in
/// `assets:broker`) is a security. Matching is case-sensitive (`"usd"` is a
/// stock), as in `commodities.ts`.
#[derive(Debug, Clone, Default)]
pub struct Currencies {
    /// The ISO codes the journal was seen using as money.
    money: HashSet<String>,
}

impl Currencies {
    /// Scan `txns` for money evidence, resolving each account's type with
    /// `account_type` (declared, else inferred from its name).
    #[must_use]
    pub fn from_journal(
        txns: &[Transaction],
        account_type: impl Fn(&str) -> Option<AccountType>,
    ) -> Self {
        let mut moves_money: HashMap<&str, bool> = HashMap::new();
        let mut money = HashSet::new();
        for posting in txns.iter().flat_map(|txn| &txn.postings) {
            let account = posting.account.0.as_str();
            let money_account = *moves_money.entry(account).or_insert_with(|| {
                matches!(
                    account_type(account),
                    Some(
                        AccountType::Cash
                            | AccountType::Liability
                            | AccountType::Revenue
                            | AccountType::Gain
                            | AccountType::Expense
                    )
                )
            });
            for amount in &posting.amounts {
                let costed = amount.cost.as_deref().map(|cost| &cost.amount.commodity.0);
                for code in costed
                    .into_iter()
                    .chain(money_account.then_some(&amount.commodity.0))
                {
                    if is_iso_code(code) && !money.contains(code) {
                        money.insert(code.clone());
                    }
                }
            }
        }
        Self { money }
    }

    /// [`Self::from_journal`] with types resolved against `declared`.
    #[must_use]
    pub fn from_declared(txns: &[Transaction], declared: &BTreeMap<String, AccountType>) -> Self {
        Self::from_journal(txns, |account| resolve_account_type(account, declared))
    }

    /// True when `commodity` is a currency in this journal: a currency glyph, or
    /// an ISO-4217 code the journal uses as money.
    #[must_use]
    pub fn is_currency(&self, commodity: &str) -> bool {
        is_glyph(commodity) || (is_iso_code(commodity) && self.money.contains(commodity))
    }
}

/// True for the spellings of the US dollar a journal may use: a quote in USD
/// needs no conversion into any of them.
#[must_use]
pub fn is_us_dollar(commodity: &str) -> bool {
    matches!(commodity, "$" | "USD" | "US$")
}

/// The currency glyphs hledger journals commonly use.
fn is_glyph(commodity: &str) -> bool {
    matches!(
        commodity,
        "$" | "€" | "£" | "¥" | "US$" | "C$" | "A$" | "HK$" | "NZ$" | "S$"
    )
}

/// An active ISO-4217 alphabetic code.
fn is_iso_code(commodity: &str) -> bool {
    matches!(
        commodity,
        "AED"
            | "AFN"
            | "ALL"
            | "AMD"
            | "ANG"
            | "AOA"
            | "ARS"
            | "AUD"
            | "AWG"
            | "AZN"
            | "BAM"
            | "BBD"
            | "BDT"
            | "BGN"
            | "BHD"
            | "BIF"
            | "BMD"
            | "BND"
            | "BOB"
            | "BRL"
            | "BSD"
            | "BTN"
            | "BWP"
            | "BYN"
            | "BZD"
            | "CAD"
            | "CDF"
            | "CHF"
            | "CLP"
            | "CNY"
            | "COP"
            | "CRC"
            | "CUP"
            | "CVE"
            | "CZK"
            | "DJF"
            | "DKK"
            | "DOP"
            | "DZD"
            | "EGP"
            | "ERN"
            | "ETB"
            | "EUR"
            | "FJD"
            | "FKP"
            | "GBP"
            | "GEL"
            | "GHS"
            | "GIP"
            | "GMD"
            | "GNF"
            | "GTQ"
            | "GYD"
            | "HKD"
            | "HNL"
            | "HTG"
            | "HUF"
            | "IDR"
            | "ILS"
            | "INR"
            | "IQD"
            | "IRR"
            | "ISK"
            | "JMD"
            | "JOD"
            | "JPY"
            | "KES"
            | "KGS"
            | "KHR"
            | "KMF"
            | "KPW"
            | "KRW"
            | "KWD"
            | "KYD"
            | "KZT"
            | "LAK"
            | "LBP"
            | "LKR"
            | "LRD"
            | "LSL"
            | "LYD"
            | "MAD"
            | "MDL"
            | "MGA"
            | "MKD"
            | "MMK"
            | "MNT"
            | "MOP"
            | "MRU"
            | "MUR"
            | "MVR"
            | "MWK"
            | "MXN"
            | "MYR"
            | "MZN"
            | "NAD"
            | "NGN"
            | "NIO"
            | "NOK"
            | "NPR"
            | "NZD"
            | "OMR"
            | "PAB"
            | "PEN"
            | "PGK"
            | "PHP"
            | "PKR"
            | "PLN"
            | "PYG"
            | "QAR"
            | "RON"
            | "RSD"
            | "RUB"
            | "RWF"
            | "SAR"
            | "SBD"
            | "SCR"
            | "SDG"
            | "SEK"
            | "SGD"
            | "SHP"
            | "SLE"
            | "SOS"
            | "SRD"
            | "SSP"
            | "STN"
            | "SVC"
            | "SYP"
            | "SZL"
            | "THB"
            | "TJS"
            | "TMT"
            | "TND"
            | "TOP"
            | "TRY"
            | "TTD"
            | "TWD"
            | "TZS"
            | "UAH"
            | "UGX"
            | "USD"
            | "UYU"
            | "UZS"
            | "VED"
            | "VES"
            | "VND"
            | "VUV"
            | "WST"
            | "XAF"
            | "XCD"
            | "XCG"
            | "XDR"
            | "XOF"
            | "XPF"
            | "YER"
            | "ZAR"
            | "ZMW"
            | "ZWG"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::test_helpers::{amt, buy, posting, txn, usd, with_cost};

    fn currencies(txns: &[Transaction]) -> Currencies {
        Currencies::from_declared(txns, &BTreeMap::new())
    }

    #[test]
    fn glyphs_are_always_currencies() {
        let none = Currencies::default();
        for commodity in ["$", "€", "£", "¥", "US$", "C$", "A$", "HK$", "NZ$", "S$"] {
            assert!(
                none.is_currency(commodity),
                "{commodity} should be a currency"
            );
        }
    }

    #[test]
    fn an_iso_code_the_journal_spends_or_banks_is_a_currency() {
        let txns = vec![
            txn(
                1,
                "2026-01-02",
                vec![
                    posting("expenses:travel", vec![amt("EUR", 5000, 2)], &[]),
                    posting("assets:bank:checking", vec![usd(-5500)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2026-01-03",
                vec![
                    posting("assets:bank:wise:gbp", vec![amt("GBP", 100, 0)], &[]),
                    posting("equity:conversion", vec![amt("GBP", -100, 0)], &[]),
                ],
                &[],
            ),
            txn(
                3,
                "2026-01-04",
                vec![
                    // Paid for in francs: the cost commodity is money.
                    posting(
                        "assets:broker:nesn",
                        vec![with_cost(amt("NESN", 1, 0), 10_000, true, "CHF")],
                        &[],
                    ),
                    posting("equity:opening", vec![amt("CHF", -100, 0)], &[]),
                ],
                &[],
            ),
        ];
        let known = currencies(&txns);
        for code in ["EUR", "GBP", "CHF"] {
            assert!(known.is_currency(code), "{code} is used as money");
        }
        for commodity in [
            "AAPL", "VTI", "GLD", "BRK.B", "usd", "eur", "", "ZZZ", "NESN",
        ] {
            assert!(!known.is_currency(commodity), "{commodity} is a stock");
        }
    }

    /// `BND` is both the Brunei dollar and Vanguard's bond ETF. Bought and held
    /// in a brokerage account, it is the fund.
    #[test]
    fn an_iso_code_only_ever_held_and_priced_is_a_security() {
        let txns = vec![txn(
            1,
            "2026-01-05",
            vec![
                buy("assets:broker:bnd", "BND", 10, 7200, true),
                posting("assets:broker:cash", vec![usd(-72_000)], &[]),
            ],
            &[],
        )];
        let known = currencies(&txns);
        assert!(!known.is_currency("BND"));
        assert!(known.is_currency("$"));
        // The same code banked in a cash account is money.
        let banked = vec![txn(
            1,
            "2026-01-05",
            vec![
                posting("assets:bank:brunei", vec![amt("BND", 100, 0)], &[]),
                posting("income:salary", vec![amt("BND", -100, 0)], &[]),
            ],
            &[],
        )];
        assert!(currencies(&banked).is_currency("BND"));
    }
}
