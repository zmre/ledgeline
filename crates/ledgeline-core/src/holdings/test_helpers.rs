//! Builders for inline holdings-engine unit tests — the Rust analogue of
//! `web/src/lib/holdings/test-helpers.ts`. Compiled only under `cfg(test)` and
//! shared by the engine and series test modules.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::decimal::Dec;
use crate::model::{
    AccountDeclaration, AccountName, Amount, AmountStyle, Commodity, CommoditySide, Cost, CostKind,
    Posting, PostingType, PriceDirective, SourcePos, Status, Tindex, Transaction,
};

use super::types::{HoldingsScope, ScopeMode};

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
pub fn amt(commodity: &str, mantissa: i128, places: u32) -> Amount {
    Amount {
        commodity: Commodity(commodity.to_string()),
        quantity: Dec::new(mantissa, places),
        style: style(places),
        cost: None,
    }
}

/// A USD amount from integer cents (exact).
pub fn usd(cents: i128) -> Amount {
    amt("$", cents, 2)
}

/// Attach a cost annotation in cents (`per` ⇒ `@` per-unit, else `@@` total).
pub fn with_cost(mut amount: Amount, cost_cents: i128, per: bool, commodity: &str) -> Amount {
    amount.cost = Some(Box::new(Cost {
        kind: if per { CostKind::Unit } else { CostKind::Total },
        amount: amt(commodity, cost_cents, 2),
    }));
    amount
}

/// A `P DATE COMMODITY PRICE` directive (`commodity` priced at `price_cents` of
/// `target`).
pub fn pd(date: &str, commodity: &str, price_cents: i128, target: &str) -> PriceDirective {
    PriceDirective {
        date: date.to_string(),
        commodity: Commodity(commodity.to_string()),
        price: amt(target, price_cents, 2),
    }
}

/// A posting from `(account, amounts, name-like tags)`.
pub fn posting(account: &str, amounts: Vec<Amount>, tags: &[(&str, &str)]) -> Posting {
    Posting {
        status: Status::Unmarked,
        ptype: PostingType::Regular,
        account: AccountName(account.to_string()),
        amounts,
        balance_assertion: None,
        date: None,
        date2: None,
        comment: String::new(),
        tags: tags
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

/// A cleared transaction with an explicit index (txn-index anchoring matters).
pub fn txn(index: u32, date: &str, postings: Vec<Posting>, tags: &[(&str, &str)]) -> Transaction {
    let pos = SourcePos { line: 1, column: 1 };
    Transaction {
        index: Tindex(index),
        date: date.to_string(),
        date2: None,
        status: Status::Cleared,
        code: String::new(),
        description: format!("txn {index}"),
        comment: String::new(),
        preceding_comment: String::new(),
        tags: tags
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        postings,
        source_span: (pos, pos),
    }
}

/// A single-leg buy posting with a `@`/`@@` cost in `$`.
pub fn buy(account: &str, symbol: &str, qty: i128, cost_cents: i128, per: bool) -> Posting {
    posting(
        account,
        vec![with_cost(amt(symbol, qty, 0), cost_cents, per, "$")],
        &[],
    )
}

/// A single-leg sell posting (`-qty`, no cost).
pub fn sell(account: &str, symbol: &str, qty: i128) -> Posting {
    posting(account, vec![amt(symbol, -qty, 0)], &[])
}

/// A single-leg cost-less buy posting.
pub fn buy_no_cost(account: &str, symbol: &str, qty: i128) -> Posting {
    posting(account, vec![amt(symbol, qty, 0)], &[])
}

/// An `account NAME  ; tags...` declaration from `(name, tags)`.
pub fn account_decl(name: &str, tags: &[(&str, &str)]) -> AccountDeclaration {
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

/// A `commodity ... SYMBOL  ; tags...` directive's tag entry from
/// `(symbol, tags)`, mirroring `Journal.commodity_tags`.
pub fn commodity_tags(symbol: &str, tags: &[(&str, &str)]) -> (Commodity, Vec<(String, String)>) {
    (
        Commodity(symbol.to_string()),
        tags.iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    )
}

/// Scope shorthand (all-time gain window).
pub fn scope(as_of: &str, mode: ScopeMode, accounts: &[&str]) -> HoldingsScope {
    HoldingsScope {
        accounts: accounts
            .iter()
            .map(|a| (*a).to_string())
            .collect::<BTreeSet<String>>(),
        mode,
        as_of: as_of.to_string(),
        gain_since: None,
    }
}

/// Scope shorthand with a gain-measurement window start.
pub fn scope_since(
    as_of: &str,
    mode: ScopeMode,
    accounts: &[&str],
    gain_since: &str,
) -> HoldingsScope {
    HoldingsScope {
        gain_since: Some(gain_since.to_string()),
        ..scope(as_of, mode, accounts)
    }
}
