//! The books column: a row's accounting (ARCHITECTURE §3.3).
//!
//! ```text
//! <account>
//! <deferred-account> → <recognize-into>, <N> months [from <YYYY-MM-DD>] [calendar]
//! ```
//!
//! `->` is accepted for `→`. A deferred cell maps one-to-one onto plan 45's
//! [`Deferral`]; the grammar's keywords are file format, not settings.

use ledgeline_core::accrual::invoice::Deferral;
use ledgeline_core::accrual::schedule::{Proration, Term};
use std::fmt;
use thiserror::Error;

use super::money::is_blank;

/// Why a books cell doesn't parse.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BooksError {
    /// Not an account name hledger would read back as written.
    #[error("`{name}` is not a usable account name: {why}")]
    Account {
        /// The name.
        name: String,
        /// The rule it breaks.
        why: &'static str,
    },
    /// A deferral that doesn't follow the grammar.
    #[error(
        "`{0}` is not a deferral; write `deferred-account → revenue-account, N months [from YYYY-MM-DD] [calendar]`"
    )]
    Deferral(String),
    /// A `from` date that isn't a real ISO date.
    #[error("`{0}` is not a date (YYYY-MM-DD)")]
    Date(String),
}

/// Longest account name accepted, as ledgeline-core's writers bound it.
const MAX_ACCOUNT_BYTES: usize = 1024;

/// A validated hledger account name: one a journal line will read back
/// exactly as written.
///
/// The rules mirror ledgeline-core's (crate-private) `periodic::check_account`,
/// plus "no pipe", since the name lives in a table cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountName(String);

impl AccountName {
    /// Validate `name`.
    ///
    /// # Errors
    /// [`BooksError::Account`] naming the rule broken.
    pub fn new(name: &str) -> Result<Self, BooksError> {
        let fail = |why| {
            Err(BooksError::Account {
                name: name.to_string(),
                why,
            })
        };
        if name.is_empty() {
            return fail("it is empty");
        }
        if name.len() > MAX_ACCOUNT_BYTES {
            return fail("it is too long");
        }
        if name.trim() != name {
            return fail("it begins or ends with whitespace");
        }
        if name.chars().any(char::is_control) {
            return fail("it contains a control character");
        }
        if name.contains([';', '#']) {
            return fail("`;` and `#` start a journal comment");
        }
        if name.contains("  ") || name.contains('\t') {
            return fail("two spaces or a tab end an account name in a journal");
        }
        if name.contains(['(', ')', '[', ']']) {
            return fail("brackets mark a virtual posting");
        }
        if name.contains('|') {
            return fail("a pipe would split the table cell");
        }
        Ok(Self(name.to_string()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<AccountName> for ledgeline_core::model::AccountName {
    fn from(name: AccountName) -> Self {
        Self(name.0)
    }
}

/// A row's accounting: a plain account, or a deferred account with its
/// recognition terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooksEntry {
    /// Booked straight to this account.
    Account(AccountName),
    /// Booked to `account` (deferred revenue) and recognized per `deferral`.
    Deferred {
        /// The deferred-revenue account the invoice credits.
        account: AccountName,
        /// The recognition terms.
        deferral: Deferral,
    },
}

impl BooksEntry {
    /// The account the invoice's transaction posts to.
    #[must_use]
    pub fn account(&self) -> &AccountName {
        match self {
            Self::Account(account) | Self::Deferred { account, .. } => account,
        }
    }

    /// The recognition terms, for a deferred entry.
    #[must_use]
    pub fn deferral(&self) -> Option<&Deferral> {
        match self {
            Self::Account(_) => None,
            Self::Deferred { deferral, .. } => Some(deferral),
        }
    }

    /// Parse a books cell; `Ok(None)` for a blank one.
    ///
    /// # Errors
    /// [`BooksError`] for a bad account name or deferral.
    pub fn parse(text: &str) -> Result<Option<Self>, BooksError> {
        if is_blank(text) {
            return Ok(None);
        }
        let text = text.trim();
        let arrow = text
            .find('→')
            .map(|at| (at, '→'.len_utf8()))
            .or_else(|| text.find("->").map(|at| (at, 2)));
        let Some((at, len)) = arrow else {
            return AccountName::new(text).map(|account| Some(Self::Account(account)));
        };
        let account = AccountName::new(text[..at].trim())?;
        let rest = &text[at + len..];
        let bad = || BooksError::Deferral(text.to_string());
        // The target is everything before the LAST comma, so the term clause
        // is unambiguous.
        let (target, terms) = rest.rsplit_once(',').ok_or_else(bad)?;
        let target = AccountName::new(target.trim())?;
        let deferral = parse_terms(terms, target).ok_or_else(bad)??;
        Ok(Some(Self::Deferred { account, deferral }))
    }
}

/// `N months [from DATE] [calendar]`, case-insensitive. `None` for text that
/// doesn't follow the grammar; `Some(Err)` for a bad date.
fn parse_terms(terms: &str, target: AccountName) -> Option<Result<Deferral, BooksError>> {
    let words: Vec<&str> = terms.split_whitespace().collect();
    let (count, unit, tail) = match words.as_slice() {
        [count, unit, tail @ ..] => (count, unit, tail),
        _ => return None,
    };
    let months: u32 = count.parse().ok().filter(|n| *n > 0)?;
    if !(unit.eq_ignore_ascii_case("months") || unit.eq_ignore_ascii_case("month")) {
        return None;
    }
    let (start, tail) = match tail {
        [from, date, rest @ ..] if from.eq_ignore_ascii_case("from") => (Some(*date), rest),
        _ => (None, tail),
    };
    let calendar = match tail {
        [] => false,
        [word] if word.eq_ignore_ascii_case("calendar") => true,
        _ => return None,
    };
    if let Some(date) = start.filter(|date| !is_iso_date(date)) {
        return Some(Err(BooksError::Date(date.to_string())));
    }
    Some(Ok(Deferral {
        target_account: target.0,
        service_start: start.map(str::to_string),
        term: Term::Months(months),
        proration: if calendar {
            Proration::Calendar
        } else {
            Proration::Anniversary
        },
        contract: None,
    }))
}

impl fmt::Display for BooksEntry {
    /// The canonical cell text: `a`, or `a → b, N months[ from D][ calendar]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(account) => write!(f, "{account}"),
            Self::Deferred { account, deferral } => {
                write!(f, "{account} → {}", deferral.target_account)?;
                match &deferral.term {
                    Term::Months(n) => write!(f, ", {n} months")?,
                    Term::Through(end) => write!(f, ", through {end}")?,
                }
                if let Some(start) = &deferral.service_start {
                    write!(f, " from {start}")?;
                }
                if deferral.proration == Proration::Calendar {
                    write!(f, " calendar")?;
                }
                Ok(())
            }
        }
    }
}

/// `YYYY-MM-DD` naming a real day: ledgeline-core's check, so the books
/// and the invoicer agree on what a date is.
pub use ledgeline_core::accrual::is_iso_date;

#[cfg(test)]
mod tests {
    use super::*;

