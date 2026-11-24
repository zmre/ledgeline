//! Numbering (ARCHITECTURE §5.1): the next number is the largest in use plus
//! one, where "plus one" increments the trailing run of digits, so
//! `2026-0042` is followed by `2026-0043` and `INV-099` by `INV-100`. There
//! is no counter file.
//!
//! "Largest" is natural order: runs of digits compare as numbers, the rest
//! as text, so `105` < `2026-0042` and `INV-9` < `INV-10`. Numbers with no
//! trailing digits (`2026-A`) can't be incremented and are skipped.
//!
//! The numbers in use come from the files and from the books' `invoice:`
//! tags ([`booked_numbers`], [`next_number`]'s `books` argument).

use crate::doc::InvoiceNumber;
use ledgeline_core::Journal;
use ledgeline_core::accrual::documents::document_posting;
use ledgeline_core::accrual::{AccrualRole, AccrualRoles, INVOICE_TAG};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::HashSet;
use thiserror::Error;

/// Why no next number could be made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NumberingError {
    /// `[invoicer] number-start` can't be a number.
    #[error("[invoicer] number-start = {0:?} can't be an invoice number")]
    BadStart(String),
}

/// One chunk of a number: a digit run or other text.
#[derive(Debug, PartialEq, Eq)]
enum Chunk<'a> {
    Digits(&'a str),
    Text(&'a str),
}

fn chunks(text: &str) -> Vec<Chunk<'_>> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    while start < bytes.len() {
        let digit = bytes[start].is_ascii_digit();
        let end = bytes[start..]
            .iter()
            .position(|b| b.is_ascii_digit() != digit)
            .map_or(bytes.len(), |n| start + n);
        // Byte boundaries only fall on ASCII digit transitions, which are
        // always char boundaries.
        let piece = &text[start..end];
        out.push(if digit {
            Chunk::Digits(piece)
        } else {
            Chunk::Text(piece)
        });
        start = end;
    }
    out
}

/// Two digit runs as numbers, without overflow: fewer significant digits is
/// smaller; then text order; then the longer (zero-padded) spelling last.
fn cmp_digits(a: &str, b: &str) -> Ordering {
    let (sa, sb) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    sa.len()
        .cmp(&sb.len())
        .then_with(|| sa.cmp(sb))
        .then_with(|| a.len().cmp(&b.len()))
}

/// Natural order of two numbers.
#[must_use]
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (ca, cb) = (chunks(a), chunks(b));
    ca.iter()
        .zip(cb.iter())
        .map(|pair| match pair {
            (Chunk::Digits(x), Chunk::Digits(y)) => cmp_digits(x, y),
            (Chunk::Digits(_), Chunk::Text(_)) => Ordering::Less,
            (Chunk::Text(_), Chunk::Digits(_)) => Ordering::Greater,
            (Chunk::Text(x), Chunk::Text(y)) => x.cmp(y),
        })
        .find(|o| *o != Ordering::Equal)
        .unwrap_or_else(|| ca.len().cmp(&cb.len()))
}

/// `number` with its trailing digit run incremented, keeping its width
/// (`0042` → `0043`, `099` → `100`, `9` → `10`), or `None` when it doesn't
/// end in a digit.
#[must_use]
pub fn increment(number: &str) -> Option<String> {
    let digits = number.len() - number.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let (head, run) = number.split_at(number.len() - digits);
    // Decimal increment on the text: no width limit, no overflow.
    let mut out: Vec<u8> = run.as_bytes().to_vec();
    let carry = out.iter_mut().rev().try_for_each(|d| {
        if *d == b'9' {
            *d = b'0';
            Ok(())
        } else {
            *d += 1;
            Err(())
        }
    });
    let run = String::from_utf8(out).ok()?;
    Some(match carry {
        Ok(()) => format!("{head}1{run}"),
        Err(()) => format!("{head}{run}"),
    })
}

