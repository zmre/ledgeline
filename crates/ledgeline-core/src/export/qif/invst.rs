//! Investment registers for the QIF export: which accounts become a
//! `!Type:Invst` register, what each security is called, and how a
//! transaction that moves a security becomes an investment record.
//!
//! # Which accounts
//!
//! The same accounts the Holdings page lists on its Stocks tab: an asset
//! account (by resolved type, never by name) that holds a commodity the
//! journal does not use as money ([`Currencies`]), unless its `holdings:` tag
//! says `other` or `none`. Those commodities are the securities. A `qif:`
//! override wins: `qif: Invst` makes an account (and its subtree) one
//! investment register, and any other `qif:` value on a security account
//! keeps it out.
//!
//! # Grouping
//!
//! A brokerage is usually booked as one account per security plus a cash
//! account, all under one parent. That parent becomes ONE register when every
//! account posted to under it is a security account or a currency-only asset
//! account, there is at most one of the latter (the brokerage's cash), and the
//! parent is not a top-level account. Otherwise each security account is its
//! own register, holding its own securities and any money posted to it.
//!
//! # Records
//!
//! [`plan`] maps one transaction to Buy/BuyX, Sell/SellX, Div/DivX,
//! ReinvDiv or ShrsIn/ShrsOut records, or says why it can't. The caller then
//! writes it the old way (cash at cost) and counts the reason.

use super::{Classifier, QifAccountType, Role, Target, class_of, join_memo, qif_amount, qif_date};
use super::{one_line, status_field};
use crate::decimal::{Dec, DecError};
use crate::edit::render_dec;
use crate::export::free_text;
use crate::holdings::{
    Currencies, HoldingsClass, declared_holdings_classes, resolve_holdings_class,
};
use crate::model::{AccountName, Commodity, CostKind, Journal, PostingType, Status, Transaction};
use crate::reports::account_types::{AccountType, AccountTypes};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

/// One security in the `!Type:Security` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Security {
    /// The `N` name, also used as every record's `Y`.
    pub name: String,
    /// The `T` type: `Stock` or `Mutual Fund`.
    pub kind: &'static str,
}

/// The journal's investment registers and securities.
#[derive(Debug)]
pub(super) struct Investments {
    /// Which commodities the journal uses as money.
    pub currencies: Currencies,
    /// Every account inside an investment register → that register's hledger
    /// account name.
    registers: BTreeMap<String, String>,
    /// Security symbol → its list entry.
    securities: BTreeMap<String, Security>,
    /// Security symbol → the currency it was first bought, sold or priced in.
    currency_of: BTreeMap<String, Commodity>,
    /// The file a costless share movement goes to when its security has no
    /// known currency: the `D` commodity, else the most-used currency.
    fallback_currency: Commodity,
}

impl Investments {
    /// Find the investment registers and securities in `applied` (the
    /// journal with auto-postings applied).
    pub(super) fn new(
        journal: &Journal,
        applied: &Journal,
        types: &AccountTypes,
        overrides: &HashMap<String, QifAccountType>,
    ) -> Self {
        let currencies = Currencies::from_journal(&applied.transactions, |a| types.resolve(a));
        let classes = declared_holdings_classes(&journal.accounts);
        let override_for = |account: &str| -> Option<(String, QifAccountType)> {
            AccountName(account.to_string())
                .self_and_ancestors()
                .into_iter()
                .find_map(|name| overrides.get(&name).map(|ty| (name.clone(), *ty)))
        };

        // Every posted account and the commodities it holds.
        let mut held: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for posting in applied
            .transactions
            .iter()
            .flat_map(|txn| &txn.postings)
            .filter(|p| p.ptype != PostingType::Virtual)
        {
            held.entry(posting.account.0.as_str())
                .or_default()
                .extend(posting.amounts.iter().map(|a| a.commodity.0.as_str()));
        }

        let mut registers: BTreeMap<String, String> = BTreeMap::new();
        let mut security_accounts: BTreeSet<&str> = BTreeSet::new();
        let mut securities: BTreeSet<&str> = BTreeSet::new();
        for (&account, commodities) in &held {
            let stocks: Vec<&str> = commodities
                .iter()
                .copied()
                .filter(|c| !currencies.is_currency(c))
                .collect();
            match override_for(account) {
                Some((declared, QifAccountType::Investment)) => {
                    registers.insert(account.to_string(), declared);
                    securities.extend(stocks);
                }
                Some(_) => {}
                None => {
                    let is_security_account = !stocks.is_empty()
                        && types.is_type(account, AccountType::Asset)
                        && !matches!(
                            resolve_holdings_class(account, &classes),
                            Some(HoldingsClass::Other | HoldingsClass::None)
                        );
                    if is_security_account {
                        security_accounts.insert(account);
                        securities.extend(stocks);
                    }
                }
            }
        }

        // Group: shallowest parents first, so a parent that takes in a whole
        // subtree claims it before a deeper one is considered.
        let mut candidates: Vec<&str> = security_accounts.iter().copied().collect();
        candidates.sort_by_key(|account| (account.matches(':').count(), *account));
        for account in candidates {
            if registers.contains_key(account) {
                continue;
            }
            let group = parent(account).filter(|p| p.contains(':')).and_then(|p| {
                clean_group(
                    p,
                    &held,
                    &security_accounts,
                    &registers,
                    types,
                    &currencies,
                    &|a| override_for(a).is_some(),
                )
                .map(|members| (p, members))
            });
            match group {
                Some((p, members)) => {
                    for member in members {
                        registers.insert(member.to_string(), p.to_string());
                    }
                }
                None => {
                    registers.insert(account.to_string(), account.to_string());
                }
            }
        }

        let currency_of = security_currencies(applied, &securities, &currencies);
        let fallback_currency = applied
            .default_commodity
            .clone()
            .unwrap_or_else(|| most_used_currency(applied, &currencies));
        Self {
            securities: name_securities(applied, &securities),
            currencies,
            registers,
            currency_of,
            fallback_currency,
        }
    }

    /// The hledger account of the investment register `account` is part of.
    pub(super) fn register(&self, account: &str) -> Option<&str> {
        self.registers.get(account).map(String::as_str)
    }

    /// Whether `account` names an investment register itself (its root,
    /// which may never be posted to).
    pub(super) fn is_register_root(&self, account: &str) -> bool {
        self.registers.values().any(|root| root == account)
    }

    /// Whether `commodity` is a security held in an investment register.
    pub(super) fn is_security(&self, commodity: &str) -> bool {
        self.securities.contains_key(commodity)
    }

