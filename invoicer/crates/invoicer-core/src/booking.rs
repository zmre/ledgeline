//! The booking split: a computed bill table to one line per resolved account
//! (and deferral), as ARCHITECTURE §4.2 describes. Pure data; the books
//! bridge (Phase 2b) turns it into a `DocumentSpec`.
//!
//! - Each item books to its own account: the books cell, else the default
//!   chain ([`resolve`]).
//! - An adjustment with a books cell books there.
//! - An adjustment with a blank books cell takes the account(s) of the row it
//!   applies to: an item's account (so the item nets its own discounts), or,
//!   under a total, the revenue lines making up that total (Patrick,
//!   2026-11-19): only what came from item rows, deferred items included,
//!   never tax, the deposit or other rows booked to their own account. It is
//!   spread pro rata by that revenue, rounded half away from zero, with the
//!   remainder on the largest line. A set `[invoicer] discount-account` takes
//!   negative ones instead.
//! - Lines with the same account and deferral merge, in first-appearance
//!   order; lines that net to zero are dropped.
//! - The lines sum to the table's total exactly, which is the receivable.

use crate::table::books::{AccountName, BooksEntry};
use crate::table::money::MoneyStyle;
use crate::table::{EvaluatedRow, Evaluation, ItemCode, RowKind, RowRef};
use ledgeline_core::Dec;
use std::collections::BTreeMap;
use thiserror::Error;

/// The defaults a blank books cell falls back through (ARCHITECTURE §3.3,
/// steps 2–5), plus `[invoicer] discount-account`. Discovering them —
/// reading the catalog, the customer note, the settings and the journal's
/// account types — is the caller's job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountDefaults {
    /// Step 2: the catalog's books entry per item code, keyed in ASCII
    /// upper case (codes match case-insensitively).
    pub catalog: BTreeMap<ItemCode, BooksEntry>,
    /// Step 3: the customer's `revenue_account`.
    pub customer: Option<BooksEntry>,
    /// Step 4: `[invoicer] revenue-account`.
    pub settings: Option<AccountName>,
    /// Step 5: the single account typed `R` or carrying `kpi: revenue`.
    pub inferred: Option<AccountName>,
    /// `[invoicer] discount-account`: where blank-account discounts go.
    pub discount: Option<AccountName>,
}

/// Where a row's account came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSource {
    /// The row's books cell.
    Written,
    /// The catalog entry for the row's item code.
    Catalog,
    /// The customer's revenue account.
    Customer,
    /// `[invoicer] revenue-account`.
    Settings,
    /// The journal's single revenue account.
    Inferred,
}

/// A row's own account: its books cell, else the first default that
/// applies. `None` when nothing resolves, which is an error the editor shows.
/// (Step 1, an adjustment taking its target's accounts, is [`split`]'s.)
#[must_use]
pub fn resolve(
    row: &EvaluatedRow,
    defaults: &AccountDefaults,
) -> Option<(BooksEntry, AccountSource)> {
    row.books
        .clone()
        .map(|entry| (entry, AccountSource::Written))
        .or_else(|| {
            row.item
                .as_ref()
                .and_then(|code| {
                    // Codes match case-insensitively, as everywhere else in
                    // the catalog.
                    defaults.catalog.get(code).or_else(|| {
                        defaults
                            .catalog
                            .get(&ItemCode(code.0.trim().to_ascii_uppercase()))
                    })
                })
                .map(|entry| (entry.clone(), AccountSource::Catalog))
        })
        .or_else(|| {
            defaults
                .customer
                .clone()
                .map(|entry| (entry, AccountSource::Customer))
        })
        .or_else(|| {
            defaults
                .settings
                .clone()
                .map(|account| (BooksEntry::Account(account), AccountSource::Settings))
        })
        .or_else(|| {
            defaults
                .inferred
                .clone()
                .map(|account| (BooksEntry::Account(account), AccountSource::Inferred))
        })
}

/// One line of the booking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookingLine {
    /// The account, and its deferral when deferred.
    pub books: BooksEntry,
    /// The amount, signed as the table writes it: revenue positive, the
    /// deposit applied negative. The journal posting is its negation.
    pub amount: Dec,
    /// The body rows that contributed, in order.
    pub rows: Vec<usize>,
}

/// A bill table, split for booking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Booking {
    /// The amount due: the receivable debit.
    pub total: Dec,
    /// The lines; they sum to `total`.
    pub lines: Vec<BookingLine>,
}

/// Why a table can't be split.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BookingError {
    /// No account resolves for the row.
    #[error(
        "{0}: no account; fill in its books column, or set a customer or [invoicer] revenue-account"
    )]
    Unresolved(RowRef),
    /// A blank-account adjustment under a total that sums to zero.
    #[error(
        "{0}: the total above it is zero, so there is nothing to spread it over; give it an account"
    )]
    CannotSpread(RowRef),
    /// A row below the final total, which the total (the amount due) leaves out.
    #[error("{0}: it is below the final total, which doesn't include it")]
    BelowTotal(RowRef),
    /// The arithmetic overflowed.
    #[error("the amounts are too large")]
    Overflow,
}

