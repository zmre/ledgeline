//! Summaries of a time book (plan 46 T-dash, B-unbilled, B-ap, T-slots).
//!
//! - [`entries`] filters by period, person, customer, billable and branch.
//! - [`account_tree`] is `hledger bal --tree`: every account with time, its
//!   own time and its roll-up, parents first.
//! - [`summary`] groups by top-level branch, customer, person, billable or
//!   period.
//! - [`unbilled`] is B-unbilled: time on a billable account with a customer
//!   and no `invoice`/`nobill` tag, per customer; billable time with no
//!   customer is listed apart. [`unpaid`] is B-ap: each person's time with no
//!   `bill` tag, billable or not.
//! - [`suggest_periods`] turns a customer's unbilled time and `cadence` into
//!   the periods to bill.
//! - [`recents`] lists accounts by last use, for slots 6–9 (T8).
//!
//! Everything is exact ([`TimeQty`]); nothing is rounded here (B-round).

use super::model::{TimeBook, TimeEntry, ancestors_and_self, is_under};
use super::{TagRole, TimeQty};
use crate::reports::periods::{Interval, add_days, bucket_end, bucket_key, bucket_start};
use std::collections::BTreeMap;

/// A date range: `from` inclusive, `to` exclusive (hledger's `-b`/`-e`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Period {
    /// First date included.
    pub from: Option<String>,
    /// First date **not** included.
    pub to: Option<String>,
}

impl Period {
    /// Everything.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            from: None,
            to: None,
        }
    }

    /// `from..to`.
    #[must_use]
    pub fn between(from: &str, to: &str) -> Self {
        Self {
            from: Some(from.to_string()),
            to: Some(to.to_string()),
        }
    }

    /// Whether ISO `date` is in it.
    #[must_use]
    pub fn contains(&self, date: &str) -> bool {
        self.from.as_deref().is_none_or(|from| date >= from)
            && self.to.as_deref().is_none_or(|to| date < to)
    }
}

/// What [`entries`] keeps.
#[derive(Debug, Clone, Default)]
pub struct Filter<'a> {
    /// The dates.
    pub period: Period,
    /// One person's time (`Some("")`: unattributed time).
    pub person: Option<&'a str>,
    /// One customer's time.
    pub customer: Option<&'a str>,
    /// Billable (or not) time only.
    pub billable: Option<bool>,
    /// Time on this account or below it.
    pub branch: Option<&'a str>,
}

/// The entries `filter` keeps, with their index in [`TimeBook::entries`].
pub fn entries<'b>(
    book: &'b TimeBook,
    filter: &'b Filter<'_>,
) -> impl Iterator<Item = (usize, &'b TimeEntry)> + 'b {
    book.entries.iter().enumerate().filter(move |(_, entry)| {
        filter.period.contains(&entry.date)
            && filter
                .person
                .is_none_or(|p| entry.person.as_deref().unwrap_or("") == p)
            && filter.branch.is_none_or(|b| is_under(&entry.account, b))
            && filter
                .customer
                .is_none_or(|c| customer_of(book, entry) == Some(c))
            && filter
                .billable
                .is_none_or(|b| is_billable(book, entry) == b)
    })
}

/// The customer an entry's account links to (nearest ancestor).
#[must_use]
pub fn customer_of<'b>(book: &'b TimeBook, entry: &TimeEntry) -> Option<&'b str> {
    book.account(&entry.account)?.customer_name()
}

/// Whether an entry's account is billable (nearest ancestor; default no).
#[must_use]
pub fn is_billable(book: &TimeBook, entry: &TimeEntry) -> bool {
    book.account(&entry.account)
        .is_some_and(super::model::TimeAccount::is_billable)
}

/// One row of [`account_tree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// The account.
    pub account: String,
    /// 0 for a top-level branch.
    pub depth: usize,
    /// Time posted to the account itself.
    pub own: TimeQty,
    /// It and everything below it.
    pub total: TimeQty,
}

/// `hledger bal --tree`: every account with time (and its ancestors), its
/// own time and its roll-up, parents before children, sorted by segment.
/// Accounts whose roll-up is zero are left out, as hledger leaves them.
#[must_use]
pub fn account_tree(book: &TimeBook, filter: &Filter<'_>) -> Vec<TreeRow> {
    let mut own: BTreeMap<String, TimeQty> = BTreeMap::new();
    let mut total: BTreeMap<String, TimeQty> = BTreeMap::new();
    for (_, entry) in entries(book, filter) {
        let slot = own.entry(entry.account.clone()).or_default();
        *slot = slot.plus(entry.quantity);
        for at in ancestors_and_self(&entry.account) {
            let slot = total.entry(at).or_default();
            *slot = slot.plus(entry.quantity);
        }
    }
    let mut rows: Vec<TreeRow> = total
        .into_iter()
        .filter(|(_, total)| !total.is_zero())
        .map(|(account, total)| TreeRow {
            depth: account.matches(':').count(),
            own: own.get(&account).copied().unwrap_or_default(),
            total,
            account,
        })
        .collect();
    rows.sort_by(|a, b| super::accounts::by_segment(&a.account, &b.account));
    rows
}