    /// The list entry for a security symbol.
    pub(super) fn security(&self, symbol: &str) -> Option<&Security> {
        self.securities.get(symbol)
    }

    /// The currency file a security's costless movements belong in.
    fn home_currency(&self, symbol: &str) -> Commodity {
        self.currency_of
            .get(symbol)
            .cloned()
            .unwrap_or_else(|| self.fallback_currency.clone())
    }
}

fn parent(account: &str) -> Option<&str> {
    account.rfind(':').map(|cut| &account[..cut])
}

/// The posted accounts under `group` (itself included) when they make one
/// clean brokerage register: securities plus at most one currency-only asset
/// account, none already in another register, none with a `qif:` override.
fn clean_group<'a>(
    group: &str,
    held: &BTreeMap<&'a str, BTreeSet<&str>>,
    security_accounts: &BTreeSet<&str>,
    registers: &BTreeMap<String, String>,
    types: &AccountTypes,
    currencies: &Currencies,
    has_override: &dyn Fn(&str) -> bool,
) -> Option<Vec<&'a str>> {
    let prefix = format!("{group}:");
    let members: Vec<&str> = held
        .keys()
        .copied()
        .filter(|a| *a == group || a.starts_with(&prefix))
        .collect();
    let mut cash_accounts = 0;
    for member in &members {
        if registers.contains_key(*member) || has_override(member) {
            return None;
        }
        if security_accounts.contains(member) {
            continue;
        }
        // Anything else must be a plain asset account; one holding a
        // non-currency commodity that is not a security (a `holdings: other`
        // house) would be swallowed, so it disqualifies the group.
        if !types.is_type(member, AccountType::Asset)
            || !held[member].iter().all(|c| currencies.is_currency(c))
        {
            return None;
        }
        cash_accounts += 1;
    }
    (cash_accounts <= 1).then_some(members)
}

/// Each security's first currency: the commodity of its first cost in print
/// order, else of its first `P` directive in a currency.
fn security_currencies(
    applied: &Journal,
    securities: &BTreeSet<&str>,
    currencies: &Currencies,
) -> BTreeMap<String, Commodity> {
    let mut out: BTreeMap<String, Commodity> = BTreeMap::new();
    for amount in super::print_order(applied)
        .into_iter()
        .flat_map(|txn| &txn.postings)
        .flat_map(|p| &p.amounts)
    {
        if let Some(cost) = amount.cost.as_deref()
            && securities.contains(amount.commodity.0.as_str())
            && currencies.is_currency(&cost.amount.commodity.0)
        {
            out.entry(amount.commodity.0.clone())
                .or_insert_with(|| cost.amount.commodity.clone());
        }
    }
    for price in &applied.prices {
        if securities.contains(price.commodity.0.as_str())
            && currencies.is_currency(&price.price.commodity.0)
        {
            out.entry(price.commodity.0.clone())
                .or_insert_with(|| price.price.commodity.clone());
        }
    }
    out
}