/// Where a next number came from: the number it follows, or the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NumberSource {
    /// It follows a document's number (a file in the books folder).
    Document,
    /// It follows a number booked in the journal (an `invoice:` tag) that no
    /// file has a larger one than: plan 45's invoices, a removed file.
    Books,
    /// Nothing is numbered yet: `[invoicer] number-start`, else `1`.
    Start,
}

impl NumberSource {
    /// The wire name (`document`, `books`, `start`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Books => "books",
            Self::Start => "start",
        }
    }
}

/// The next number and why: the number it follows and where that is, so an
/// app can say "2005, next after 2004 booked in the journal" (Phase 4
/// feedback H19). The rule is [`next_number`]'s; this only explains it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextNumber {
    /// The number proposed.
    pub number: InvoiceNumber,
    /// The largest number in use, which it follows; `None` at the start.
    pub after: Option<String>,
    /// Where `after` is: a document, or only the books. A number that is
    /// both is a document's.
    pub source: NumberSource,
}

/// The next number: the largest incrementable number among `files` and
/// `books`, plus one; or `start` when there is none.
///
/// # Errors
/// [`NumberingError::BadStart`] when `start` is needed and can't be a
/// number.
pub fn next_number<'a>(
    files: impl IntoIterator<Item = &'a str>,
    books: impl IntoIterator<Item = &'a str>,
    start: &str,
) -> Result<InvoiceNumber, NumberingError> {
    explain_next_number(files, books, start).map(|next| next.number)
}

/// [`next_number`] with where it comes from ([`NextNumber`]).
///
/// # Errors
/// [`NumberingError::BadStart`] when `start` is needed and can't be a
/// number.
pub fn explain_next_number<'a>(
    files: impl IntoIterator<Item = &'a str>,
    books: impl IntoIterator<Item = &'a str>,
    start: &str,
) -> Result<NextNumber, NumberingError> {
    // The first of equal largest wins, so a number with a file is the
    // file's even when the books tag it too.
    let largest = files
        .into_iter()
        .map(|n| (n, NumberSource::Document))
        .chain(books.into_iter().map(|n| (n, NumberSource::Books)))
        .map(|(n, source)| (n.trim(), source))
        .filter(|(n, _)| n.ends_with(|c: char| c.is_ascii_digit()))
        .fold(
            None::<(&str, NumberSource)>,
            |best, (n, source)| match best {
                Some((b, _)) if natural_cmp(n, b) != Ordering::Greater => best,
                _ => Some((n, source)),
            },
        );
    match largest {
        Some((n, source)) => increment(n)
            .and_then(|next| InvoiceNumber::new(&next))
            .map(|number| NextNumber {
                number,
                after: Some(n.to_string()),
                source,
            })
            .ok_or_else(|| NumberingError::BadStart(n.to_string())),
        None => InvoiceNumber::new(start)
            .map(|number| NextNumber {
                number,
                after: None,
                source: NumberSource::Start,
            })
            .ok_or_else(|| NumberingError::BadStart(start.to_string())),
    }
}