/// What [`summary`] groups by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    /// The full account.
    Account,
    /// The top-level branch (`clients`, `internal`).
    Branch,
    /// The linked customer (`""` for none).
    Customer,
    /// The person (`""` for unattributed).
    Person,
    /// `yes` / `no` (`hledger bal --pivot billable`, resolved).
    Billable,
    /// A period bucket key (`2026-11-26`, `2026-W48`, `2026-11`, …).
    Period(Interval),
}

/// Total time per group, sorted by key.
#[must_use]
pub fn summary(book: &TimeBook, filter: &Filter<'_>, group: GroupBy) -> Vec<(String, TimeQty)> {
    let mut totals: BTreeMap<String, TimeQty> = BTreeMap::new();
    for (_, entry) in entries(book, filter) {
        let key = match group {
            GroupBy::Account => entry.account.clone(),
            GroupBy::Branch => entry.account.split(':').next().unwrap_or("").to_string(),
            GroupBy::Customer => customer_of(book, entry).unwrap_or("").to_string(),
            GroupBy::Person => entry.person.clone().unwrap_or_default(),
            GroupBy::Billable => if is_billable(book, entry) {
                "yes"
            } else {
                "no"
            }
            .to_string(),
            GroupBy::Period(interval) => bucket_key(&entry.date, interval),
        };
        let slot = totals.entry(key).or_default();
        *slot = slot.plus(entry.quantity);
    }
    totals.into_iter().collect()
}

/// A set of entries and what they add up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// Indices into [`TimeBook::entries`], in file order.
    pub entries: Vec<usize>,
    /// The total.
    pub quantity: TimeQty,
    /// The earliest date.
    pub oldest: String,
    /// The latest date.
    pub newest: String,
}

impl Batch {
    fn of(book: &TimeBook, entries: Vec<usize>) -> Option<Self> {
        let dates = || entries.iter().map(|&at| book.entries[at].date.as_str());
        Some(Self {
            quantity: TimeQty::sum(entries.iter().map(|&at| book.entries[at].quantity)),
            oldest: dates().min()?.to_string(),
            newest: dates().max()?.to_string(),
            entries,
        })
    }
}

/// B-unbilled.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnbilledReport {
    /// Per customer (by name), the billable time with no `invoice`/`nobill`.
    pub customers: Vec<(String, Batch)>,
    /// Billable time on accounts with no customer (*Needs a customer*), per
    /// account.
    pub needs_customer: Vec<(String, Batch)>,
}

/// Unbilled time through `through` (inclusive; `None` for all): time on a
/// billable account with no `invoice` or `nobill` tag, per customer, and the
/// billable time no customer is linked to.
#[must_use]
pub fn unbilled(book: &TimeBook, through: Option<&str>) -> UnbilledReport {
    let names = &book.tags;
    let mut customers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut orphans: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (at, entry) in book.entries.iter().enumerate() {
        if through.is_some_and(|through| entry.date.as_str() > through)
            || entry.quantity.is_zero()
            || names.has(&entry.tags, TagRole::Invoice)
            || names.has(&entry.tags, TagRole::Nobill)
        {
            continue;
        }
        let Some(account) = book.account(&entry.account) else {
            continue;
        };
        if !account.is_billable() {
            continue;
        }
        match account.customer_name() {
            Some(customer) => customers.entry(customer).or_default().push(at),
            None => orphans.entry(entry.account.as_str()).or_default().push(at),
        }
    }
    let batches = |map: BTreeMap<&str, Vec<usize>>| -> Vec<(String, Batch)> {
        map.into_iter()
            .filter_map(|(key, entries)| Some((key.to_string(), Batch::of(book, entries)?)))
            .collect()
    };
    UnbilledReport {
        customers: batches(customers),
        needs_customer: batches(orphans),
    }
}