/// The currency with the most postings, ties by name; empty when none.
fn most_used_currency(applied: &Journal, currencies: &Currencies) -> Commodity {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for amount in applied
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .flat_map(|p| &p.amounts)
        .filter(|a| currencies.is_currency(&a.commodity.0))
    {
        *counts.entry(amount.commodity.0.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(a_name, a), (b_name, b)| a.cmp(b).then_with(|| b_name.cmp(a_name)))
        .map_or_else(
            || Commodity(String::new()),
            |(c, _)| Commodity(c.to_string()),
        )
}

/// Name every security: the `commodity` directive's `name:` tag (the
/// canonical place a security is named, as on the Holdings page), else the
/// first `name:` tag on a posting of it, else its symbol. Two securities with
/// the same name get their symbols appended, so `Y` stays unambiguous.
fn name_securities(applied: &Journal, securities: &BTreeSet<&str>) -> BTreeMap<String, Security> {
    let directive = |symbol: &str, tag: &str| -> Option<String> {
        applied
            .commodity_tags
            .iter()
            .filter(|(commodity, _)| commodity.0 == symbol)
            .flat_map(|(_, tags)| tags)
            .find(|(name, value)| name == tag && !value.trim().is_empty())
            .map(|(_, value)| value.trim().to_string())
    };
    let posting_name = |symbol: &str| -> Option<String> {
        super::print_order(applied)
            .into_iter()
            .flat_map(|txn| &txn.postings)
            .filter(|p| p.amounts.iter().any(|a| a.commodity.0 == symbol))
            .flat_map(|p| &p.tags)
            .find(|(name, value)| name == "name" && !value.trim().is_empty())
            .map(|(_, value)| value.trim().to_string())
    };
    let named: Vec<(String, String, &'static str)> = securities
        .iter()
        .map(|symbol| {
            let name = directive(symbol, "name")
                .or_else(|| posting_name(symbol))
                .unwrap_or_else(|| (*symbol).to_string());
            let kind = match directive(symbol, "type").map(|t| squash(&t)).as_deref() {
                Some("mutualfund") => "Mutual Fund",
                _ => "Stock",
            };
            ((*symbol).to_string(), one_line(&name), kind)
        })
        .collect();
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for (_, name, _) in &named {
        *uses.entry(name.as_str()).or_default() += 1;
    }
    named
        .iter()
        .map(|(symbol, name, kind)| {
            let name = if uses[name.as_str()] > 1 && name != symbol {
                format!("{name} ({symbol})")
            } else {
                name.clone()
            };
            (symbol.clone(), Security { name, kind })
        })
        .collect()
}

/// Lowercase with everything but letters and digits removed.
fn squash(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// One `^`-terminated `!Type:Invst` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InvstRecord {
    pub date: String,
    pub status: Status,
    /// `N`: Buy, BuyX, Sell, ShrsIn, XIn, MiscInc …
    pub action: &'static str,
    /// `Y`: the security's list name.
    pub security: Option<String>,
    /// `I`: price per unit.
    pub price: Option<Dec>,
    /// `Q`: units, always positive.
    pub quantity: Option<Dec>,
    /// `T`: the amount, always positive.
    pub total: Option<Dec>,
    pub payee: String,
    pub memo: String,
    /// `O`: commission.
    pub commission: Option<Dec>,
    /// `L`: a category, or `[Account]` for the X actions.
    pub link: Option<String>,
    /// `$`: the amount moved to or from the `L` account.
    pub transferred: Option<Dec>,
}

impl InvstRecord {
    pub(super) fn render(&self, out: &mut String) {
        let _ = writeln!(out, "D{}", qif_date(&self.date));
        let _ = writeln!(out, "N{}", self.action);
        if let Some(security) = &self.security {
            let _ = writeln!(out, "Y{security}");
        }
        if let Some(price) = self.price {
            let _ = writeln!(out, "I{}", qif_amount(price));
        }
        if let Some(quantity) = self.quantity {
            let _ = writeln!(out, "Q{}", render_dec(quantity, '.'));
        }
        if let Some(total) = self.total {
            let _ = writeln!(out, "T{}", qif_amount(total));
        }
        if let Some(mark) = status_field(self.status) {
            let _ = writeln!(out, "C{mark}");
        }
        if !self.payee.is_empty() {
            let _ = writeln!(out, "P{}", one_line(&self.payee));
        }
        if !self.memo.is_empty() {
            let _ = writeln!(out, "M{}", one_line(&self.memo));
        }
        if let Some(commission) = self.commission {
            let _ = writeln!(out, "O{}", qif_amount(commission));
        }
        if let Some(link) = &self.link {
            let _ = writeln!(out, "L{link}");
        }
        if let Some(amount) = self.transferred {
            let _ = writeln!(out, "${}", qif_amount(amount));
        }
        out.push_str("^\n");
    }
}

/// A cash-only record placed in an investment register (a deposit, a
/// withdrawal, interest with no security, an opening balance): it has exactly
/// one split, so it becomes `XIn`/`XOut` for a transfer or
/// `MiscInc`/`MiscExp` for a category, by the sign of its total.
pub(super) fn from_cash(record: &super::Record) -> InvstRecord {
    let split = record.splits.first();
    let incoming = record.total.mantissa >= 0;
    let (action, link, transferred) = match split.map(|s| &s.target) {
        Some(Target::Transfer(_)) => (
            if incoming { "XIn" } else { "XOut" },
            split.map(super::split_target),
            Some(record.total.abs().unwrap_or(record.total)),
        ),
        Some(Target::Category(_)) => (
            if incoming { "MiscInc" } else { "MiscExp" },
            split.map(super::split_target),
            None,
        ),
        None => (if incoming { "XIn" } else { "XOut" }, None, None),
    };
    let memo = join_memo([record.memo.as_str(), split.map_or("", |s| s.memo.as_str())]);
    InvstRecord {
        date: record.date.clone(),
        status: record.status,
        action,
        security: None,
        price: None,
        quantity: None,
        total: Some(record.total.abs().unwrap_or(record.total)),
        payee: record.payee.clone(),
        memo,
        commission: None,
        link,
        transferred,
    }
}

/// A `!Type:Prices` line: `"AAPL",228.00,"9/30/24"` — symbol, price, and an
/// unpadded `M/D/YY` date, quoted as Quicken writes it.
pub(super) fn price_line(symbol: &str, price: Dec, iso_date: &str) -> String {
    let date = match (iso_date.get(2..4), iso_date.get(5..7), iso_date.get(8..10)) {
        (Some(y), Some(m), Some(d)) => format!(
            "{}/{}/{y}",
            m.trim_start_matches('0'),
            d.trim_start_matches('0')
        ),
        _ => iso_date.to_string(),
    };
    format!(
        "\"{}\",{},\"{date}\"",
        symbol.replace('"', ""),
        qif_amount(price)
    )
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Why a transaction moving a security was written as cash at cost instead.
/// Each is one line in the loss report.
pub(super) mod reason {
    pub const SEVERAL_SECURITIES: &str = "more than one security in one transaction";
    pub const NON_CURRENCY_PRICE: &str = "a security traded for something other than a currency";
    pub const OTHER_COMMODITY: &str =
        "a security and another non-currency commodity in one transaction";
    pub const EXCHANGE: &str = "a currency exchange in the same transaction as a security";
    pub const MIXED_CURRENCIES: &str = "money in more than one currency";
    pub const PARTLY_COSTED: &str = "only some of the units have a cost";
    pub const NO_COST: &str = "units bought or sold for money with no written cost";
    pub const COST_CURRENCY: &str = "a cost in a different currency from the money paid";
    pub const SEVERAL_SOURCES: &str = "money from (or to) more than one place in one trade";
    pub const UNITS_AND_MONEY: &str = "units from outside an investment account alongside money";
    pub const MANY_REGISTERS: &str = "units moving between more than two investment accounts";
    pub const NEGATIVE_FEE: &str = "an expense credited in a trade";
    pub const UNMAPPED: &str = "a security transaction no QIF investment action covers";
}

/// The outcome of [`plan`].
#[derive(Debug)]
pub(super) enum Planned {
    /// The transaction moves no security in an investment register.
    NotATrade,
    /// Written as investment records.
    Trade(Trade),
    /// Written as cash at cost, for this reason.
    Fallback(&'static str),
}

/// A transaction written as investment records.
#[derive(Debug)]
pub(super) struct Trade {
    /// The currency file it goes in.
    pub currency: Commodity,
    /// The security's symbol.
    pub symbol: String,
    /// Records by hledger register account.
    pub records: Vec<(String, InvstRecord)>,
    /// The other account money came from or went to (an X action), and its
    /// own signed change — mirrored there under `TransferPlacement::Mirrored`.
    pub counterpart: Option<(String, Dec)>,
    /// A realized gain or loss posting was left out (QIF apps compute gains
    /// from their own lots).
    pub realized_gain: bool,
    /// A dividend whose income account QIF has no field for (`Div`,
    /// `ReinvDiv`: the app uses its own dividend category).
    pub dividend_category: bool,
}

/// A unit leg's cost: its currency, its signed total, and its per-unit price
/// when written per unit (`@`).
type UnitCost<'a> = (&'a Commodity, Dec, Option<Dec>);

/// One posting amount as the planner sees it.
#[derive(Debug)]
enum Part<'a> {
    /// Units of a security in an investment register.
    Units {
        register: &'a str,
        symbol: &'a str,
        quantity: Dec,
        /// The signed total cost and its currency, and the per-unit price
        /// when it was written per unit.
        cost: Option<UnitCost<'a>>,
        status: Status,
        memo: String,
    },
    /// Units of a security leaving or entering through an account outside
    /// any investment register (equity, usually).
    Source { symbol: &'a str, quantity: Dec },
    /// Money.
    Money {
        account: &'a str,
        commodity: &'a Commodity,
        quantity: Dec,
    },
}

/// Where money in a trade came from or went to.
struct Flows<'a> {
    /// Money in each investment register's own cash.
    own: BTreeMap<&'a str, Dec>,
    /// Money in other registers, by hledger account.
    other: BTreeMap<&'a str, Dec>,
    income: Dec,
    expense: Dec,
    equity: Dec,
}

impl<'a> Flows<'a> {
    fn new(parts: &[Part<'a>], c: &'a Classifier) -> Result<Self, DecError> {
        let mut flows = Flows {
            own: BTreeMap::new(),
            other: BTreeMap::new(),
            income: Dec::zero(),
            expense: Dec::zero(),
            equity: Dec::zero(),
        };
        for part in parts {
            let Part::Money {
                account, quantity, ..
            } = part
            else {
                continue;
            };
            let slot = if let Some(register) = c.invest.register(account) {
                flows.own.entry(register).or_insert_with(Dec::zero)
            } else {
                match c.role(account) {
                    Role::Register(_) => flows.other.entry(account).or_insert_with(Dec::zero),
                    Role::Category { income: true } => &mut flows.income,
                    Role::Category { income: false } => &mut flows.expense,
                    Role::Equity { .. } => &mut flows.equity,
                }
            };
            *slot = slot.add(*quantity)?;
        }
        flows.own.retain(|_, v| !v.is_zero());
        flows.other.retain(|_, v| !v.is_zero());
        Ok(flows)
    }

    /// The one place money came from or went to, besides income and
    /// expenses; `None` when there is none or more than one.
    fn single(&self) -> Option<Counter<'a>> {
        let mut found = self
            .own
            .iter()
            .map(|(r, v)| Counter::Own(r, *v))
            .chain(self.other.iter().map(|(a, v)| Counter::Other(a, *v)))
            .chain((!self.equity.is_zero()).then_some(Counter::Equity));
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }
}

#[derive(Debug, Clone, Copy)]
enum Counter<'a> {
    Own(&'a str, Dec),
    Other(&'a str, Dec),
    Equity,
}

/// Map one transaction (auto-postings applied) to investment records.
///
/// # Errors
/// [`DecError`] only if summing amounts overflows.
pub(super) fn plan(txn: &Transaction, c: &Classifier) -> Result<Planned, DecError> {
    let invest = &c.invest;
    let mut parts: Vec<Part<'_>> = Vec::new();
    let mut exchange = false;
    let mut other_commodity = false;
    for posting in txn
        .postings
        .iter()
        .filter(|p| p.ptype != PostingType::Virtual)
    {
        let account = posting.account.0.as_str();
        for amount in &posting.amounts {
            let symbol = amount.commodity.0.as_str();
            let cost = amount.cost.as_deref();
            match (invest.register(account), invest.is_security(symbol)) {
                (Some(register), true) => {
                    let cost = match cost {
                        None => None,
                        Some(cost) if invest.currencies.is_currency(&cost.amount.commodity.0) => {
                            let unit =
                                (cost.kind == CostKind::Unit).then_some(cost.amount.quantity);
                            Some((&cost.amount.commodity, amount.at_cost()?.1, unit))
                        }
                        Some(_) => return Ok(Planned::Fallback(reason::NON_CURRENCY_PRICE)),
                    };
                    parts.push(Part::Units {
                        register,
                        symbol,
                        quantity: amount.quantity,
                        cost,
                        status: posting.status,
                        memo: free_text(&posting.comment),
                    });
                }
                (None, true) => parts.push(Part::Source {
                    symbol,
                    quantity: amount.quantity,
                }),
                _ => match cost {
                    // The other side of a costed security leg, written from
                    // this end (`$-1,000 @ 0.005 GLD`): units from outside.
                    Some(cost) if invest.is_security(&cost.amount.commodity.0) => {
                        let (commodity, quantity) = amount.at_cost()?;
                        parts.push(Part::Source {
                            symbol: commodity.0.as_str(),
                            quantity,
                        });
                    }
                    Some(_) => exchange = true,
                    None if invest.currencies.is_currency(symbol) => parts.push(Part::Money {
                        account,
                        commodity: &amount.commodity,
                        quantity: amount.quantity,
                    }),
                    None => other_commodity = true,
                },
            }
        }
    }
    if !parts.iter().any(|p| matches!(p, Part::Units { .. })) {
        return Ok(Planned::NotATrade);
    }
    if exchange {
        return Ok(Planned::Fallback(reason::EXCHANGE));
    }
    if other_commodity {
        return Ok(Planned::Fallback(reason::OTHER_COMMODITY));
    }
    let symbols: BTreeSet<&str> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Units { symbol, .. } | Part::Source { symbol, .. } => Some(*symbol),
            Part::Money { .. } => None,
        })
        .collect();
    let [symbol] = symbols.into_iter().collect::<Vec<_>>()[..] else {
        return Ok(Planned::Fallback(reason::SEVERAL_SECURITIES));
    };
    let money: BTreeSet<&Commodity> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Money {
                commodity,
                quantity,
                ..
            } if !quantity.is_zero() => Some(*commodity),
            _ => None,
        })
        .collect();
    if money.len() > 1 {
        return Ok(Planned::Fallback(reason::MIXED_CURRENCIES));
    }
    let money_currency = money.into_iter().next().cloned();

    let mut net: BTreeMap<&str, Dec> = BTreeMap::new();
    let mut sources = Dec::zero();
    for part in &parts {
        match part {
            Part::Units {
                register, quantity, ..
            } => {
                let entry = net.entry(register).or_insert_with(Dec::zero);
                *entry = entry.add(*quantity)?;
            }
            Part::Source { quantity, .. } => sources = sources.add(*quantity)?,
            Part::Money { .. } => {}
        }
    }
    let touched: Vec<&str> = net.keys().copied().collect();
    net.retain(|_, q| !q.is_zero());
    let flows = Flows::new(&parts, c)?;
    let builder = Builder {
        txn,
        c,
        parts: &parts,
        symbol,
    };

    match net.into_iter().collect::<Vec<_>>()[..] {
        [] if touched.len() == 1 && sources.is_zero() => {
            builder.dividend(touched[0], &flows, money_currency)
        }
        [] if money_currency.is_none() && sources.is_zero() => {
            // Units moved between accounts of one register: nothing changes
            // in QIF.
            Ok(Planned::Trade(builder.trade(
                invest.home_currency(symbol),
                Vec::new(),
                None,
            )))
        }
        [(from, out), (to, into)]
            if money_currency.is_none() && sources.is_zero() && out.add(into)?.is_zero() =>
        {
            let (from, to, quantity) = if out.mantissa < 0 {
                (from, to, into)
            } else {
                (to, from, out)
            };
            builder.moved_in_kind(from, to, quantity)
        }
        [(register, quantity)] => {
            builder.trade_in(register, quantity, sources, &flows, money_currency)
        }
        [] => Ok(Planned::Fallback(reason::UNMAPPED)),
        _ => Ok(Planned::Fallback(reason::MANY_REGISTERS)),
    }
}

