//! A customer's account (invoicer PLAN 4b): what it owes, whether any of it
//! is overdue, and its last payment, for the customer list and page.
//!
//! The balance is the dashboard's (`receivables`, ARCHITECTURE §4.6), so the
//! two never disagree: in accrual books every receivable document still
//! owed that is one of the customer's invoices or carries its `customer:`
//! tag (an opening balance, plan 45's own); in cash books its sent invoices
//! less the payments applied. The last payment is the latest payment (not a
//! credit) the books link to one of its invoices; payments on the same day
//! are added up.

use super::query::{contains_text, missing_last, search_text};
use super::{CustomerEntry, CustomerQuery, CustomerSort, SortDir, billed_to, customer_entry};
use crate::customers::Customer;
use crate::receivables::{self, OpenItem};
use crate::session::Session;
use ledgeline_core::Dec;
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::BTreeMap;

/// The DSO window `receivables` is asked for (unused here, but required).
const DSO_DAYS: u32 = 90;

/// An amount owed, in one commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Balance {
    /// The commodity (`$`).
    pub commodity: String,
    /// What is owed, in the journal's style (`$8,350.00`).
    pub amount: String,
    /// The same, as a number (sorting by what is owed).
    #[serde(skip)]
    pub value: Dec,
}

/// A payment, in one commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastPayment {
    /// When.
    pub date: String,
    /// How much, in the journal's style.
    pub amount: String,
}

/// What a customer owes and has paid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomerAccount {
    /// What is owed, per commodity; empty when nothing is.
    pub balance: Vec<Balance>,
    /// How many owed documents are past due.
    pub overdue: usize,
    /// The earliest due date among those.
    pub oldest_overdue: Option<String>,
    /// The latest payment linked to one of its invoices.
    pub last_payment: Option<LastPayment>,
}

/// A `customers list` row with its account: `GET /api/customers`.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerAccountEntry {
    /// The list row.
    #[serde(flatten)]
    pub entry: CustomerEntry,
    /// What it owes and has paid.
    pub account: CustomerAccount,
}

/// The open items (as `receivables` lists them) on `today`.
#[must_use]
pub fn open_items(session: &Session, today: &str) -> Vec<OpenItem> {
    receivables::receivables(session.index(), session.book_states(), today, DSO_DAYS).open
}

/// `customer`'s account, from `open` ([`open_items`]).
#[must_use]
pub fn customer_account(
    session: &Session,
    customer: &Customer,
    open: &[OpenItem],
) -> CustomerAccount {
    let index = session.index();
    let money = index.money();
    let docs = billed_to(index, customer);
    let paths: Vec<&VaultPath> = docs.iter().map(|d| &d.path).collect();
    let mine = |item: &&OpenItem| {
        item.path.as_ref().is_some_and(|p| paths.contains(&p))
            || item.customer.as_deref() == Some(customer.ledger_customer.as_str())
    };
    let mut owed: BTreeMap<&str, Dec> = BTreeMap::new();
    let mut overdue = 0;
    let mut oldest: Option<&str> = None;
    for item in open.iter().filter(mine) {
        let sum = owed
            .entry(item.commodity.as_str())
            .or_insert_with(Dec::zero);
        *sum = sum.add(item.outstanding).unwrap_or(*sum);
        if item.days_past_due > 0 && !item.outstanding.is_zero() {
            overdue += 1;
            if oldest.is_none_or(|o| item.due.as_str() < o) {
                oldest = Some(item.due.as_str());
            }
        }
    }
    let balance = owed
        .into_iter()
        .filter(|(_, amount)| !amount.is_zero())
        .map(|(commodity, amount)| Balance {
            commodity: commodity.to_string(),
            amount: money.style(Some(commodity)).format(amount),
            value: amount,
        })
        .collect();

    // Every payment linked to one of its invoices: (date, commodity, amount).
    let states = session.book_states();
    let mut payments: Vec<(&str, &str, Dec)> = Vec::new();
    for doc in &docs {
        let Some(standing) = doc.number().and_then(|n| states.standing(n)) else {
            continue;
        };
        for booking in &standing.bookings {
            if let Some(r) = &booking.receivable {
                payments.extend(
                    r.settled_by
                        .iter()
                        .filter(|s| !s.credit)
                        .map(|s| (s.date.as_str(), r.commodity.as_str(), s.amount)),
                );
            }
        }
        payments.extend(
            standing
                .payments
                .iter()
                .map(|p| (p.source.date.as_str(), p.commodity.as_str(), p.amount)),
        );
    }
    let last_payment = payments
        .iter()
        .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)))
        .map(|&(date, commodity, _)| {
            let total = payments
                .iter()
                .filter(|p| p.0 == date && p.1 == commodity)
                .fold(Dec::zero(), |sum, p| sum.add(p.2).unwrap_or(sum));
            LastPayment {
                date: date.to_string(),
                amount: money.style(Some(commodity)).format(total),
            }
        });
    CustomerAccount {
        balance,
        overdue,
        oldest_overdue: oldest.map(str::to_string),
        last_payment,
    }
}