/// B-ap: per person, all of their time (billable or not) with no `bill` tag,
/// through `through`. Unattributed time is keyed `""`.
#[must_use]
pub fn unpaid(book: &TimeBook, through: Option<&str>) -> Vec<(String, Batch)> {
    let mut people: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (at, entry) in book.entries.iter().enumerate() {
        if through.is_some_and(|through| entry.date.as_str() > through)
            || entry.quantity.is_zero()
            || book.tags.has(&entry.tags, TagRole::Bill)
        {
            continue;
        }
        people
            .entry(entry.person.as_deref().unwrap_or(""))
            .or_default()
            .push(at);
    }
    people
        .into_iter()
        .filter_map(|(person, entries)| Some((person.to_string(), Batch::of(book, entries)?)))
        .collect()
}

/// How often a customer is billed (the customer note's `cadence`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    /// Each ISO week.
    Weekly,
    /// Each calendar month.
    Monthly,
    /// Each calendar quarter.
    Quarterly,
    /// None set: everything through the end of last month.
    None,
}

impl Cadence {
    /// `weekly`, `monthly`, `quarterly`; anything else is [`Cadence::None`].
    #[must_use]
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("weekly") => Self::Weekly,
            Some("monthly") => Self::Monthly,
            Some("quarterly") => Self::Quarterly,
            _ => Self::None,
        }
    }

    const fn interval(self) -> Option<Interval> {
        match self {
            Self::Weekly => Some(Interval::Weekly),
            Self::Monthly => Some(Interval::Monthly),
            Self::Quarterly => Some(Interval::Quarterly),
            Self::None => None,
        }
    }
}

/// A period to bill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedPeriod {
    /// First date (inclusive).
    pub start: String,
    /// Last date (inclusive).
    pub end: String,
    /// The unbilled time in it.
    pub batch: Batch,
}

/// The periods to bill a customer's unbilled `batch` in, as of `today`
/// (B-unbilled): with a cadence, each **complete** period holding unbilled
/// time, the most recent first (the main suggestion) and older ones after
/// it; the current, unfinished period is left to accrue. Without one, a
/// single period from the oldest entry through the end of last month.
#[must_use]
pub fn suggest_periods(
    book: &TimeBook,
    batch: &Batch,
    cadence: Cadence,
    today: &str,
) -> Vec<SuggestedPeriod> {
    let Some(interval) = cadence.interval() else {
        let month_start = bucket_start(&bucket_key(today, Interval::Monthly))
            .unwrap_or_else(|_| today.to_string());
        let end = add_days(&month_start, -1);
        let entries: Vec<usize> = batch
            .entries
            .iter()
            .copied()
            .filter(|&at| book.entries[at].date.as_str() <= end.as_str())
            .collect();
        return Batch::of(book, entries)
            .map(|batch| SuggestedPeriod {
                start: batch.oldest.clone(),
                end,
                batch,
            })
            .into_iter()
            .collect();
    };
    let current = bucket_key(today, interval);
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &at in &batch.entries {
        let key = bucket_key(&book.entries[at].date, interval);
        if key < current {
            buckets.entry(key).or_default().push(at);
        }
    }
    buckets
        .into_iter()
        .rev()
        .filter_map(|(key, entries)| {
            Some(SuggestedPeriod {
                start: bucket_start(&key).ok()?,
                end: bucket_end(&key).ok()?,
                batch: Batch::of(book, entries)?,
            })
        })
        .collect()
}

