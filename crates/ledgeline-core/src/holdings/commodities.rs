//! Currency-vs-stock classification — port of
//! `web/src/lib/holdings/commodities.ts`.
//!
//! A "stock" is any commodity that is NOT a currency; currencies are the
//! bundled ISO-4217 three-letter alphabetic codes plus the currency glyphs
//! hledger journals commonly use. Everything else (`AAPL`, `VTI`, `GLD`, …) is a
//! stock the holdings engine tracks with an average-cost pool.

/// True when `commodity` is a currency (an active ISO-4217 alphabetic code or a
/// currency glyph); everything else is a stock.
///
/// The set matches `commodities.ts` exactly, including case-sensitivity (`"usd"`
/// and `"eur"` are stocks, not currencies).
#[must_use]
pub fn is_currency(commodity: &str) -> bool {
    matches!(
        commodity,
        // ISO-4217 alphabetic codes
        "AED" | "AFN" | "ALL" | "AMD" | "ANG" | "AOA" | "ARS" | "AUD" | "AWG" | "AZN"
            | "BAM" | "BBD" | "BDT" | "BGN" | "BHD" | "BIF" | "BMD" | "BND" | "BOB" | "BRL"
            | "BSD" | "BTN" | "BWP" | "BYN" | "BZD" | "CAD" | "CDF" | "CHF" | "CLP" | "CNY"
            | "COP" | "CRC" | "CUP" | "CVE" | "CZK" | "DJF" | "DKK" | "DOP" | "DZD" | "EGP"
            | "ERN" | "ETB" | "EUR" | "FJD" | "FKP" | "GBP" | "GEL" | "GHS" | "GIP" | "GMD"
            | "GNF" | "GTQ" | "GYD" | "HKD" | "HNL" | "HTG" | "HUF" | "IDR" | "ILS" | "INR"
            | "IQD" | "IRR" | "ISK" | "JMD" | "JOD" | "JPY" | "KES" | "KGS" | "KHR" | "KMF"
            | "KPW" | "KRW" | "KWD" | "KYD" | "KZT" | "LAK" | "LBP" | "LKR" | "LRD" | "LSL"
            | "LYD" | "MAD" | "MDL" | "MGA" | "MKD" | "MMK" | "MNT" | "MOP" | "MRU" | "MUR"
            | "MVR" | "MWK" | "MXN" | "MYR" | "MZN" | "NAD" | "NGN" | "NIO" | "NOK" | "NPR"
            | "NZD" | "OMR" | "PAB" | "PEN" | "PGK" | "PHP" | "PKR" | "PLN" | "PYG" | "QAR"
            | "RON" | "RSD" | "RUB" | "RWF" | "SAR" | "SBD" | "SCR" | "SDG" | "SEK" | "SGD"
            | "SHP" | "SLE" | "SOS" | "SRD" | "SSP" | "STN" | "SVC" | "SYP" | "SZL" | "THB"
            | "TJS" | "TMT" | "TND" | "TOP" | "TRY" | "TTD" | "TWD" | "TZS" | "UAH" | "UGX"
            | "USD" | "UYU" | "UZS" | "VED" | "VES" | "VND" | "VUV" | "WST" | "XAF" | "XCD"
            | "XCG" | "XDR" | "XOF" | "XPF" | "YER" | "ZAR" | "ZMW" | "ZWG"
            // symbol glyphs
            | "$" | "€" | "£" | "¥" | "US$" | "C$" | "A$" | "HK$" | "NZ$" | "S$"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_currencies() {
        for commodity in [
            "USD", "EUR", "GBP", "JPY", "CHF", "$", "€", "£", "¥", "US$", "C$", "A$", "HK$", "NZ$",
            "S$",
        ] {
            assert!(is_currency(commodity), "{commodity} should be a currency");
        }
    }

    #[test]
    fn classifies_stocks() {
        for commodity in ["AAPL", "VTI", "GLD", "BRK.B", "usd", "eur", "", "ZZZ"] {
            assert!(
                !is_currency(commodity),
                "{commodity} should be a stock (not a currency)"
            );
        }
    }
}
