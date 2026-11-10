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
//!   `balanceAssertion`, so the common case stays small; likewise only an
//!   elided posting carries `elided: true` (plan 32 D35).
//!
//! It also holds [`Transfers`]: which linked transfer pairs (plan 35 D11)
//! collapse into one row, where that row sits under a filter, and the search
//! text it is found by. Every route that counts or positions rows asks it, so
//! they agree on what a row is.

use super::checks::WireFlag;
use super::wire::{DeclaredStyles, WireDec, WireStyle, js_trim_end};
use crate::decimal::Dec;
use crate::display::{MAX_DISPLAY_DECIMALS, format_amount};
use crate::matching::{self, PostingRef};
use crate::model::{
    Amount, AmountStyle, Commodity, CostKind, Journal, Posting, PostingType, Status, Transaction,
};
use crate::wire::{account_tag_map, posting_tags};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

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
    /// `true` when the file writes no amount here and `amounts` is the
    /// balancer's inference ([`Posting::elided`], plan 32 D35), so an editor
    /// leaves the leg blank and a re-render keeps it auto-balancing. Absent
    /// otherwise — additive, so a client that does not know it reads the
    /// inferred amounts exactly as before.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    elided: bool,
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
            elided: posting.elided,
        }
    }

    /// `quantity` of `commodity` as a [`RowAmount`], for a figure no posting
    /// wrote (a running balance, an assertion's calculated value). Displays in
    /// the declared style when there is one, else in `style` — either way with
    /// at least `quantity`'s own places, so no digit the sum holds is hidden.
    #[must_use]
    pub fn amount_of(
        &self,
        commodity: &Commodity,
        quantity: Dec,
        style: &AmountStyle,
    ) -> RowAmount {
        self.amount(&Amount {
            commodity: commodity.clone(),
            quantity,
            style: AmountStyle {
                precision: style.precision.max(quantity.places),
                ..style.clone()
            },
            cost: None,
        })
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

// ---------------------------------------------------------------------------
// Transfer rows (plan 35 D11)
// ---------------------------------------------------------------------------

/// One end of a [`Transfer`]: its transaction and the real account on its
/// side of the clearing account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferEnd {
    /// The half's position in `journal.transactions`.
    pub position: u32,
    /// The half's real (non-clearing) account, when it has one.
    pub account: Option<String>,
    /// Its short label (the last segment, or the full name when both ends'
    /// last segments collide — [`matching::AccountLabels`]).
    pub label: Option<String>,
}

/// A valid link shown as ONE journal row (D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// The `match` id.
    pub id: String,
    /// The two halves' positions, earliest `(date, tindex)` first.
    pub halves: [u32; 2],
    /// The source: the half whose real posting is negative.
    pub from: TransferEnd,
    /// The destination.
    pub to: TransferEnd,
}

impl Transfer {
    /// The other half's position.
    #[must_use]
    pub fn partner(&self, position: u32) -> u32 {
        if self.halves[0] == position {
            self.halves[1]
        } else {
            self.halves[0]
        }
    }
}

/// Every valid link in a journal, as the journal rows see them (D11).
///
/// - A pair **collapses** into one transfer row when neither of its
///   transactions is in another valid link. A group of three or more
///   postings (A3) never collapses; its transactions are only linked. A transaction in two links (a
///   transfer and a fee through the same clearing account) cannot collapse
///   into either without hiding the other, so it and its partners stay
///   ordinary rows ([`Self::is_linked`] still marks them for the ⇄ icon).
/// - The row sits at the **earliest half the filter keeps**
///   ([`Self::collapse`]).
/// - Its search text is **both halves'** ([`Self::join_haystacks`]).
#[derive(Debug, Default)]
pub struct Transfers {
    pairs: Vec<Transfer>,
    /// Each collapsing half's position → its pair in `pairs`.
    by_position: HashMap<u32, usize>,
    /// Every position with a half in any valid link.
    linked: HashSet<u32>,
}