/// The units' costs added up: their currency (`None` when no unit has a
/// cost), the unsigned total, and the per-unit price when every unit was
/// written at the same `@` price.
struct CostSummary {
    currency: Option<Commodity>,
    total: Dec,
    unit: Option<Dec>,
}

impl CostSummary {
    fn of(costs: &[UnitCost<'_>]) -> Result<Self, DecError> {
        let first_unit = costs.first().and_then(|(_, _, unit)| *unit);
        Ok(Self {
            currency: costs.first().map(|(c, _, _)| (*c).clone()),
            total: costs
                .iter()
                .try_fold(Dec::zero(), |sum, (_, total, _)| sum.add(*total))?
                .abs()?,
            unit: costs
                .iter()
                .all(|(_, _, unit)| *unit == first_unit)
                .then_some(first_unit)
                .flatten(),
        })
    }
}

/// What every record of one planned transaction shares.
struct Builder<'a, 'b> {
    txn: &'a Transaction,
    c: &'a Classifier,
    parts: &'b [Part<'a>],
    symbol: &'a str,
}

/// The fields one record fills; the rest come from the transaction.
#[derive(Default)]
struct Fields {
    action: &'static str,
    price: Option<Dec>,
    quantity: Option<Dec>,
    total: Option<Dec>,
    commission: Option<Dec>,
    link: Option<String>,
    transferred: Option<Dec>,
}

