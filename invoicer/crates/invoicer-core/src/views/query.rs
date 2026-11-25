//! Sorting and filtering the two lists (invoicer PLAN 5b, S1–S4): the
//! words a query names, parsed once here so the API, the CLI and the SPA's
//! URL all mean the same thing by them.
//!
//! The invoice list's filters live on [`super::ListQuery`]; the customer
//! list's on [`CustomerQuery`], applied by [`super::account::query_customers`].
//! Every comparison ends in the list's own order (newest first for invoices,
//! by name for customers), so ties never shuffle, and a missing value (no due
//! date, no total, no payment) sorts last whichever way the column goes.

use crate::customers::ContactKind;
use std::cmp::Ordering;

/// Which way a column sorts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortDir {
    /// Smallest, earliest or A first.
    #[default]
    Asc,
    /// Largest, latest or Z first.
    Desc,
}

impl SortDir {
    /// `asc` or `desc`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "asc" => Some(Self::Asc),
            "desc" => Some(Self::Desc),
            _ => None,
        }
    }

    /// As the URL writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }

    /// `ordering` this way round.
    #[must_use]
    pub fn apply(self, ordering: Ordering) -> Ordering {
        match self {
            Self::Asc => ordering,
            Self::Desc => ordering.reverse(),
        }
    }
}

/// Compare two values that may be missing: present ones by `cmp` in `dir`,
/// a missing one always after a present one.
pub(crate) fn missing_last<T>(
    a: Option<T>,
    b: Option<T>,
    dir: SortDir,
    cmp: impl FnOnce(&T, &T) -> Ordering,
) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => dir.apply(cmp(&a, &b)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// An invoice list column to sort by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceSort {
    /// The number, in natural order (`99` before `100`).
    Number,
    /// The customer's name.
    Customer,
    /// The invoice date.
    Date,
    /// The due date.
    Due,
    /// The amount due (by value; then by commodity).
    Total,
    /// Where it stands: draft, open, overdue, paid, void.
    Status,
}

impl InvoiceSort {
    /// Every column, as the URL names them.
    pub const ALL: [Self; 6] = [
        Self::Number,
        Self::Customer,
        Self::Date,
        Self::Due,
        Self::Total,
        Self::Status,
    ];

    /// `number`, `customer`, `date`, `due`, `total` or `status`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }

    /// As the URL writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Customer => "customer",
            Self::Date => "date",
            Self::Due => "due",
            Self::Total => "total",
            Self::Status => "status",
        }
    }
}

/// The invoice list's `status` filter: a `status:` value, or where a sent
/// invoice stands with the books.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStatus {
    /// `status: draft`.
    Draft,
    /// `status: sent`, paid or not.
    Sent,
    /// Sent and paid in full (not open).
    Paid,
    /// `status: void`.
    Void,
    /// Sent and not paid in full, overdue included.
    Open,
    /// Open and past its due date.
    Overdue,
}

impl ListStatus {
    /// Every value, as the URL names them.
    pub const ALL: [Self; 6] = [
        Self::Draft,
        Self::Sent,
        Self::Paid,
        Self::Void,
        Self::Open,
        Self::Overdue,
    ];

    /// `draft`, `sent`, `paid`, `void`, `open` or `overdue`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }

    /// As the URL writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Sent => "sent",
            Self::Paid => "paid",
            Self::Void => "void",
            Self::Open => "open",
            Self::Overdue => "overdue",
        }
    }
}

/// The invoice list's `aged` filter: open invoices by days past due, as the
/// dashboard's aging buckets link them (Ledgeline's buckets are `current`,
/// `1-30`, `31-60`, `61-90` and `91+`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aged {
    /// Not past due yet (0 days or fewer).
    Current,
    /// `min` to `max` days past due, both included; no `max` is "or more".
    Days {
        /// The fewest days past due.
        min: i64,
        /// The most, when bounded.
        max: Option<i64>,
    },
}

impl Aged {
    /// `current`, `A-B` (A to B days past due, inclusive) or `N+` (N days or
    /// more). `None` for anything else, a range running backwards included.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text == "current" {
            return Some(Self::Current);
        }
        let days = |s: &str| s.trim().parse::<i64>().ok().filter(|d| *d >= 0);
        if let Some(min) = text.strip_suffix('+') {
            return Some(Self::Days {
                min: days(min)?,
                max: None,
            });
        }
        let (min, max) = text.split_once('-')?;
        let (min, max) = (days(min)?, days(max)?);
        (min <= max).then_some(Self::Days {
            min,
            max: Some(max),
        })
    }

    /// As the URL writes it.
    #[must_use]
    pub fn as_text(self) -> String {
        match self {
            Self::Current => "current".to_string(),
            Self::Days { min, max: None } => format!("{min}+"),
            Self::Days {
                min,
                max: Some(max),
            } => format!("{min}-{max}"),
        }
    }

    /// Whether an open invoice `days` past due (negative: not yet due) is in
    /// this bucket.
    #[must_use]
    pub fn holds(self, days: i64) -> bool {
        match self {
            Self::Current => days <= 0,
            Self::Days { min, max } => days >= min && max.is_none_or(|max| days <= max),
        }
    }
}