impl Transfers {
    /// The links in `journal` ([`matching::links`]).
    #[must_use]
    pub fn of(journal: &Journal) -> Self {
        let index = matching::links(journal);
        if index.pairs().is_empty() && index.groups().is_empty() {
            return Self::default();
        }
        let clearing = matching::clearing_accounts(journal);
        let labels = matching::AccountLabels::of(journal);
        let position = |at: PostingRef| u32::try_from(at.position).unwrap_or(u32::MAX);
        let mut transfers = Self::default();
        // A group of three or more (A3) never collapses: its rows carry ⇄,
        // and a pair sharing a transaction with it stays expanded too
        // (`link_count`).
        transfers
            .linked
            .extend(index.groups().values().flatten().map(|at| position(*at)));
        for (id, (a, b)) in index.pairs() {
            transfers.linked.extend([position(*a), position(*b)]);
            if index.link_count(a.position) != 1 || index.link_count(b.position) != 1 {
                continue;
            }
            let key = |at: PostingRef| {
                let txn = &journal.transactions[at.position];
                (txn.date.as_str(), txn.index.0)
            };
            let halves = if key(*a) <= key(*b) {
                [position(*a), position(*b)]
            } else {
                [position(*b), position(*a)]
            };
            let [from, to] = matching::direction(journal, *a, *b);
            let accounts = [from, to].map(|at| matching::real_account(journal, &clearing, at));
            let [from_label, to_label] = labels.pair(accounts[0], accounts[1]);
            let at = transfers.pairs.len();
            transfers.by_position.extend(halves.map(|half| (half, at)));
            transfers.pairs.push(Transfer {
                id: id.clone(),
                halves,
                from: TransferEnd {
                    position: position(from),
                    account: accounts[0].map(str::to_string),
                    label: from_label,
                },
                to: TransferEnd {
                    position: position(to),
                    account: accounts[1].map(str::to_string),
                    label: to_label,
                },
            });
        }
        transfers
    }

    /// Whether no pair collapses.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The collapsing pair the transaction at `position` is a half of.
    #[must_use]
    pub fn get(&self, position: u32) -> Option<&Transfer> {
        self.by_position.get(&position).map(|&at| &self.pairs[at])
    }

    /// Whether the transaction at `position` has a half in any valid link,
    /// collapsing or not: the row's ⇄ icon.
    #[must_use]
    pub fn is_linked(&self, position: u32) -> bool {
        self.linked.contains(&position)
    }

    /// The rows of `filtered` (positions in display order): every position,
    /// except that a collapsing pair with BOTH halves kept shows once, at its
    /// earliest half. A pair with one half kept shows at that half (the
    /// partial-filter case), so it is still a transfer row.
    #[must_use]
    pub fn collapse(&self, filtered: &[u32]) -> Vec<u32> {
        if self.is_empty() {
            return filtered.to_vec();
        }
        let kept: HashSet<u32> = filtered
            .iter()
            .copied()
            .filter(|position| self.by_position.contains_key(position))
            .collect();
        filtered
            .iter()
            .copied()
            .filter(|&position| {
                self.get(position).is_none_or(|transfer| {
                    transfer.halves[0] == position || !kept.contains(&transfer.partner(position))
                })
            })
            .collect()
    }

    /// Give each collapsing half the search text of both halves (its own
    /// first), so a query on either description finds the transfer row.
    /// `haystacks` is indexed by position.
    pub fn join_haystacks(&self, haystacks: &mut [Box<str>]) {
        for transfer in &self.pairs {
            let [a, b] = transfer.halves.map(|half| half as usize);
            let (Some(first), Some(second)) = (haystacks.get(a), haystacks.get(b)) else {
                continue;
            };
            let joined_a = format!("{first}\n{second}").into_boxed_str();
            let joined_b = format!("{second}\n{first}").into_boxed_str();
            haystacks[a] = joined_a;
            haystacks[b] = joined_b;
        }
    }
}