impl Builder<'_, '_> {
    /// The units in `register`, the cost of those with any, and whether every
    /// one of them has a cost.
    fn costs(&self, register: &str) -> (Vec<UnitCost<'_>>, bool) {
        let mut costs = Vec::new();
        let mut all = true;
        for part in self.parts {
            if let Part::Units {
                register: r,
                quantity,
                cost,
                ..
            } = part
                && *r == register
                && !quantity.is_zero()
            {
                match cost {
                    Some(cost) => costs.push(*cost),
                    None => all = false,
                }
            }
        }
        (costs, all)
    }

    fn record(&self, register: &str, fields: Fields) -> InvstRecord {
        let own = self.parts.iter().filter_map(|p| match p {
            Part::Units {
                register: r,
                status,
                memo,
                ..
            } if *r == register => Some((*status, memo.as_str())),
            _ => None,
        });
        let (statuses, memos): (Vec<Status>, Vec<&str>) = own.unzip();
        let txn_memo = free_text(&self.txn.comment);
        InvstRecord {
            date: self.txn.date.clone(),
            status: statuses
                .into_iter()
                .find(|s| *s != Status::Unmarked)
                .unwrap_or(self.txn.status),
            action: fields.action,
            security: self.c.invest.security(self.symbol).map(|s| s.name.clone()),
            price: fields.price,
            quantity: fields.quantity,
            total: fields.total,
            payee: self.txn.description.clone(),
            memo: join_memo(std::iter::once(txn_memo.as_str()).chain(memos)),
            commission: fields.commission,
            link: fields.link,
            transferred: fields.transferred,
        }
    }

    fn trade(
        &self,
        currency: Commodity,
        records: Vec<(String, InvstRecord)>,
        counterpart: Option<(String, Dec)>,
    ) -> Trade {
        Trade {
            currency,
            symbol: self.symbol.to_string(),
            records,
            counterpart,
            realized_gain: false,
            dividend_category: false,
        }
    }

    /// `[Account]` for the `L` of an X action, with the transaction's class.
    fn transfer_link(&self, account: &str) -> String {
        let class = class_of(&[], self.txn);
        let target = format!("[{account}]");
        match class {
            Some(class) => format!("{target}/{class}"),
            None => target,
        }
    }

    /// Income with units that do not change (a `0 AAPL` posting naming the
    /// security): `Div` into the register's cash, `DivX` to another account.
    fn dividend(
        &self,
        register: &str,
        flows: &Flows<'_>,
        currency: Option<Commodity>,
    ) -> Result<Planned, DecError> {
        let Some(currency) = currency else {
            return Ok(Planned::Trade(self.trade(
                self.c.invest.home_currency(self.symbol),
                Vec::new(),
                None,
            )));
        };
        if flows.income.mantissa >= 0 || !flows.expense.is_zero() {
            return Ok(Planned::Fallback(reason::UNMAPPED));
        }
        let amount = flows.income.neg()?;
        let (fields, counterpart) = match flows.single() {
            Some(Counter::Own(r, _)) if r == register => (
                Fields {
                    action: "Div",
                    total: Some(amount),
                    ..Fields::default()
                },
                None,
            ),
            Some(Counter::Own(account, change) | Counter::Other(account, change)) => (
                Fields {
                    action: "DivX",
                    total: Some(amount),
                    link: Some(self.link_for(account)),
                    transferred: Some(amount),
                    ..Fields::default()
                },
                Some((account.to_string(), change)),
            ),
            _ => return Ok(Planned::Fallback(reason::SEVERAL_SOURCES)),
        };
        let mut trade = self.trade(
            currency,
            vec![(register.to_string(), self.record(register, fields))],
            counterpart,
        );
        trade.dividend_category = true;
        Ok(Planned::Trade(trade))
    }

    /// The `L[...]` for money moving to `account`'s register.
    fn link_for(&self, account: &str) -> String {
        self.transfer_link(&super::qif_name(self.c.register_account(account)))
    }

    /// Units leaving one investment register for another, no money:
    /// `ShrsOut` there, `ShrsIn` here, at cost when the units carry one.
    fn moved_in_kind(&self, from: &str, to: &str, quantity: Dec) -> Result<Planned, DecError> {
        let (incoming, _) = self.costs(to);
        let costs = if incoming.is_empty() {
            self.costs(from).0
        } else {
            incoming
        };
        let cost = CostSummary::of(&costs)?;
        let fields = |action| Fields {
            action,
            price: cost.unit,
            quantity: Some(quantity),
            total: cost.currency.as_ref().map(|_| cost.total),
            ..Fields::default()
        };
        let records = vec![
            (from.to_string(), self.record(from, fields("ShrsOut"))),
            (to.to_string(), self.record(to, fields("ShrsIn"))),
        ];
        Ok(Planned::Trade(
            self.trade(
                cost.currency
                    .unwrap_or_else(|| self.c.invest.home_currency(self.symbol)),
                records,
                None,
            ),
        ))
    }

