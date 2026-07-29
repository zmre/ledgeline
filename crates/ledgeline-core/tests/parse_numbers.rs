//! Number-and-commodity parsing parity with hledger 1.52 (PARSE-3/4/5/9).
//!
//! Every expectation here was captured from a real `hledger 1.52 -f FILE print
//! -O json` run on the exact journal text in the test. These four findings
//! share one code region in `parse.rs` — the amount tokenizer — and each was a
//! silent wrong number rather than an error:
//!
//! - **PARSE-3**: the decimal mark inferred from the literal was computed and
//!   then discarded in favour of a hard-coded `'.'`, so an undeclared
//!   commodity's `1,50 CHF` read as 150 (100x) and `1.234,56 CHF` as 1.23456
//!   (1000x).
//! - **PARSE-4**: the numeric prefix stopped at the first non-`[0-9.,]` byte and
//!   whatever followed became the commodity name unvalidated, so `1 000.00 EUR`
//!   read as **1** of a commodity `"000.00 EUR"` and `0x10 XX` as **0.0**.
//! - **PARSE-5**: lot notation fell into the commodity name, so
//!   `10 AAPL {$5.00}` became a distinct commodity `"AAPL {$5.00}"` — a second
//!   holdings position with its own cost basis, silently.
//! - **PARSE-9**: quoted commodity names kept their quotes, so a `commodity`
//!   directive or `P` price never associated with the posting's commodity.
//!
//! The corpus fixtures under `fixtures/corpus/` cover the same ground against
//! committed hledger goldens; these tests additionally pin the *value*, which
//! is what the findings were actually about.

use ledgeline_core::model::{Amount, CostKind};
use ledgeline_core::{Dec, parse_journal};

/// Parse `text` and return the first transaction's first posting's amounts.
fn first_amounts(text: &str) -> Vec<Amount> {
    let journal = parse_journal(text, "test.journal")
        .unwrap_or_else(|e| panic!("expected {text:?} to parse, got: {e}"));
    journal.transactions[0].postings[0].amounts.clone()
}

/// Parse a one-posting journal and return `(commodity, mantissa, places)`.
fn first_amount(text: &str) -> (String, i128, u32) {
    let amounts = first_amounts(text);
    assert_eq!(amounts.len(), 1, "expected exactly one amount in {text:?}");
    let amount = &amounts[0];
    (
        amount.commodity.0.clone(),
        amount.quantity.mantissa,
        amount.quantity.places,
    )
}

/// Wrap `amount_text` in a two-posting transaction whose second leg is elided.
fn posting(amount_text: &str) -> String {
    format!("2024-01-01 t\n    a  {amount_text}\n    b\n")
}

// ---------------------------------------------------------------------------
// PARSE-3 — the decimal mark inferred from the literal is the one used
// ---------------------------------------------------------------------------

#[test]
fn undeclared_commodity_infers_its_own_decimal_mark() {
    // hledger 1.52: a single separator is the decimal point however many digits
    // follow it. Previously each of these was off by 100x or 1000x.
    assert_eq!(
        first_amount(&posting("1,50 CHF")),
        ("CHF".to_string(), 150, 2)
    );
    assert_eq!(first_amount(&posting("$1,234")), ("$".to_string(), 1234, 3));
    assert_eq!(
        first_amount(&posting("1.234,56 CHF")),
        ("CHF".to_string(), 123_456, 2)
    );
}

#[test]
fn repeated_separator_is_a_digit_group_not_a_decimal_point() {
    // A separator that repeats cannot be a decimal point, so hledger reads the
    // whole literal as a whole number. Previously `1.2.3` was 12.3 (10x low).
    assert_eq!(
        first_amount(&posting("1.2.3 XX")),
        ("XX".to_string(), 123, 0)
    );
    assert_eq!(
        first_amount(&posting("1.234.567 XX")),
        ("XX".to_string(), 1_234_567, 0)
    );
}

#[test]
fn a_declared_commodity_style_still_wins_over_inference() {
    // The regression guard for the fix: `commodity $1,000.00` declares `.` as
    // the decimal mark, so `,` is a digit group and `$1,234` is one thousand
    // two hundred and thirty four — the opposite reading from the undeclared
    // case above. Verified against hledger 1.52.
    let text = format!("commodity $1,000.00\n{}", posting("$1,234"));
    assert_eq!(first_amount(&text), ("$".to_string(), 1234, 0));
}

#[test]
fn a_decimal_mark_directive_still_wins_over_inference() {
    let text = format!("decimal-mark ,\n{}", posting("1.234,56 CHF"));
    assert_eq!(first_amount(&text), ("CHF".to_string(), 123_456, 2));
}

// ---------------------------------------------------------------------------
// PARSE-4 — separators inside a number, and no fabricated commodities
// ---------------------------------------------------------------------------