/// The split's running state.
#[derive(Debug, Clone, Default)]
struct Split {
    lines: Vec<BookingLine>,
    /// Per line: the part of its amount that came from item rows (the
    /// items, adjustments netted into them, and earlier spreads). These are
    /// the weights a blank adjustment under a total spreads by.
    revenue: Vec<Dec>,
    /// Per body row: the line an item went to.
    item_line: Vec<Option<usize>>,
    /// Per body row: for a total, every line's revenue when it was reached.
    snapshot: Vec<Option<Vec<Dec>>>,
}

/// What a posting is, for the pro-rata weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Feed {
    /// Item-derived: an item, or an adjustment netted into one, or a
    /// spread share. Counts toward a later spread.
    Revenue,
    /// A row booked to its own account (tax, a deposit, a discount account,
    /// a defaulted untargeted fixed adjustment). Never shares a spread.
    Own,
}

impl Split {
    fn post(
        mut self,
        books: BooksEntry,
        amount: Dec,
        row: usize,
        feed: Feed,
    ) -> Result<(Self, usize), BookingError> {
        let at = match self.lines.iter().position(|line| line.books == books) {
            Some(at) => at,
            None => {
                self.lines.push(BookingLine {
                    books,
                    amount: Dec::zero(),
                    rows: Vec::new(),
                });
                self.revenue.push(Dec::zero());
                self.lines.len() - 1
            }
        };
        if feed == Feed::Revenue {
            self.revenue[at] = self.revenue[at]
                .add(amount)
                .map_err(|_| BookingError::Overflow)?;
        }
        let line = &mut self.lines[at];
        line.amount = line
            .amount
            .add(amount)
            .map_err(|_| BookingError::Overflow)?;
        if line.rows.last() != Some(&row) {
            line.rows.push(row);
        }
        Ok((self, at))
    }

    fn spread(
        self,
        amount: Dec,
        weights: &[Dec],
        at: &RowRef,
        row: usize,
        precision: u32,
    ) -> Result<Self, BookingError> {
        pro_rata(amount, weights, precision)?
            .ok_or_else(|| BookingError::CannotSpread(at.clone()))?
            .into_iter()
            .filter(|(_, share)| !share.is_zero())
            .try_fold(self, |split, (line, share)| {
                let books = split.lines[line].books.clone();
                split
                    .post(books, share, row, Feed::Revenue)
                    .map(|(split, _)| split)
            })
    }
}

/// Split `amount` over the non-zero `weights` (indexed by line) in
/// proportion, at `precision`: Ledgeline's
/// [`pro_rata`](ledgeline_core::decimal::pro_rata) rule (shares rounded
/// half away from zero, the remainder on the largest weight), the one cash
/// re-splits use too, so both put the odd cent on the same line. `None` when the weights sum to
/// zero.
fn pro_rata(
    amount: Dec,
    weights: &[Dec],
    precision: u32,
) -> Result<Option<Vec<(usize, Dec)>>, BookingError> {
    let nonzero: Vec<(usize, Dec)> = weights
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, w)| !w.is_zero())
        .collect();
    let sum = nonzero
        .iter()
        .try_fold(Dec::zero(), |acc, (_, w)| acc.add(*w))
        .map_err(|_| BookingError::Overflow)?;
    if sum.is_zero() {
        // The caller turns this into `CannotSpread` with the row.
        return Ok(None);
    }
    let amounts: Vec<Dec> = nonzero.iter().map(|(_, w)| *w).collect();
    let shares = ledgeline_core::decimal::pro_rata(amount, &amounts, Some(precision))
        .map_err(|_| BookingError::Overflow)?;
    Ok(Some(
        nonzero
            .into_iter()
            .zip(shares)
            .map(|((line, _), share)| (line, share))
            .collect(),
    ))
}

/// Split a computed bill table into booking lines.
///
/// # Errors
/// [`BookingError`] naming the row: no account resolves, a blank-account
/// adjustment under a zero total, or a row below the final total.
pub fn split(
    evaluation: &Evaluation,
    defaults: &AccountDefaults,
    money: &MoneyStyle,
) -> Result<Booking, BookingError> {
    let rows = &evaluation.rows;
    if let Some(stray) = evaluation.final_total.and_then(|last| {
        rows.iter()
            .skip(last + 1)
            .find(|row| row.kind != RowKind::Spacer)
    }) {
        return Err(BookingError::BelowTotal(stray.at.clone()));
    }
    let start = Split {
        item_line: vec![None; rows.len()],
        snapshot: vec![None; rows.len()],
        ..Split::default()
    };
    let done = rows.iter().enumerate().try_fold(start, |split, (i, row)| {
        book_row(split, i, row, rows, defaults, money.precision())
    })?;
    let lines: Vec<BookingLine> = done
        .lines
        .into_iter()
        .filter(|line| !line.amount.is_zero())
        .collect();
    Ok(Booking {
        total: evaluation.total,
        lines,
    })
}