/// Every customer with its account, by title (`customers list` order).
#[must_use]
pub fn customer_accounts(session: &Session, today: &str) -> Vec<CustomerAccountEntry> {
    let index = session.index();
    let states = Some(session.book_states());
    let open = open_items(session, today);
    let mut all: Vec<CustomerAccountEntry> = index
        .customers()
        .iter()
        .map(|c| CustomerAccountEntry {
            entry: customer_entry(index, states, c),
            account: customer_account(session, c, &open),
        })
        .collect();
    all.sort_by(|a, b| {
        crate::notes::normalize_name(&a.entry.title)
            .cmp(&crate::notes::normalize_name(&b.entry.title))
            .then_with(|| a.entry.path.cmp(&b.entry.path))
    });
    all
}

/// `rows` ([`customer_accounts`]) narrowed and ordered by `query` (PLAN
/// 5b, S2). Ties, and the default, are by name.
#[must_use]
pub fn query_customers(
    rows: Vec<CustomerAccountEntry>,
    query: &CustomerQuery,
) -> Vec<CustomerAccountEntry> {
    let text = search_text(query.text.as_deref());
    let mut out: Vec<CustomerAccountEntry> = rows
        .into_iter()
        .filter(|r| !query.owes || !r.account.balance.is_empty())
        .filter(|r| !query.overdue || r.account.overdue > 0)
        .filter(|r| query.kind.is_none_or(|k| r.entry.kind == k))
        .filter(|r| {
            text.as_deref().is_none_or(|t| {
                [
                    Some(r.entry.title.as_str()),
                    Some(r.entry.ledger_customer.as_str()),
                    Some(r.entry.billing.name.as_str()),
                    r.entry.billing.email.as_deref(),
                ]
                .into_iter()
                .any(|f| contains_text(f, t))
            })
        })
        .collect();
    let dir = query.dir;
    let owed = |r: &CustomerAccountEntry| {
        r.account
            .balance
            .first()
            .map_or_else(Dec::zero, |b| b.value)
    };
    match query.sort.unwrap_or(CustomerSort::Name) {
        // Already by name: only the direction can change it.
        CustomerSort::Name => {
            if dir == SortDir::Desc {
                out.reverse();
            }
        }
        CustomerSort::Owed => out.sort_by(|a, b| dir.apply(owed(a).cmp(&owed(b)))),
        CustomerSort::Overdue => out.sort_by(|a, b| {
            // More overdue invoices, then an older oldest one, is "more overdue".
            let older = match (&a.account.oldest_overdue, &b.account.oldest_overdue) {
                (Some(x), Some(y)) => y.cmp(x),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            };
            dir.apply(a.account.overdue.cmp(&b.account.overdue).then(older))
        }),
        CustomerSort::LastPayment => out.sort_by(|a, b| {
            missing_last(
                a.account.last_payment.as_ref().map(|p| p.date.as_str()),
                b.account.last_payment.as_ref().map(|p| p.date.as_str()),
                dir,
                Ord::cmp,
            )
        }),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn session(name: &str) -> Session {
        let journal = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
            .join("books.journal");
        Session::open_local(&journal).expect("opens")
    }

    fn accounts(name: &str) -> Vec<(String, CustomerAccount)> {
        let session = session(name);
        customer_accounts(&session, "2026-11-22")
            .into_iter()
            .map(|row| (row.entry.title, row.account))
            .collect()
    }

    fn owed(account: &CustomerAccount) -> Vec<&str> {
        account.balance.iter().map(|b| b.amount.as_str()).collect()
    }

    #[test]
    fn accrual_accounts_read_the_receivables() {
        let all = accounts("accrual");
        let titles: Vec<&str> = all.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Contoso",
                "Customer X",
                "Fabrikam",
                "Fred Smith",
                "Tailspin",
                "Wingtip"
            ]
        );
        let of = |title: &str| &all.iter().find(|(t, _)| t == title).expect(title).1;
        // Contoso: 107 partly paid, $2,000 left and past due.
        let contoso = of("Contoso");
        assert_eq!(owed(contoso), ["$2,000.00"]);
        assert_eq!(contoso.overdue, 1);
        assert_eq!(contoso.oldest_overdue.as_deref(), Some("2026-11-14"));
        assert_eq!(
            contoso.last_payment,
            Some(LastPayment {
                date: "2026-11-05".into(),
                amount: "$1,000.00".into()
            })
        );
        // Fabrikam: paid up, with a last payment.
        let fabrikam = of("Fabrikam");
        assert!(fabrikam.balance.is_empty());
        assert_eq!(fabrikam.overdue, 0);
        assert_eq!(
            fabrikam.last_payment.as_ref().map(|p| p.date.as_str()),
            Some("2026-10-20")
        );
        // Wingtip: its invoice and the receivable tagged with its name.
        let wingtip = of("Wingtip");
        assert_eq!(owed(wingtip), ["$2,700.00"]);
        assert_eq!(wingtip.overdue, 2);
        assert_eq!(wingtip.last_payment, None);
        // A person billed through a company owes nothing of their own.
        assert_eq!(
            of("Fred Smith"),
            &CustomerAccount {
                balance: Vec::new(),
                overdue: 0,
                oldest_overdue: None,
                last_payment: None
            }
        );
    }

    #[test]
    fn customers_filter_and_sort() {
        let session = session("accrual");
        let titles = |query: CustomerQuery| -> Vec<String> {
            query_customers(customer_accounts(&session, "2026-11-22"), &query)
                .into_iter()
                .map(|r| r.entry.title)
                .collect()
        };
        assert_eq!(
            titles(CustomerQuery {
                owes: true,
                ..CustomerQuery::default()
            }),
            ["Contoso", "Customer X", "Tailspin", "Wingtip"]
        );
        assert_eq!(
            titles(CustomerQuery {
                overdue: true,
                ..CustomerQuery::default()
            }),
            ["Contoso", "Tailspin", "Wingtip"]
        );
        assert_eq!(
            titles(CustomerQuery {
                kind: Some(crate::customers::ContactKind::Person),
                ..CustomerQuery::default()
            }),
            ["Fred Smith"]
        );
        // Title, books name, billing name or email, any case.
        assert_eq!(
            titles(CustomerQuery {
                text: Some("FRED".into()),
                ..CustomerQuery::default()
            }),
            ["Fred Smith"]
        );
        let sorted = |sort: CustomerSort, dir: SortDir| {
            titles(CustomerQuery {
                sort: Some(sort),
                dir,
                ..CustomerQuery::default()
            })
        };
        assert_eq!(
            sorted(CustomerSort::Name, SortDir::Desc),
            [
                "Wingtip",
                "Tailspin",
                "Fred Smith",
                "Fabrikam",
                "Customer X",
                "Contoso"
            ]
        );
        // Most owed first; nothing owed is zero, by name.
        assert_eq!(
            sorted(CustomerSort::Owed, SortDir::Desc),
            [
                "Customer X",
                "Tailspin",
                "Wingtip",
                "Contoso",
                "Fabrikam",
                "Fred Smith"
            ]
        );
        // Wingtip has two overdue; Tailspin's one is older than Contoso's.
        assert_eq!(
            sorted(CustomerSort::Overdue, SortDir::Desc)[..3],
            ["Wingtip", "Tailspin", "Contoso"]
        );
        // Latest payment first; none last, either way round.
        assert_eq!(
            sorted(CustomerSort::LastPayment, SortDir::Desc),
            [
                "Contoso",
                "Fabrikam",
                "Customer X",
                "Fred Smith",
                "Tailspin",
                "Wingtip"
            ]
        );
        assert_eq!(
            sorted(CustomerSort::LastPayment, SortDir::Asc),
            [
                "Customer X",
                "Fabrikam",
                "Contoso",
                "Fred Smith",
                "Tailspin",
                "Wingtip"
            ]
        );
    }

    #[test]
    fn cash_accounts_read_the_markdown_less_payments() {
        let all = accounts("cash");
        let of = |title: &str| &all.iter().find(|(t, _)| t == title).expect(title).1;
        let x = of("Customer X");
        assert_eq!(owed(x), ["$7,000.00"]);
        assert_eq!(
            x.last_payment,
            Some(LastPayment {
                date: "2026-11-08".into(),
                amount: "$5,000.00".into()
            })
        );
        let contoso = of("Contoso");
        assert!(contoso.balance.is_empty());
        assert_eq!(
            contoso.last_payment.as_ref().map(|p| p.amount.as_str()),
            Some("$3,000.00")
        );
        assert_eq!(owed(of("Fabrikam")), ["$1,500.00"]);
    }
}