/// Whether `haystack` holds `needle`, ignoring case (`needle` already
/// lower-cased and trimmed).
pub(crate) fn contains_text(haystack: Option<&str>, needle: &str) -> bool {
    haystack.is_some_and(|h| h.to_lowercase().contains(needle))
}

/// A search text as matched: trimmed, lower-cased; `None` when blank.
#[must_use]
pub fn search_text(text: Option<&str>) -> Option<String> {
    text.map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
}

/// A customer list column to sort by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomerSort {
    /// The name.
    Name,
    /// What it owes (the first commodity's balance; nothing owed is zero).
    Owed,
    /// How many of its invoices are overdue, then the oldest due date.
    Overdue,
    /// The date of its last payment.
    LastPayment,
}

impl CustomerSort {
    /// Every column, as the URL names them.
    pub const ALL: [Self; 4] = [Self::Name, Self::Owed, Self::Overdue, Self::LastPayment];

    /// `name`, `owed`, `overdue` or `last-payment`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }

    /// As the URL writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Owed => "owed",
            Self::Overdue => "overdue",
            Self::LastPayment => "last-payment",
        }
    }
}

/// The customer list's filters and sort (`GET /api/customers`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomerQuery {
    /// Only customers that owe something.
    pub owes: bool,
    /// Only customers with an overdue invoice.
    pub overdue: bool,
    /// Only people, or only organizations.
    pub kind: Option<ContactKind>,
    /// Only names (title, name in the books, billing name or email)
    /// holding this text, any case.
    pub text: Option<String>,
    /// The column; `None` is by name.
    pub sort: Option<CustomerSort>,
    /// Which way.
    pub dir: SortDir,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aged_reads_buckets_and_ranges() {
        assert_eq!(Aged::parse("current"), Some(Aged::Current));
        assert_eq!(
            Aged::parse("31-60"),
            Some(Aged::Days {
                min: 31,
                max: Some(60)
            })
        );
        assert_eq!(Aged::parse("91+"), Some(Aged::Days { min: 91, max: None }));
        for bad in ["", "60-31", "-5", "abc", "1-", "x+", "-1-3"] {
            assert_eq!(Aged::parse(bad), None, "{bad}");
        }
        let bucket = Aged::parse("31-60").expect("reads");
        assert!(!bucket.holds(30));
        assert!(bucket.holds(31));
        assert!(bucket.holds(60));
        assert!(!bucket.holds(61));
        assert!(Aged::Current.holds(0));
        assert!(Aged::Current.holds(-3));
        assert!(!Aged::Current.holds(1));
        assert!(Aged::parse("91+").expect("reads").holds(400));
        assert_eq!(bucket.as_text(), "31-60");
        assert_eq!(Aged::Days { min: 91, max: None }.as_text(), "91+");
    }

    #[test]
    fn names_round_trip() {
        for s in InvoiceSort::ALL {
            assert_eq!(InvoiceSort::parse(s.as_str()), Some(s));
        }
        for s in ListStatus::ALL {
            assert_eq!(ListStatus::parse(s.as_str()), Some(s));
        }
        for s in CustomerSort::ALL {
            assert_eq!(CustomerSort::parse(s.as_str()), Some(s));
        }
        assert_eq!(SortDir::parse("desc"), Some(SortDir::Desc));
        assert_eq!(SortDir::parse("down"), None);
        assert_eq!(InvoiceSort::parse("amount"), None);
    }

    #[test]
    fn missing_values_sort_last_both_ways() {
        let cmp = |a: Option<i32>, b: Option<i32>, dir| missing_last(a, b, dir, |x, y| x.cmp(y));
        assert_eq!(cmp(Some(1), None, SortDir::Asc), Ordering::Less);
        assert_eq!(cmp(Some(1), None, SortDir::Desc), Ordering::Less);
        assert_eq!(cmp(Some(1), Some(2), SortDir::Desc), Ordering::Greater);
        assert_eq!(search_text(Some("  PO ")), Some("po".to_string()));
        assert_eq!(search_text(Some("  ")), None);
    }
}