/// Every `invoice:` tag value in the books, on a transaction or one of its
/// postings, deduplicated in first-use order: the numbers the books have
/// used, whether or not a file still names them (plan 45's invoices, an
/// applied payment, a void reversal). [`next_number`]'s `books`.
///
/// A vendor's bill carries the same tag in its own namespace (plan 45: a
/// bill 105 is not your invoice 105), so transactions whose settlement
/// posting is payable-side are left out.
#[must_use]
pub fn booked_numbers(journal: &Journal) -> Vec<String> {
    let roles = AccrualRoles::of(journal);
    let mut seen: HashSet<&str> = HashSet::new();
    journal
        .transactions
        .iter()
        .filter(|t| !matches!(document_posting(t, &roles), Some((_, AccrualRole::Payable))))
        .flat_map(|t| {
            t.tags
                .iter()
                .chain(t.postings.iter().flat_map(|p| p.tags.iter()))
        })
        .filter(|(name, _)| name == INVOICE_TAG)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty() && seen.insert(value))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn booked_numbers_read_transaction_and_posting_tags() {
        let journal = ledgeline_core::parse_journal(
            "2026-01-01 A  ; invoice: 2001\n    assets:ar  $1\n    revenue\n\n\
             2026-01-02 B\n    assets:bank  $1  ; invoice: 2002\n    revenue  ; invoice: 2001\n\n\
             2026-01-03 C  ; invoiced: 9\n    assets:bank  $1\n    revenue\n\n\
             2026-01-04 Vendor bill  ; invoice: PA-9\n    expenses:rent  $1\n    liabilities:accounts payable\n",
            "books.journal",
        )
        .expect("parses");
        assert_eq!(booked_numbers(&journal), ["2001", "2002"]);
    }

    #[test]
    fn explains_where_the_next_number_comes_from() {
        let books =
            explain_next_number(["105", "108"], ["2001", "2004", " 105"], "1").expect("next");
        assert_eq!(books.number.as_str(), "2005");
        assert_eq!(books.after.as_deref(), Some("2004"));
        assert_eq!(books.source, NumberSource::Books);
        // A number both filed and booked is the document's.
        let doc = explain_next_number(["105", "108"], ["108"], "1").expect("next");
        assert_eq!(
            (doc.number.as_str(), doc.after.as_deref(), doc.source),
            ("109", Some("108"), NumberSource::Document)
        );
        let start = explain_next_number([], [], "1001").expect("next");
        assert_eq!(
            (start.number.as_str(), start.after, start.source),
            ("1001", None, NumberSource::Start)
        );
        assert_eq!(NumberSource::Books.as_str(), "books");
        assert_eq!(
            serde_json::to_value(NumberSource::Document).expect("json"),
            "document"
        );
    }

    fn next(files: &[&str]) -> String {
        next_number(files.iter().copied(), [], "1")
            .expect("next")
            .to_string()
    }

    #[test]
    fn increments_the_trailing_digit_run() {
        assert_eq!(increment("105").as_deref(), Some("106"));
        assert_eq!(increment("2026-0042").as_deref(), Some("2026-0043"));
        assert_eq!(increment("INV-099").as_deref(), Some("INV-100"));
        assert_eq!(increment("999").as_deref(), Some("1000"));
        assert_eq!(increment("A9").as_deref(), Some("A10"));
        assert_eq!(increment("2026-A"), None);
        assert_eq!(
            increment("99999999999999999999999999").as_deref(),
            Some("100000000000000000000000000")
        );
    }

    #[test]
    fn the_largest_in_natural_order_plus_one() {
        assert_eq!(next(&["101", "105", "99"]), "106");
        assert_eq!(next(&["INV-9", "INV-10"]), "INV-11");
        assert_eq!(next(&["105", "2026-0042"]), "2026-0043");
        assert_eq!(next(&["7", "2026-A"]), "8");
        assert_eq!(next(&[]), "1");
        assert_eq!(next_number([], [], "1001").expect("start").as_str(), "1001");
        assert_eq!(
            next_number(["101"], ["2004"], "1").expect("books").as_str(),
            "2005"
        );
        assert!(next_number([], [], "a,b").is_err());
    }

    #[test]
    fn natural_order_details() {
        assert_eq!(natural_cmp("0042", "42"), Ordering::Greater);
        assert_eq!(natural_cmp("a2", "a10"), Ordering::Less);
        assert_eq!(natural_cmp("2026-0042", "2026-0043"), Ordering::Less);
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
    }

    proptest! {
        #[test]
        fn next_is_greater_than_everything(numbers in prop::collection::vec("[A-Z]{0,3}-?[0-9]{1,6}", 1..20)) {
            let refs: Vec<&str> = numbers.iter().map(String::as_str).collect();
            let next = next_number(refs.iter().copied(), [], "1").expect("next");
            let max = refs.iter().copied().max_by(|a, b| natural_cmp(a, b)).expect("one");
            prop_assert_eq!(natural_cmp(next.as_str(), max), Ordering::Greater);
        }

        #[test]
        fn increment_adds_one(n in 0u64..10_000_000) {
            prop_assert_eq!(increment(&n.to_string()), Some((n + 1).to_string()));
        }
    }
}
