//! Numbering (ARCHITECTURE §5.1): the next number is the largest in use plus
//! one, where "plus one" increments the trailing run of digits, so
//! `2026-0042` is followed by `2026-0043` and `INV-099` by `INV-100`. There
//! is no counter file.
//!
//! "Largest" is natural order: runs of digits compare as numbers, the rest
//! as text, so `105` < `2026-0042` and `INV-9` < `INV-10`. Numbers with no
//! trailing digits (`2026-A`) can't be incremented and are skipped.
//!
//! The numbers in use come from the files and, in Phase 2, from the books'
//! `invoice:` tags ([`next_number`]'s `books` argument is that seam).

use crate::doc::InvoiceNumber;
use std::cmp::Ordering;
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
    let largest = files
        .into_iter()
        .chain(books)
        .map(str::trim)
        .filter(|n| n.ends_with(|c: char| c.is_ascii_digit()))
        .max_by(|a, b| natural_cmp(a, b));
    match largest {
        Some(n) => increment(n)
            .and_then(|next| InvoiceNumber::new(&next))
            .ok_or_else(|| NumberingError::BadStart(n.to_string())),
        None => {
            InvoiceNumber::new(start).ok_or_else(|| NumberingError::BadStart(start.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
