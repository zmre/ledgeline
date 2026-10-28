//! One transaction in the SPA's domain shape (`web/src/lib/domain/types.ts`
//! `Transaction`), and the lowercase search text the journal filter matches
//! against (plan 26 B5).
//!
//! Both reproduce what `web/src/lib/api/normalize.ts` used to build from the
//! hledger wire for EVERY transaction on every load:
//!
//! - an amount displays in its declared `commodity` style (precision a floor,
//!   never a cap) or else its own as-written style ([`DeclaredStyles`]);
//! - a cost is unsigned (hledger 1.52's JSON signs `@@` totals; the domain
//!   contract is the magnitude);
//! - comments are right-trimmed; a posting's tags include the account- and
//!   commodity-directive tags hledger attaches (`wire::posting_tags`);
//! - a regular posting has no `type` and an unasserting one no
//!   `balanceAssertion`, so the common case stays small.

use super::checks::WireFlag;
use super::wire::{DeclaredStyles, WireDec, WireStyle, js_trim_end};
use crate::display::{MAX_DISPLAY_DECIMALS, format_amount};
use crate::model::{Amount, CostKind, Journal, Posting, PostingType, Status, Transaction};
use crate::wire::{account_tag_map, posting_tags};
use serde::Serialize;
use std::collections::HashMap;

/// A cost annotation: `@` per unit when `per`, `@@` total otherwise; `qty`
/// always the unsigned magnitude.
#[derive(Debug, Serialize)]
pub struct RowCost {
    commodity: String,
    qty: WireDec,
    per: bool,
}

/// One single-commodity amount.
#[derive(Debug, Serialize)]
pub struct RowAmount {
    commodity: String,
    qty: WireDec,
    style: WireStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<RowCost>,
}

/// A `=`/`==`/`=*`/`==*` balance assertion.
#[derive(Debug, Serialize)]
pub struct RowAssertion {
    amount: RowAmount,
    inclusive: bool,
    total: bool,
}

/// One posting.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowPosting {
    account: String,
    amounts: Vec<RowAmount>,
    status: &'static str,
    comment: String,
    tags: Vec<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    posting_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    balance_assertion: Option<RowAssertion>,
    /// The `=` rule that generated this posting ([`Posting::generated`]);
    /// absent for a posting written in the file — the common case, so it
    /// costs nothing on the wire unless `--auto` is in force.
    #[serde(skip_serializing_if = "Option::is_none")]
    generated: Option<RowRuleRef>,
}

/// Which `=` rule generated a posting: its position in the journal's rule list
/// (what Settings › Automatic postings focuses by) and its query as written.
/// The file is left out — the rule list names it, relative to the journal —
/// so a row never carries a path.
#[derive(Debug, Serialize)]
pub struct RowRuleRef {
    rule: usize,
    query: String,
}

/// One transaction, with the check findings anchored to it embedded as
/// `flags` (so a table row needs no second lookup).
#[derive(Debug, Serialize)]
pub struct Row<'a> {
    index: u32,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    date2: Option<String>,
    status: &'static str,
    code: String,
    description: String,
    comment: String,
    tags: Vec<(String, String)>,
    postings: Vec<RowPosting>,
    flags: Vec<WireFlag<'a>>,
}

/// Journal-wide inputs every row needs, built once per request (or per
/// snapshot) rather than once per transaction.
pub struct RowContext<'a> {
    declared: &'a DeclaredStyles,
    account_tags: HashMap<&'a str, &'a [(String, String)]>,
    commodity_tags: HashMap<&'a str, &'a [(String, String)]>,
}

impl<'a> RowContext<'a> {
    /// The tag tables of `journal`, and its declared styles.
    #[must_use]
    pub fn new(journal: &'a Journal, declared: &'a DeclaredStyles) -> Self {
        Self {
            declared,
            account_tags: account_tag_map(&journal.accounts),
            commodity_tags: journal
                .commodity_tags
                .iter()
                .map(|(commodity, tags)| (commodity.0.as_str(), tags.as_slice()))
                .collect(),
        }
    }

    /// `txn` in the domain shape, carrying `flags`.
    #[must_use]
    pub fn row<'f>(&self, txn: &Transaction, flags: Vec<WireFlag<'f>>) -> Row<'f> {
        Row {
            index: txn.index.0,
            date: txn.date.clone(),
            date2: txn.date2.clone(),
            status: status_code(txn.status),
            code: txn.code.clone(),
            description: txn.description.clone(),
            comment: js_trim_end(&txn.comment).to_string(),
            tags: txn.tags.clone(),
            postings: txn
                .postings
                .iter()
                .map(|posting| self.posting(posting))
                .collect(),
            flags,
        }
    }

    fn posting(&self, posting: &Posting) -> RowPosting {
        RowPosting {
            account: posting.account.0.clone(),
            amounts: posting
                .amounts
                .iter()
                .map(|amount| self.amount(amount))
                .collect(),
            status: status_code(posting.status),
            comment: js_trim_end(&posting.comment).to_string(),
            tags: posting_tags(posting, &self.account_tags, &self.commodity_tags),
            date: posting.date.clone(),
            posting_type: match posting.ptype {
                PostingType::Regular => None,
                PostingType::Virtual => Some("virtual"),
                PostingType::BalancedVirtual => Some("balancedVirtual"),
            },
            balance_assertion: posting
                .balance_assertion
                .as_ref()
                .map(|assertion| RowAssertion {
                    amount: self.amount(&assertion.amount),
                    inclusive: assertion.inclusive,
                    total: assertion.total,
                }),
            generated: posting.generated.as_ref().map(|origin| RowRuleRef {
                rule: origin.rule,
                query: origin.query.clone(),
            }),
        }
    }

    fn amount(&self, amount: &Amount) -> RowAmount {
        RowAmount {
            commodity: amount.commodity.0.clone(),
            qty: amount.quantity.into(),
            style: WireStyle::from(&self.declared.display_style(amount)),
            cost: amount.cost.as_ref().map(|cost| RowCost {
                commodity: cost.amount.commodity.0.clone(),
                qty: cost
                    .amount
                    .quantity
                    .abs()
                    .unwrap_or(cost.amount.quantity)
                    .into(),
                per: cost.kind == CostKind::Unit,
            }),
        }
    }
}

/// The domain's `TxnStatus` spelling.
fn status_code(status: Status) -> &'static str {
    match status {
        Status::Unmarked => "unmarked",
        Status::Pending => "pending",
        Status::Cleared => "cleared",
    }
}

/// The lowercase search text a free-text filter matches against (the SPA's
/// `buildHaystack`): description, code, comment, then per posting its account,
/// comment, and each amount formatted for display plus its commodity — the
/// empty parts dropped, the rest joined by newlines.
///
/// The amount is rendered with [`format_amount`], the B2 port of the SPA's
/// formatter, so a query typed off the screen (`1,500.00`) finds the row it was
/// read from.
#[must_use]
pub fn haystack(txn: &Transaction, declared: &DeclaredStyles) -> String {
    let mut parts: Vec<String> = vec![
        txn.description.clone(),
        txn.code.clone(),
        js_trim_end(&txn.comment).to_string(),
    ];
    for posting in &txn.postings {
        parts.push(posting.account.0.clone());
        parts.push(js_trim_end(&posting.comment).to_string());
        for amount in &posting.amounts {
            let style = declared.display_style(amount);
            parts.push(format_amount(
                &amount.commodity.0,
                amount.quantity,
                &style,
                MAX_DISPLAY_DECIMALS,
            ));
            parts.push(amount.commodity.0.clone());
        }
    }
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}