#[test]
fn space_and_nbsp_group_digits_inside_a_posting_amount() {
    // Both used to yield quantity 1 of a fabricated commodity "000.00 EUR".
    assert_eq!(
        first_amount(&posting("1 000.00 EUR")),
        ("EUR".to_string(), 100_000, 2)
    );
    assert_eq!(
        first_amount(&posting("1\u{a0}000.00 EUR")),
        ("EUR".to_string(), 100_000, 2)
    );
    assert_eq!(
        first_amount(&posting("1 000 000.00 EUR")),
        ("EUR".to_string(), 100_000_000, 2)
    );
}

#[test]
fn a_malformed_number_is_rejected_rather_than_read_as_zero() {
    // The worst class in the review: `0x10` stopped the numeric prefix at `0`,
    // so the posting silently contributed 0.0 to every report. hledger rejects
    // both of these.
    assert!(parse_journal(&posting("0x10 XX"), "t.journal").is_err());
    assert!(parse_journal(&posting("100.0O USD"), "t.journal").is_err());
}

#[test]
fn an_unquoted_commodity_may_not_contain_a_space() {
    // hledger requires quoting for a symbol with a space, so anything left over
    // after the number that contains whitespace is a parse error, never a
    // commodity name.
    assert!(parse_journal(&posting("100 US Dollars"), "t.journal").is_err());
}

#[test]
fn an_underscore_is_not_a_digit_group_separator() {
    // hledger 1.52 rejects `1_000.00` outright (unlike the space forms above).
    assert!(parse_journal(&posting("1_000.00 EUR"), "t.journal").is_err());
}

// ---------------------------------------------------------------------------
// PARSE-5 — lot notation
// ---------------------------------------------------------------------------

/// A parsed cost flattened for comparison: `(kind, commodity, mantissa, places)`.
type CostParts = (CostKind, String, i128, u32);

/// A parsed amount flattened for comparison: `(commodity, mantissa, places, cost)`.
type AmountParts = (String, i128, u32, Option<CostParts>);

fn first_amount_with_cost(text: &str) -> AmountParts {
    let amounts = first_amounts(text);
    assert_eq!(amounts.len(), 1, "expected exactly one amount in {text:?}");
    let amount = &amounts[0];
    let cost = amount.cost.as_ref().map(|cost| {
        (
            cost.kind,
            cost.amount.commodity.0.clone(),
            cost.amount.quantity.mantissa,
            cost.amount.quantity.places,
        )
    });
    (
        amount.commodity.0.clone(),
        amount.quantity.mantissa,
        amount.quantity.places,
        cost,
    )
}

const LOT_BUY: &str =
    "2024-01-01 lotcost\n    assets:stock   10 AAPL {$5.00}\n    assets:cash   $-50.00\n";

#[test]
fn a_unit_lot_price_yields_the_commodity_and_a_total_cost() {
    // hledger 1.52 derives a TOTAL cost of quantity x unit lot price, and does
    // not normalize it away: $50.00 at scale 2, not $50 at scale 0.
    assert_eq!(
        first_amount_with_cost(LOT_BUY),
        (
            "AAPL".to_string(),
            10,
            0,
            Some((CostKind::Total, "$".to_string(), 5000, 2))
        )
    );
}

#[test]
fn a_total_lot_price_is_taken_as_written() {
    let text = "2024-01-01 t\n    assets:stock   10 AAPL {{$50.00}}\n    assets:cash   $-50.00\n";
    assert_eq!(
        first_amount_with_cost(text),
        (
            "AAPL".to_string(),
            10,
            0,
            Some((CostKind::Total, "$".to_string(), 5000, 2))
        )
    );
}

#[test]
fn a_lot_date_is_accepted_and_ignored() {
    let text = "2024-01-01 t\n    assets:stock   10 AAPL {$5.00} [2023-06-01]\n    assets:cash   $-50.00\n";
    assert_eq!(
        first_amount_with_cost(text),
        (
            "AAPL".to_string(),
            10,
            0,
            Some((CostKind::Total, "$".to_string(), 5000, 2))
        )
    );
}

#[test]
fn selling_from_a_lot_keeps_the_sign() {
    let text = "2024-01-01 t\n    assets:stock   -10 AAPL {$5.00}\n    assets:cash   $50.00\n";
    assert_eq!(
        first_amount_with_cost(text),
        (
            "AAPL".to_string(),
            -10,
            0,
            Some((CostKind::Total, "$".to_string(), -5000, 2))
        )
    );
}

#[test]
fn an_explicit_transaction_price_overrides_the_lot_price() {
    // hledger: the `@` wins, so the transaction balances at $70 and needs the
    // extra leg. The lot price is not a second cost.
    let text = "2024-01-01 t\n    assets:stock   10 AAPL {$5.00} @ $7.00\n    assets:cash   $-50.00\n    equity:x\n";
    assert_eq!(
        first_amount_with_cost(text),
        (
            "AAPL".to_string(),
            10,
            0,
            Some((CostKind::Unit, "$".to_string(), 700, 2))
        )
    );
}