/// The `n` accounts used most recently (latest date, then latest in file
/// order), skipping `exclude` and archived accounts: slots 6–9 (T8).
#[must_use]
pub fn recents(book: &TimeBook, n: usize, exclude: &[&str]) -> Vec<String> {
    let mut last: BTreeMap<&str, (&str, usize)> = BTreeMap::new();
    for (at, entry) in book.entries.iter().enumerate() {
        let seen = last
            .entry(entry.account.as_str())
            .or_insert((&entry.date, at));
        if (entry.date.as_str(), at) > *seen {
            *seen = (&entry.date, at);
        }
    }
    let mut ranked: Vec<(&str, (&str, usize))> = last
        .into_iter()
        .filter(|(account, _)| {
            !exclude.contains(account)
                && !book
                    .account(account)
                    .is_some_and(super::model::TimeAccount::is_archived)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked
        .into_iter()
        .take(n)
        .map(|(account, _)| account.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountDeclaration, AccountName, SourcePos};
    use crate::time::TimeFormat;
    use crate::time::book::TimeConfig;
    use crate::time::model::TimeSource;
    use crate::vault::VaultPath;

    fn book() -> TimeBook {
        let decl = |name: &str, tags: &[(&str, &str)]| AccountDeclaration {
            name: AccountName(name.to_string()),
            tags: tags
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            comment: String::new(),
            position: SourcePos { line: 1, column: 1 },
            source_file: "/x".into(),
        };
        let decls = vec![
            decl("clients", &[("billable", "yes")]),
            decl("clients:acme", &[("customer", "Acme")]),
            decl("clients:acme:travel", &[("billable", "no")]),
            decl("clients:globex", &[("customer", "Globex")]),
            decl("internal", &[("billable", "no")]),
            decl("internal:old", &[("archived", "yes")]),
        ];
        let text = "2026-09-30\n  clients:acme:tax  1h\n\
2026-10-02  ; person: pat\n  clients:acme:tax  2h  ; invoice: 101\n  clients:acme:tax  30m\n  clients:acme:travel  1h\n  clients:orphan  15m\n\
2026-10-20  ; person: sam\n  clients:globex  45m  ; bill: sam-2026-10\n  internal:admin  1h\n  internal:old  1h\n\
2026-11-03  ; person: pat\n  clients:acme:tax  20m\n  clients:acme:tax  10m  ; nobill: courtesy\n";
        let source = TimeSource::new(
            VaultPath::new("t.timedot").expect("p"),
            "t.timedot".into(),
            TimeFormat::Timedot,
            text.to_string(),
            None,
        );
        TimeBook::build(vec![source], &decls, &TimeConfig::default(), None)
    }

    #[test]
    fn tree_and_summaries() {
        let book = book();
        let tree = account_tree(&book, &Filter::default());
        let clients = tree
            .iter()
            .find(|r| r.account == "clients")
            .expect("clients");
        assert_eq!(
            clients.total,
            TimeQty::from_minutes(60 + 120 + 30 + 60 + 15 + 45 + 20 + 10)
        );
        assert_eq!(clients.own, TimeQty::ZERO);
        assert_eq!(tree[0].account, "clients");
        let by_customer = summary(&book, &Filter::default(), GroupBy::Customer);
        assert_eq!(
            by_customer[1],
            ("Acme".to_string(), TimeQty::from_minutes(300))
        );
        let billable = summary(&book, &Filter::default(), GroupBy::Billable);
        assert_eq!(
            billable,
            vec![
                ("no".to_string(), TimeQty::from_minutes(180)),
                ("yes".to_string(), TimeQty::from_minutes(300)),
            ]
        );
        let october = Filter {
            period: Period::between("2026-10-01", "2026-11-01"),
            person: Some("pat"),
            ..Filter::default()
        };
        assert_eq!(entries(&book, &october).count(), 4);
        let months = summary(
            &book,
            &Filter::default(),
            GroupBy::Period(Interval::Monthly),
        );
        assert_eq!(months.len(), 3);
    }

    #[test]
    fn unbilled_unpaid_and_periods() {
        let book = book();
        let report = unbilled(&book, None);
        let (name, acme) = &report.customers[0];
        assert_eq!(name, "Acme");
        assert_eq!(acme.quantity, TimeQty::from_minutes(60 + 30 + 20));
        assert_eq!(acme.oldest, "2026-09-30");
        assert_eq!(report.customers[1].1.quantity, TimeQty::from_minutes(45));
        assert_eq!(report.needs_customer[0].0, "clients:orphan");
        let monthly = suggest_periods(&book, acme, Cadence::Monthly, "2026-11-26");
        let spans: Vec<(&str, &str, TimeQty)> = monthly
            .iter()
            .map(|p| (p.start.as_str(), p.end.as_str(), p.batch.quantity))
            .collect();
        assert_eq!(
            spans,
            vec![
                ("2026-10-01", "2026-10-31", TimeQty::from_minutes(30)),
                ("2026-09-01", "2026-09-30", TimeQty::from_minutes(60)),
            ]
        );
        let none = suggest_periods(&book, acme, Cadence::parse(None), "2026-11-26");
        assert_eq!(none.len(), 1);
        assert_eq!(
            (none[0].start.as_str(), none[0].end.as_str()),
            ("2026-09-30", "2026-10-31")
        );
        assert_eq!(none[0].batch.quantity, TimeQty::from_minutes(90));
        let unpaid = unpaid(&book, Some("2026-10-31"));
        let people: Vec<(&str, TimeQty)> = unpaid
            .iter()
            .map(|(p, b)| (p.as_str(), b.quantity))
            .collect();
        assert_eq!(
            people,
            vec![
                ("", TimeQty::from_minutes(60)),
                ("pat", TimeQty::from_minutes(225)),
                ("sam", TimeQty::from_minutes(120)),
            ]
        );
        assert_eq!(
            recents(&book, 3, &["clients:acme:tax"]),
            vec![
                "internal:admin".to_string(),
                "clients:globex".to_string(),
                "clients:orphan".to_string(),
            ]
        );
    }
}