    fn deferred(text: &str) -> (String, Deferral) {
        match BooksEntry::parse(text) {
            Ok(Some(BooksEntry::Deferred { account, deferral })) => {
                (account.as_str().to_string(), deferral)
            }
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn plain_and_blank_cells() {
        assert_eq!(BooksEntry::parse("  &nbsp; "), Ok(None));
        assert_eq!(
            BooksEntry::parse(" revenue:consulting "),
            Ok(Some(BooksEntry::Account(
                AccountName::new("revenue:consulting").expect("valid")
            )))
        );
    }

    #[test]
    fn the_worked_example_deferral() {
        let (account, d) =
            deferred("liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01");
        assert_eq!(account, "liabilities:deferred-revenue");
        assert_eq!(d.target_account, "revenue:support");
        assert_eq!(d.term, Term::Months(12));
        assert_eq!(d.service_start.as_deref(), Some("2026-12-01"));
        assert_eq!(d.proration, Proration::Anniversary);
    }

    #[test]
    fn ascii_arrow_singular_month_and_calendar() {
        let (_, d) = deferred("a:b -> c:d, 1 Month calendar");
        assert_eq!(d.term, Term::Months(1));
        assert_eq!(d.service_start, None);
        assert_eq!(d.proration, Proration::Calendar);
    }

    #[test]
    fn the_target_runs_to_the_last_comma() {
        let (_, d) = deferred("a → revenue:x, y, 3 months");
        assert_eq!(d.target_account, "revenue:x, y");
    }

    #[test]
    fn display_round_trips() {
        for text in [
            "revenue:consulting",
            "liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01 calendar",
        ] {
            let entry = BooksEntry::parse(text).expect("parses").expect("present");
            assert_eq!(entry.to_string(), text);
        }
    }

    #[test]
    fn bad_cells_say_why() {
        for (text, needle) in [
            ("a → b", "not a deferral"),
            ("a → b, twelve months", "not a deferral"),
            ("a → b, 0 months", "not a deferral"),
            ("a → b, 3 weeks", "not a deferral"),
            ("a → b, 3 months from 2026-02-30", "not a date"),
            ("a → b, 3 months calendar extra", "not a deferral"),
            ("revenue  x", "two spaces"),
            ("(revenue)", "virtual"),
            ("x ; y", "comment"),
        ] {
            let error = BooksEntry::parse(text).expect_err(text).to_string();
            assert!(error.contains(needle), "{text}: {error}");
        }
    }

    #[test]
    fn iso_dates() {
        assert!(is_iso_date("2024-02-29"));
        assert!(!is_iso_date("2025-02-29"));
        assert!(!is_iso_date("2025-13-01"));
        assert!(!is_iso_date("2025-1-01"));
    }
}