fn book_row(
    mut split: Split,
    i: usize,
    row: &EvaluatedRow,
    rows: &[EvaluatedRow],
    defaults: &AccountDefaults,
    precision: u32,
) -> Result<Split, BookingError> {
    let unresolved = || BookingError::Unresolved(row.at.clone());
    let amount = row.amount.unwrap_or_else(Dec::zero);
    match row.kind {
        RowKind::Spacer => Ok(split),
        RowKind::Total => {
            split.snapshot[i] = Some(split.revenue.clone());
            Ok(split)
        }
        RowKind::Item => {
            let (books, _) = resolve(row, defaults).ok_or_else(unresolved)?;
            let (mut split, line) = split.post(books, amount, i, Feed::Revenue)?;
            split.item_line[i] = Some(line);
            Ok(split)
        }
        RowKind::Adjustment(_) | RowKind::FixedAdjustment => {
            if let Some(books) = row.books.clone() {
                return split
                    .post(books, amount, i, Feed::Own)
                    .map(|(split, _)| split);
            }
            if let Some(discount) = defaults.discount.clone().filter(|_| amount.mantissa < 0) {
                return split
                    .post(BooksEntry::Account(discount), amount, i, Feed::Own)
                    .map(|(split, _)| split);
            }
            match row.applies_to.map(|t| (t, rows[t].kind)) {
                Some((t, RowKind::Item)) => {
                    let line = split.item_line[t].ok_or_else(unresolved)?;
                    let books = split.lines[line].books.clone();
                    split
                        .post(books, amount, i, Feed::Revenue)
                        .map(|(split, _)| split)
                }
                Some((t, RowKind::Total)) => {
                    if amount.is_zero() {
                        return Ok(split);
                    }
                    let weights = split.snapshot[t].clone().unwrap_or_default();
                    if weights.iter().all(Dec::is_zero) {
                        return Err(BookingError::CannotSpread(row.at.clone()));
                    }
                    split.spread(amount, &weights, &row.at, i, precision)
                }
                _ => {
                    let (books, _) = resolve(row, defaults).ok_or_else(unresolved)?;
                    split
                        .post(books, amount, i, Feed::Own)
                        .map(|(split, _)| split)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro_rata_rounds_and_puts_the_remainder_on_the_largest() {
        // $100.00 over 1:1:1 → 33.33 each, the extra cent on the first
        // largest.
        let shares = pro_rata(
            Dec::new(10_000, 2),
            &[Dec::new(1, 0), Dec::new(1, 0), Dec::new(1, 0)],
            2,
        )
        .expect("splits")
        .expect("non-zero weights");
        assert_eq!(
            shares,
            [
                (0, Dec::new(3334, 2)),
                (1, Dec::new(3333, 2)),
                (2, Dec::new(3333, 2))
            ]
        );
        // The worked example's 2%: -405 over 12,000 / 2,250 / 6,000.
        let shares = pro_rata(
            Dec::new(-40_500, 2),
            &[
                Dec::new(12_000, 0),
                Dec::new(225_000, 2),
                Dec::new(6_000, 0),
            ],
            2,
        )
        .expect("splits")
        .expect("non-zero weights");
        assert_eq!(
            shares.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
            [Dec::new(-240, 0), Dec::new(-45, 0), Dec::new(-120, 0)]
        );
    }

    #[test]
    fn booking_spreads_and_cash_re_splits_put_the_odd_cents_on_the_same_line() {
        // RC-11: one rule, so a $100 discount over these lines and a cash
        // re-split of the same rows agree to the cent.
        let weights = [
            Dec::new(30_000, 2),
            Dec::new(70_000, 2),
            Dec::new(30_000, 2),
        ];
        let spread = pro_rata(Dec::new(-10_000, 2), &weights, 2)
            .expect("splits")
            .expect("non-zero weights");
        let named: Vec<(String, Dec)> = weights
            .iter()
            .enumerate()
            .map(|(at, w)| (format!("revenue:{at}"), *w))
            .collect();
        let rescaled = crate::books::spec::scale(Dec::new(-10_000, 2), &named).expect("re-splits");
        assert_eq!(
            spread.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
            rescaled.iter().map(|(_, s)| *s).collect::<Vec<_>>()
        );
        assert_eq!(
            spread.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
            [Dec::new(-2308, 2), Dec::new(-5384, 2), Dec::new(-2308, 2)]
        );
    }

    #[test]
    fn weights_summing_to_zero_cannot_be_spread() {
        assert_eq!(
            pro_rata(Dec::new(500, 2), &[Dec::new(10, 0), Dec::new(-10, 0)], 2),
            Ok(None)
        );
    }
}