    /// Units entering or leaving one register: a buy, a sell, a reinvested
    /// dividend, or shares in or out against equity.
    fn trade_in(
        &self,
        register: &str,
        quantity: Dec,
        sources: Dec,
        flows: &Flows<'_>,
        currency: Option<Commodity>,
    ) -> Result<Planned, DecError> {
        let (costs, all_costed) = self.costs(register);
        if !costs.is_empty() && !all_costed {
            return Ok(Planned::Fallback(reason::PARTLY_COSTED));
        }
        if costs
            .iter()
            .any(|(c, _, _)| Some(*c) != costs.first().map(|(c, _, _)| *c))
        {
            return Ok(Planned::Fallback(reason::MIXED_CURRENCIES));
        }
        let CostSummary {
            currency: cost_currency,
            total: cost_total,
            unit: unit_price,
        } = CostSummary::of(&costs)?;
        let units = quantity.abs()?;
        let buying = quantity.mantissa > 0;

        // Units from outside any register, no money: shares in or out.
        if !sources.is_zero() {
            if currency.is_some() || !sources.add(quantity)?.is_zero() {
                return Ok(Planned::Fallback(reason::UNITS_AND_MONEY));
            }
            let fields = Fields {
                action: if buying { "ShrsIn" } else { "ShrsOut" },
                price: unit_price,
                quantity: Some(units),
                total: (!costs.is_empty()).then_some(cost_total),
                ..Fields::default()
            };
            return Ok(Planned::Trade(self.trade(
                cost_currency.unwrap_or_else(|| self.c.invest.home_currency(self.symbol)),
                vec![(register.to_string(), self.record(register, fields))],
                None,
            )));
        }

        let Some(currency) = currency else {
            return Ok(Planned::Fallback(reason::UNMAPPED));
        };
        if costs.is_empty() {
            return Ok(Planned::Fallback(reason::NO_COST));
        }
        if cost_currency.as_ref() != Some(&currency) {
            return Ok(Planned::Fallback(reason::COST_CURRENCY));
        }
        if flows.expense.mantissa < 0 {
            return Ok(Planned::Fallback(reason::NEGATIVE_FEE));
        }
        let commission = (!flows.expense.is_zero()).then_some(flows.expense);
        let gain = !buying && !flows.income.is_zero();
        let mut fields = Fields {
            price: if gain { None } else { unit_price },
            quantity: Some(units),
            commission,
            ..Fields::default()
        };
        let mut counterpart = None;
        let mut dividend_category = false;
        match (buying, flows.single(), flows.income.is_zero()) {
            // Bought with the register's own cash / sold into it.
            (_, Some(Counter::Own(r, change)), income) if r == register && (income || !buying) => {
                fields.action = if buying { "Buy" } else { "Sell" };
                fields.total = Some(change.abs()?);
            }
            // Cash from, or proceeds to, another account.
            (_, Some(Counter::Own(account, change) | Counter::Other(account, change)), income)
                if income || !buying =>
            {
                fields.action = if buying { "BuyX" } else { "SellX" };
                fields.total = Some(change.abs()?);
                fields.link = Some(self.link_for(account));
                fields.transferred = Some(change.abs()?);
                counterpart = Some((account.to_string(), change));
            }
            // Bought with income alone: a reinvested dividend.
            (true, None, false) if commission.is_none() => {
                fields.action = "ReinvDiv";
                fields.total = Some(cost_total);
                dividend_category = true;
            }
            // Units at cost against equity: shares in or out.
            (_, Some(Counter::Equity), true) if commission.is_none() => {
                fields.action = if buying { "ShrsIn" } else { "ShrsOut" };
                fields.total = Some(cost_total);
            }
            _ => return Ok(Planned::Fallback(reason::SEVERAL_SOURCES)),
        }
        let mut trade = self.trade(
            currency,
            vec![(register.to_string(), self.record(register, fields))],
            counterpart,
        );
        trade.realized_gain = gain;
        trade.dividend_category = dividend_category;
        Ok(Planned::Trade(trade))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{QifExport, QifOptions, TransferPlacement, export_qif};
    use super::*;
    use crate::parse_journal;

    const DECLS: &str = "account assets:bank  ; type: C\n\
                         account assets:broker:cash  ; type: C\n\
                         account assets:broker:aapl  ; type: A\n\
                         account assets:broker:msft  ; type: A\n\
                         account income  ; type: R\n\
                         account expenses  ; type: X\n\
                         account equity  ; type: E\n\n";

    /// A buy that makes `assets:broker` an investment register.
    const BUY: &str =
        "2024-01-01 Buy\n    assets:broker:aapl  1 AAPL @ $1\n    assets:broker:cash\n\n";

    fn export_with(text: &str, transfers: TransferPlacement) -> QifExport {
        let journal = parse_journal(text, "test.journal").expect("parses");
        export_qif(
            &journal,
            &QifOptions {
                name: "t".into(),
                transfers,
            },
        )
        .expect("exports")
    }

    fn export(body: &str) -> QifExport {
        export_with(&format!("{DECLS}{body}"), TransferPlacement::Once)
    }

    /// The investment records of the first file, from the first `!Type:Invst`.
    fn invst(out: &QifExport) -> &str {
        let text = &out.files[0].text;
        text.find("!Type:Invst\n").map_or("", |at| &text[at..])
    }

    #[test]
    fn a_buy_from_the_registers_own_cash_is_buy_with_its_commission() {
        let out = export(
            "2024-01-02 * Buy\n    assets:broker:aapl  10 AAPL @ $150.00\n    \
             expenses:fees  $5\n    assets:broker:cash\n",
        );
        assert!(
            invst(&out).starts_with(
                "!Type:Invst\nD01/02/2024\nNBuy\nYAAPL\nI150.00\nQ10\nT1505.00\nCX\nPBuy\nO5.00\n^\n"
            ),
            "{}",
            out.files[0].text
        );
        // The cash account is part of the one register.
        assert!(out.files[0].text.contains("Nassets:broker\nTInvst\n"));
        assert!(!out.files[0].text.contains("Nassets:broker:cash\n"));
        assert_eq!(out.loss.investment_transactions, 1);
    }

    #[test]
    fn a_buy_paid_from_another_account_is_buyx_and_mirrors() {
        let body = "2024-01-02 Buy\n    assets:broker:aapl  2 AAPL @@ $301\n    assets:bank\n";
        let out = export(body);
        // `@@`: no per-unit price is written, the total says it.
        assert!(
            invst(&out).contains("NBuyX\nYAAPL\nQ2\nT301.00\nPBuy\nL[assets:bank]\n$301.00\n^\n"),
            "{}",
            invst(&out)
        );
        assert!(!out.files[0].text.contains("!Type:Bank\n"));
        let mirrored = export_with(&format!("{DECLS}{body}"), TransferPlacement::Mirrored);
        assert!(
            mirrored.files[0]
                .text
                .contains("!Type:Bank\nD01/02/2024\nT-301.00\nPBuy\nL[assets:broker]\n^\n"),
            "{}",
            mirrored.files[0].text
        );
    }

    #[test]
    fn a_sell_is_written_at_the_journals_sale_price() {
        let out = export(
            "2024-02-01 Sell\n    assets:broker:aapl  -4 AAPL @ $160.00\n    \
             expenses:fees  $1\n    assets:broker:cash\n",
        );
        assert!(
            invst(&out).contains("NSell\nYAAPL\nI160.00\nQ4\nT639.00\nPSell\nO1.00\n^\n"),
            "{}",
            invst(&out)
        );
    }

    #[test]
    fn a_realized_gain_posting_is_left_out_with_the_price() {
        // With a gain posting the `@` is the basis, so the price is not
        // written: the total is the cash received.
        let out = export(
            "2024-02-01 Sell\n    assets:broker:aapl  -4 AAPL @ $100.00\n    \
             assets:bank  $500\n    income:gains\n",
        );
        assert!(
            invst(&out).contains("NSellX\nYAAPL\nQ4\nT500.00\nPSell\nL[assets:bank]\n$500.00\n^\n"),
            "{}",
            invst(&out)
        );
        assert_eq!(out.loss.realized_gains, 1);
        assert!(out.loss.render().contains("1 realized gain posting"));
    }

    #[test]
    fn a_dividend_naming_its_security_is_div_or_divx() {
        let out = export(
            "2024-03-01 Div\n    assets:broker:aapl  0 AAPL\n    assets:broker:cash  $2.50\n    income:div\n\n\
             2024-03-02 DivX\n    assets:broker:aapl  0 AAPL\n    assets:bank  $3.00\n    income:div\n",
        );
        let text = invst(&out);
        assert!(text.contains("NDiv\nYAAPL\nT2.50\nPDiv\n^\n"), "{text}");
        assert!(
            text.contains("NDivX\nYAAPL\nT3.00\nPDivX\nL[assets:bank]\n$3.00\n^\n"),
            "{text}"
        );
        assert_eq!(out.loss.dividend_categories, 2);
    }

    #[test]
    fn income_with_no_security_is_misc_income_with_its_category() {
        let out = export(&format!(
            "{BUY}2024-03-01 Interest\n    assets:broker:cash  $1.25\n    income:interest\n"
        ));
        assert!(
            invst(&out).contains("NMiscInc\nT1.25\nPInterest\nLincome:interest\n^\n"),
            "{}",
            out.files[0].text
        );
        assert_eq!(out.loss.investment_transactions, 1);
    }

    #[test]
    fn a_buy_paid_by_income_is_a_reinvested_dividend() {
        let out = export(
            "2024-03-01 Reinvest\n    assets:broker:aapl  0.5 AAPL @ $150\n    income:div\n",
        );
        assert!(
            invst(&out).contains("NReinvDiv\nYAAPL\nI150.00\nQ0.5\nT75.00\n"),
            "{}",
            invst(&out)
        );
    }

    #[test]
    fn units_from_equity_are_shares_in_at_cost_or_without_one() {
        let out = export(
            "2024-01-01 Transfer in\n    assets:broker:aapl  3 AAPL @ $100\n    equity:transfers\n\n\
             2024-01-02 Gift\n    assets:broker:msft  2 MSFT\n    equity:gifts  -2 MSFT\n\n\
             2024-01-03 Gift written from the other end\n    assets:broker:msft  1 MSFT\n    \
             equity:gifts  $-400 @ 0.0025 MSFT\n",
        );
        let text = invst(&out);
        assert!(
            text.contains("NShrsIn\nYAAPL\nI100.00\nQ3\nT300.00\n"),
            "{text}"
        );
        assert!(text.contains("NShrsIn\nYMSFT\nQ2\nPGift\n^\n"), "{text}");
        assert!(text.contains("NShrsIn\nYMSFT\nQ1\nPGift written"), "{text}");
        // No equity holding-account record, no MSFT file.
        assert_eq!(out.files.len(), 1);
        assert_eq!(out.loss.holding_account_transactions, 0);
    }

    #[test]
    fn units_moved_between_registers_are_shares_out_and_in() {
        let out = export_with(
            &format!(
                "{DECLS}account assets:ira  ; type: A\n\n\
                 2024-01-01 Buy\n    assets:broker:aapl  5 AAPL @ $100\n    assets:bank\n\n\
                 2024-02-01 Move\n    assets:broker:aapl  -2 AAPL\n    assets:ira  2 AAPL\n"
            ),
            TransferPlacement::Once,
        );
        let text = &out.files[0].text;
        assert!(text.contains("NShrsOut\nYAAPL\nQ2\nPMove\n^\n"), "{text}");
        assert!(
            text.contains("Nassets:ira\nTInvst\n^\n!Type:Invst\nD02/01/2024\nNShrsIn\nYAAPL\nQ2\n"),
            "{text}"
        );
    }

    #[test]
    fn money_to_another_investment_registers_cash_mirrors_as_xin_there() {
        let out = export_with(
            &format!(
                "{DECLS}account assets:other:cash  ; type: C\naccount assets:other:vti  ; type: A\n\n\
                 {BUY}2024-01-02 Buy\n    assets:other:vti  1 VTI @ $5\n    assets:other:cash\n\n\
                 2024-01-03 Sell to the other brokerage\n    assets:broker:aapl  -1 AAPL @ $2\n    \
                 assets:other:cash\n"
            ),
            TransferPlacement::Mirrored,
        );
        let text = &out.files[0].text;
        assert!(text.contains("NSellX\nYAAPL\nI2.00\nQ1\nT2.00\n"), "{text}");
        assert!(text.contains("L[assets:other]\n$2.00\n"), "{text}");
        assert!(
            text.contains("NXIn\nT2.00\nPSell to the other brokerage\nL[assets:broker]\n$2.00\n"),
            "{text}"
        );
        assert!(!text.contains("TOth A"), "{text}");
    }

    #[test]
    fn opening_cash_in_an_investment_register_is_xin_from_the_holding_account() {
        let out = export(&format!(
            "2024-01-01 Opening\n    assets:broker:cash  $1000\n    equity:opening\n\n{BUY}"
        ));
        assert!(
            invst(&out).contains(
                "NXIn\nT1000.00\nPOpening\nMequity:opening\nL[Ledgeline Adjustments]\n$1000.00\n^\n"
            ),
            "{}",
            out.files[0].text
        );
        assert_eq!(out.loss.opening_balances, 0);
    }

    #[test]
    fn a_top_level_parent_or_a_second_cash_account_keeps_registers_apart() {
        let out = export_with(
            "account assets:aapl  ; type: A\naccount assets:bank  ; type: C\n\
             account assets:b:aapl  ; type: A\naccount assets:b:cash  ; type: C\n\
             account assets:b:margin  ; type: C\n\n\
             2024-01-01 A\n    assets:aapl  1 AAPL @ $1\n    assets:bank\n\n\
             2024-01-02 B\n    assets:b:aapl  1 AAPL @ $1\n    assets:b:cash\n\n\
             2024-01-03 C\n    assets:b:margin  $1\n    assets:b:cash\n",
            TransferPlacement::Once,
        );
        let text = &out.files[0].text;
        assert!(text.contains("Nassets:aapl\nTInvst\n"), "{text}");
        assert!(text.contains("Nassets:b:aapl\nTInvst\n"), "{text}");
        assert!(text.contains("Nassets:b:cash\nTBank\n"), "{text}");
        assert!(text.contains("Nassets:bank\nTBank\n"), "{text}");
        assert!(!text.contains("Nassets:b\n"), "{text}");
    }

    #[test]
    fn holdings_other_and_qif_tags_decide_membership() {
        let out = export_with(
            "account assets:house  ; type: A, holdings: other\n\
             account assets:gold  ; type: A, qif: Oth A\n\
             account assets:pension  ; type: A, qif: Invst\n\
             account assets:bank  ; type: C\naccount equity  ; type: E\n\n\
             2024-01-01 House\n    assets:house  1 HOME @ $100\n    assets:bank\n\n\
             2024-01-02 Gold\n    assets:gold  1 GLD @ $2\n    assets:bank\n\n\
             2024-01-03 Pension\n    assets:pension:fund  1 FND @ $3\n    assets:bank\n",
            TransferPlacement::Once,
        );
        let text = &out.files[0].text;
        assert!(text.contains("Nassets:house\nTOth A\n"), "{text}");
        assert!(text.contains("Nassets:gold\nTOth A\n"), "{text}");
        assert!(text.contains("Nassets:pension\nTInvst\n"), "{text}");
        assert!(text.contains("NBuyX\nYFND\n"), "{text}");
        assert!(
            !text.contains("SHOME\n") && !text.contains("SGLD\n"),
            "{text}"
        );
        assert_eq!(out.loss.cost_postings, 2);
    }

    #[test]
    fn what_no_action_fits_falls_back_to_cash_at_cost_with_a_reason() {
        let out = export(
            "2024-01-01 Two at once\n    assets:broker:aapl  1 AAPL @ $10\n    \
             assets:broker:msft  1 MSFT @ $10\n    assets:bank\n\n\
             2024-01-02 No cost\n    assets:broker:aapl  1 AAPL\n    assets:bank  $-10\n",
        );
        let reasons: Vec<&str> = out
            .loss
            .investment_fallbacks
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(reasons, [reason::SEVERAL_SECURITIES, reason::NO_COST]);
        assert!(
            out.loss.render().contains(&format!(
                "1 security transaction written as cash at cost ({})",
                reason::NO_COST
            )),
            "{}",
            out.loss.render()
        );
        // The two-security buy is a transfer at cost into the register,
        // written in the bank (an investment record cannot split).
        assert!(
            out.files[0]
                .text
                .contains("T-20.00\nPTwo at once\nL[assets:broker]\nS[assets:broker]\n$-10.00\n"),
            "{}",
            out.files[0].text
        );
    }

    #[test]
    fn securities_are_named_and_typed_from_their_directives() {
        let out = export(
            "commodity AAPL  ; name: Apple Inc.\ncommodity MSFT  ; type: Mutual Fund\n\n\
             2024-01-01 A\n    assets:broker:aapl  1 AAPL @ $1  ; name: Ignored\n    assets:bank\n\n\
             2024-01-02 M\n    assets:broker:msft  1 MSFT @ $1  ; name: Microsoft\n    assets:bank\n",
        );
        let text = &out.files[0].text;
        assert!(
            text.contains("!Type:Security\nNApple Inc.\nSAAPL\nTStock\n^\n"),
            "{text}"
        );
        assert!(
            text.contains("!Type:Security\nNMicrosoft\nSMSFT\nTMutual Fund\n^\n"),
            "{text}"
        );
        assert!(text.contains("NBuyX\nYMicrosoft\n"), "{text}");
        // The posting `name:` tags are carried, not lost.
        assert_eq!(out.loss.posting_tags, 0);
    }

    #[test]
    fn two_securities_with_one_name_are_told_apart() {
        let out = export(
            "2024-01-01 A\n    assets:broker:aapl  1 AAPL @ $1  ; name: Tech\n    assets:bank\n\n\
             2024-01-02 M\n    assets:broker:msft  1 MSFT @ $1  ; name: Tech\n    assets:bank\n",
        );
        let text = &out.files[0].text;
        assert!(text.contains("NTech (AAPL)\nSAAPL\n"), "{text}");
        assert!(text.contains("YTech (MSFT)\n"), "{text}");
    }

    #[test]
    fn prices_are_written_only_in_the_file_their_security_is_listed_in() {
        let out = export(
            "P 2024-02-01 AAPL $12.5\nP 2024-02-01 AAPL 11 EUR\nP 2024-02-01 EUR $1.10\n\n\
             2024-01-01 A\n    assets:broker:aapl  1 AAPL @ $10\n    assets:bank\n\n\
             2024-01-02 E\n    expenses:travel  5 EUR\n    assets:bank  -5 EUR\n",
        );
        assert!(
            out.files[0]
                .text
                .ends_with("!Type:Prices\n\"AAPL\",12.50,\"2/1/24\"\n^\n"),
            "{}",
            out.files[0].text
        );
        assert!(!out.files[1].text.contains("!Type:Prices"));
        assert_eq!(out.loss.prices_written, 1);
        assert_eq!(out.loss.exchange_rates, 1);
        assert_eq!(out.loss.price_directives, 1);
    }

    #[test]
    fn price_lines_use_quickens_short_date() {
        assert_eq!(
            price_line("\"AB 1\"", Dec::new(2280, 1), "2024-09-03"),
            "\"AB 1\",228.00,\"9/3/24\""
        );
    }
}