#[test]
fn an_empty_lot_records_no_cost() {
    let text = "2024-01-01 t\n    a  10 AAPL {}\n    b\n";
    assert_eq!(
        first_amount_with_cost(text),
        ("AAPL".to_string(), 10, 0, None)
    );
}

#[test]
fn lots_and_plain_holdings_are_the_same_commodity() {
    // The reported symptom: `/api/holdings` showed two positions with two cost
    // bases and no warning, because `"AAPL {$220.00}"` was a distinct commodity
    // from `AAPL`. One position of 15 is what hledger reports.
    let text = "2024-01-01 buy\n    assets:stock   10 AAPL {$220.00}\n    assets:cash   $-2200.00\n\
                \n2024-02-01 gift\n    assets:stock   5 AAPL\n    equity:gifts\n";
    let journal = parse_journal(text, "t.journal").expect("parses");
    let commodities: Vec<String> = journal
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .flat_map(|posting| &posting.amounts)
        .map(|amount| amount.commodity.0.clone())
        .filter(|commodity| commodity != "$")
        .collect();
    assert_eq!(commodities, vec!["AAPL", "AAPL", "AAPL"]);
}

#[test]
fn an_unterminated_lot_brace_is_rejected() {
    // `{` opens lot notation, so it can never be part of a commodity symbol.
    assert!(parse_journal(&posting("10 AA{PL"), "t.journal").is_err());
    assert!(parse_journal(&posting("10 AAPL {$5.00"), "t.journal").is_err());
    assert!(parse_journal(&posting("10 AAPL }"), "t.journal").is_err());
}

#[test]
fn a_square_bracket_is_still_a_legal_commodity_character() {
    // hledger accepts `10 AA[PL` as the commodity `AA[PL`; only a `[...]`
    // directly after a lot price is a lot date. Guards against over-tightening.
    assert_eq!(
        first_amount(&posting("10 AA[PL")),
        ("AA[PL".to_string(), 10, 0)
    );
}

// ---------------------------------------------------------------------------
// PARSE-9 — quoted commodity names, and an honest overflow error
// ---------------------------------------------------------------------------

#[test]
fn a_quoted_commodity_name_drops_its_quotes_on_both_sides() {
    assert_eq!(
        first_amount(&posting("3 \"green apples\"")),
        ("green apples".to_string(), 3, 0)
    );
    assert_eq!(
        first_amount(&posting("\"green apples\" 3")),
        ("green apples".to_string(), 3, 0)
    );
}

#[test]
fn a_quoted_commodity_in_a_price_directive_associates_with_the_posting() {
    // `parse_price_directive` used to split on whitespace, so the commodity
    // became `"green` and the price `apples" $1.00` failed to parse. The whole
    // point of the directive is that the two symbols match.
    let text = format!(
        "P 2024-01-01 \"green apples\" $1.00\n{}",
        posting("3 \"green apples\"")
    );
    let journal = parse_journal(&text, "t.journal").expect("parses");
    assert_eq!(journal.prices.len(), 1);
    assert_eq!(journal.prices[0].commodity.0, "green apples");
    assert_eq!(journal.prices[0].price.quantity, Dec::new(100, 2));
    assert_eq!(
        journal.prices[0].commodity, journal.transactions[0].postings[0].amounts[0].commodity,
        "the price directive must name the same commodity as the posting"
    );
}

#[test]
fn a_commodity_directive_associates_with_a_quoted_commodity() {
    // With the quotes stripped, a declared style now applies: `1,50` under
    // `commodity 1.000,00 "green apples"` is one and a half, not 150.
    let text = format!(
        "commodity 1.000,00 \"green apples\"\n{}",
        posting("1,50 \"green apples\"")
    );
    assert_eq!(first_amount(&text), ("green apples".to_string(), 150, 2));
}

#[test]
fn an_unterminated_quote_is_rejected() {
    assert!(parse_journal(&posting("3 \"green apples"), "t.journal").is_err());
    assert!(parse_journal(&posting("3 \"green\" apples"), "t.journal").is_err());
}

#[test]
fn a_mantissa_too_large_for_i128_reports_overflow() {
    // hledger's mantissa is arbitrary-precision and ours is i128, so this
    // 42-digit amount is a genuine limitation. It must fail loudly and say why:
    // the old message was "invalid numeric literal", which sends the user
    // looking for a typo that is not there.
    let error = parse_journal(
        &posting("123456789012345678901234567890123456789012 XX"),
        "t.journal",
    )
    .expect_err("42 digits overflows i128");
    let rendered = error.to_string();
    assert!(
        rendered.contains("overflow"),
        "expected an overflow error, got: {rendered}"
    );
}
