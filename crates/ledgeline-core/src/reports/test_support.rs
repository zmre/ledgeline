//! Builders for inline report-engine unit tests — the Rust analogue of
//! `web/src/lib/reports/test-helpers.ts`. Compiled only under `cfg(test)`.

use super::mixed_amount::MixedAmount;
use crate::decimal::Dec;
use crate::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, Posting, PostingType,
    PriceDirective, SourcePos, Status, Tindex, Transaction,
};

fn style(precision: u32) -> AmountStyle {
    AmountStyle {
        side: CommoditySide::Left,
        spaced: false,
        decimal_mark: Some('.'),
        digit_groups: None,
        precision,
    }
}

/// An arbitrary-commodity amount from mantissa + decimal places (exact).
pub fn amount(commodity: &str, mantissa: i128, places: u32) -> Amount {
    Amount {
        commodity: Commodity(commodity.to_string()),
        quantity: Dec::new(mantissa, places),
        style: style(places),
        cost: None,
    }
}

/// A USD amount from integer cents (exact).
pub fn usd(cents: i128) -> Amount {
    amount("$", cents, 2)
}

/// A `P DATE COMMODITY PRICE` directive.
pub fn price(date: &str, commodity: &str, price_amount: Amount) -> PriceDirective {
    PriceDirective {
        date: date.to_string(),
        commodity: Commodity(commodity.to_string()),
        price: price_amount,
    }
}

fn posting(account: &str, amounts: Vec<Amount>) -> Posting {
    Posting {
        status: Status::Unmarked,
        ptype: PostingType::Regular,
        account: AccountName(account.to_string()),
        amounts,
        balance_assertion: None,
        date: None,
        date2: None,
        comment: String::new(),
        tags: Vec::new(),
    }
}

/// A cleared transaction from `(account, amounts)` posting tuples.
pub fn txn(index: u32, date: &str, postings: Vec<(&str, Vec<Amount>)>) -> Transaction {
    let pos = SourcePos { line: 1, column: 1 };
    Transaction {
        index: Tindex(index),
        date: date.to_string(),
        date2: None,
        status: Status::Cleared,
        code: String::new(),
        description: String::new(),
        comment: String::new(),
        preceding_comment: String::new(),
        tags: Vec::new(),
        postings: postings
            .into_iter()
            .map(|(account, amounts)| posting(account, amounts))
            .collect(),
        source_span: (pos, pos),
    }
}

/// Build a `MixedAmount` from `(commodity, mantissa, places)` triples.
pub fn mixed(pairs: &[(&str, i128, u32)]) -> MixedAmount {
    let mut ma = MixedAmount::new();
    for (commodity, mantissa, places) in pairs {
        ma.accumulate(
            &Commodity((*commodity).to_string()),
            Dec::new(*mantissa, *places),
        )
        .expect("test mixed amount does not overflow");
    }
    ma.drop_zeros();
    ma
}
