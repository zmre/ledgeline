//! Balance-assertion evaluation — the post-parse pass that makes `= AMOUNT`
//! mean something.
//!
//! [`crate::parse`] reads the four assertion operators, [`crate::model`] stores
//! them and [`crate::wire`] emits them, but until this module nothing ever
//! *checked* one: a journal asserting a balance of `$99.00` where the running
//! balance was `$1.00` parsed clean and fed every report. Balance assertions are
//! the reconciliation guard against exactly the class of error this engine
//! exists to catch, so evaluating them is not optional.
//!
//! # Semantics
//!
//! Verified against hledger 1.52 (`hledger -f FILE check assertions`); each rule
//! below is pinned by a test in `tests/balance_assertions.rs`.
//!
//! **Ordering.** Every posting in the journal is placed in one global sequence
//! ordered by its *effective date* — the posting's own `date:` tag when it has
//! one, otherwise its transaction's primary date — with ties broken by file read
//! order (transaction order, then posting order within the transaction). This is
//! a **posting**-level sort, not a transaction-level one: hledger will reorder
//! two postings *within a single transaction* if they carry different `date:`
//! tags, and will interleave postings from different transactions. `date2` never
//! participates.
//!
//! **Scope.** An assertion is evaluated against the running balance *after* its
//! own posting has been added. Real, virtual (`(a)`) and balanced-virtual
//! (`[a]`) postings all contribute. Costs (`@`/`@@`) are ignored entirely — the
//! assertion sees the posting's own commodity and quantity.
//!
//! **The four operators.**
//!
//! | Written | Commodities checked | Accounts summed |
//! |---------|---------------------|-----------------|
//! | `=`     | the asserted one    | this account only |
//! | `==`    | all (see below)     | this account only |
//! | `=*`    | the asserted one    | this account + subaccounts |
//! | `==*`   | all (see below)     | this account + subaccounts |
//!
//! `==` additionally asserts that every *other* commodity is zero. The set of
//! "other commodities" is drawn from the account's **own** (exclusive) balance
//! even for the inclusive `==*` form — a quirk of hledger's implementation that
//! this module reproduces deliberately, because journals in the wild depend on
//! it. Two consequences, both pinned by tests:
//!
//! - a commodity held only in a *subaccount* is invisible to `==*`'s zero check;
//! - a commodity that has netted to exactly zero in the account's own balance is
//!   still *present* there, so `==*` does check it inclusively — which is why
//!   the running balance below must retain zeroed commodities.
//!
//! # Not a parse error
//!
//! This pass returns a list of failures rather than aborting. See
//! [`check_balance_assertions`] for why.

use crate::decimal::{Dec, DecError};
use crate::model::{
    AccountName, AmountStyle, BalanceAssertion, Commodity, CommoditySide, Journal, Posting,
    SourcePos, Transaction,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One account's running balance, commodity → exact quantity.
///
/// Deliberately **not** [`crate::reports::MixedAmount`]: that type's documented
/// contract is that a commodity netting to exactly zero is dropped, and the
/// `==`/`==*` zero check depends on such a commodity *remaining* present (see
/// the module docs). Reusing it would mean depending on `accumulate` never
/// pruning — an unstated property of a type whose stated contract is the
/// opposite — and would also invert the layering, since this is a parse-time
/// check and `reports` consumes an already-checked journal.
type AccountBalance = BTreeMap<Commodity, Dec>;

/// Every account's running balance, keyed by full account name.
///
/// A `BTreeMap` so the subaccount scan and the `==` commodity iteration are
/// deterministic.
type Balances = BTreeMap<String, AccountBalance>;

/// A balance assertion that did not hold.
///
/// Carries the structured facts rather than a pre-rendered string so a caller
/// can surface it as a `Problem`-shaped diagnostic, an HTTP payload, or the
/// hledger-style text of [`Display`](std::fmt::Display).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionFailure {
    /// The file the asserting posting was parsed from. For a posting in an
    /// `include`d file this is the included file, matching
    /// [`Transaction::source_file`] — and matching which path hledger names.
    pub source_file: PathBuf,
    /// Position of the `=` sign, relative to [`Self::source_file`].
    pub position: SourcePos,
    /// The asserting transaction's primary date, for a caller that wants to
    /// point at the transaction rather than the line.
    pub transaction_date: String,
    /// 0-based index of the asserting transaction in [`Journal::transactions`].
    ///
    /// The failures come back in hledger's evaluation order, which is NOT file
    /// order, so a caller that wants to flag the offending row needs the
    /// position told to it rather than inferred from this list.
    pub transaction_index: usize,
    /// The account whose balance was asserted.
    pub account: AccountName,
    /// The commodity this particular failure is about. For a `==` failure this
    /// may be a commodity the user never named — the operator asserts every
    /// *other* commodity is zero, and this is one that was not.
    pub commodity: Commodity,
    /// What the journal claimed the balance was.
    pub asserted: Dec,
    /// What the running balance actually was.
    pub calculated: Dec,
    /// Subaccount-inclusive (`=*` / `==*`).
    pub inclusive: bool,
    /// Total, i.e. all-commodities (`==` / `==*`).
    pub total: bool,
    /// The commodity's display style, used only to render [`Self::asserted`] and
    /// [`Self::calculated`] in messages. Never consulted when comparing.
    pub style: AmountStyle,
}

impl AssertionFailure {
    /// The assertion as written: `=`, `==`, `=*` or `==*`.
    #[must_use]
    pub fn operator(&self) -> &'static str {
        match (self.total, self.inclusive) {
            (true, true) => "==*",
            (true, false) => "==",
            (false, true) => "=*",
            (false, false) => "=",
        }
    }

    /// The diagnostic body **without** the leading `file:line:col`, for a caller
    /// that supplies its own location prefix (e.g. wrapping this in
    /// [`crate::parse::ParseError::Located`], whose `Display` already prints
    /// one).
    #[must_use]
    pub fn message(&self) -> String {
        let scope = if self.total {
            "Across all commodities".to_owned()
        } else {
            format!("In commodity {}", self.commodity.0)
        };
        let subaccounts = if self.inclusive {
            "including"
        } else {
            "excluding"
        };
        let difference = self.asserted.sub(self.calculated).map_or_else(
            |_| String::new(),
            |diff| format!("\n(difference: {})", self.render(diff)),
        );
        format!(
            "balance assertion failed in {}\n\
             {scope} at this point, {subaccounts} subaccounts, ignoring costs,\n\
             the asserted balance is:       {}\n\
             but the calculated balance is: {}{difference}",
            self.account.0,
            self.render(self.asserted),
            self.render(self.calculated),
        )
    }

    /// Render a quantity in this failure's commodity, **exactly** — see
    /// [`render_amount`].
    fn render(&self, quantity: Dec) -> String {
        render_amount(quantity, &self.commodity, &self.style)
    }
}

/// Render `quantity` in `commodity`, **exactly** — every digit the `Dec` holds,
/// no rounding and no digit grouping. Only the style's side/spacing/decimal-mark
/// are honoured, so a display precision can never make two values that differ
/// look identical in a message that says they do.
///
/// Shared with [`crate::parse`]'s unbalanced-transaction diagnostic, which has
/// the same requirement.
pub(crate) fn render_amount(quantity: Dec, commodity: &Commodity, style: &AmountStyle) -> String {
    let number = render_dec(quantity, style.decimal_mark.unwrap_or('.'));
    let symbol = &commodity.0;
    if symbol.is_empty() {
        return number;
    }
    match (style.side, style.spaced) {
        (CommoditySide::Left, false) => format!("{symbol}{number}"),
        (CommoditySide::Left, true) => format!("{symbol} {number}"),
        (CommoditySide::Right, false) => format!("{number}{symbol}"),
        (CommoditySide::Right, true) => format!("{number} {symbol}"),
    }
}

impl std::fmt::Display for AssertionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.source_file.display(),
            self.position.line,
            self.position.column,
            self.message()
        )
    }
}

impl std::error::Error for AssertionFailure {}

/// The widest fractional precision [`render_dec`] will lay out. Matches the
/// clamp in `edit.rs`: 255 is hledger's own maximum displayed precision, so this
/// cannot truncate a value hledger could have written.
const MAX_RENDER_PLACES: u32 = 255;

/// Render a [`Dec`] exactly, using `mark` as the decimal separator:
/// `Dec::new(-100, 2)` → `-1.00`, `Dec::new(5, 3)` → `0.005`.
///
/// Total by construction: the zero padding uses [`str::repeat`] rather than a
/// format width (which is a `u16` and panics past 65535 places), and `places` is
/// clamped so the output stays a few hundred bytes.
fn render_dec(value: Dec, mark: char) -> String {
    let sign = if value.mantissa < 0 { "-" } else { "" };
    let digits = value.mantissa.unsigned_abs().to_string();
    let places = value.places.min(MAX_RENDER_PLACES) as usize;
    if places == 0 {
        return format!("{sign}{digits}");
    }
    // Guarantee at least one integer digit before the mark, so `padded.len()`
    // exceeds `places` and the split below is in range. Every byte is ASCII, so
    // it is also on a char boundary.
    let padded = match (places + 1).checked_sub(digits.len()) {
        Some(zeros) if zeros > 0 => "0".repeat(zeros) + &digits,
        _ => digits,
    };
    let split = padded.len() - places;
    format!("{sign}{}{mark}{}", &padded[..split], &padded[split..])
}

/// How to render each commodity in a diagnostic.
///
/// [`Journal::commodity_styles`] carries only the styles a `commodity`/`D`
/// directive declared, so an undeclared commodity would otherwise be rendered
/// with some *other* commodity's style — printing `EUR0` where hledger prints
/// `0 EUR`. Declared styles win; every remaining commodity takes the style of
/// its first occurrence in file order, which is hledger's own rule.
///
/// Presentation only: nothing here participates in a comparison. Shared with
/// [`crate::parse`]'s unbalanced-transaction diagnostic.
pub(crate) fn display_styles(journal: &Journal) -> BTreeMap<&Commodity, &AmountStyle> {
    let mut styles: BTreeMap<&Commodity, &AmountStyle> = journal
        .commodity_styles
        .iter()
        .map(|(commodity, style)| (commodity, style))
        .collect();
    let amounts = journal
        .transactions
        .iter()
        .flat_map(|transaction| &transaction.postings)
        .flat_map(|posting| {
            posting.amounts.iter().chain(
                posting
                    .balance_assertion
                    .iter()
                    .map(|assertion| &assertion.amount),
            )
        })
        // A commodity that only ever appears as a COST (`10 AAPL @ $5.00` in a
        // journal that never writes a bare `$` amount) still needs a style: it
        // is what an unbalanced-transaction residual is reported in.
        .flat_map(|amount| {
            std::iter::once(amount).chain(amount.cost.iter().map(|cost| &cost.amount))
        });
    for amount in amounts {
        styles.entry(&amount.commodity).or_insert(&amount.style);
    }
    styles
}

/// One posting placed in the journal-wide evaluation order.
struct Entry<'a> {
    /// Effective date: the posting's `date:` tag, else the transaction's date.
    date: &'a str,
    transaction: &'a Transaction,
    /// 0-based position of `transaction` in `Journal::transactions`, carried
    /// through the sort so a failure can name the row it belongs to.
    transaction_index: usize,
    posting: &'a Posting,
}

/// Evaluate every balance assertion in `journal`, returning the ones that fail.
///
/// # Diagnostics, not a hard error
///
/// This returns a list instead of `Err`-ing on the first failure, for three
/// reasons specific to how this codebase surfaces parse errors:
///
/// 1. **Blast radius.** `parse_journal` backs the live-reload watcher and the
///    editor's reparse-to-validate guard. A failing assertion promoted to a
///    `ParseError` would make a previously-openable journal refuse to open, make
///    the watcher silently keep serving stale data (and unbind the editor, so
///    write endpoints start answering 501), and — because the guard reparses the
///    *whole* journal — make one stale assertion anywhere reject every unrelated
///    edit everywhere.
/// 2. **Precedent.** The engine already has a collect-and-continue channel for
///    exactly this shape of problem: `HoldingsReport::warnings`. The frontend
///    has a journal-wide one in `web/src/lib/checks/` (`Problem` / `CheckRule`).
///    Fail-fast `ParseError` is reserved for journals that cannot be *read*; a
///    journal with a failing assertion is perfectly readable and its other
///    numbers are still worth showing.
/// 3. **Completeness.** A reconciliation pass that reports the first break and
///    hides the rest is a worse reconciliation tool.
///
/// The trade-off is that a returned-and-ignored failure is no more visible than
/// no check at all. A caller **must** surface these; see the module docs.
///
/// Failures come back in evaluation order, and within one posting the explicitly
/// asserted commodity comes first, then any `==` zero-check commodities in
/// lexical order — so the first failure for a given posting is the one hledger
/// would have reported before aborting.
///
/// # Errors
/// Returns [`DecError`] only if summing a running balance overflows `i128`.
pub fn check_balance_assertions(journal: &Journal) -> Result<Vec<AssertionFailure>, DecError> {
    let styles = display_styles(journal);

    let mut entries: Vec<Entry> = journal
        .transactions
        .iter()
        .enumerate()
        .flat_map(|(transaction_index, transaction)| {
            transaction.postings.iter().map(move |posting| Entry {
                date: posting.date.as_deref().unwrap_or(&transaction.date),
                transaction,
                transaction_index,
                posting,
            })
        })
        .collect();
    // ISO `YYYY-MM-DD` sorts lexically exactly as it sorts chronologically, and
    // `sort_by_key` is stable — so postings sharing a date keep the order they
    // were built in, which is transaction read order (`include`d files land at
    // the point of their `include` directive) then posting order.
    entries.sort_by_key(|entry| entry.date);

    let mut balances = Balances::new();
    let mut failures = Vec::new();
    for entry in entries {
        let account = &entry.posting.account;
        {
            let own = balances.entry(account.0.clone()).or_default();
            for amount in &entry.posting.amounts {
                // `amount.cost` is deliberately untouched: assertions ignore costs.
                let running = own
                    .entry(amount.commodity.clone())
                    .or_insert_with(Dec::zero);
                *running = running.add(amount.quantity)?;
            }
        }
        if let Some(assertion) = &entry.posting.balance_assertion {
            check_one(
                &balances,
                account,
                assertion,
                entry.transaction,
                entry.transaction_index,
                &styles,
                &mut failures,
            )?;
        }
    }
    Ok(failures)
}

/// Check a single assertion against the balances as they stand *after* its own
/// posting has been applied, appending any failures to `out`.
fn check_one(
    balances: &Balances,
    account: &AccountName,
    assertion: &BalanceAssertion,
    transaction: &Transaction,
    transaction_index: usize,
    styles: &BTreeMap<&Commodity, &AmountStyle>,
    out: &mut Vec<AssertionFailure>,
) -> Result<(), DecError> {
    let own = balances.get(&account.0);
    let subject = if assertion.inclusive {
        inclusive_balance(balances, &account.0)?
    } else {
        own.cloned().unwrap_or_default()
    };
    let balance_of =
        |commodity: &Commodity| subject.get(commodity).copied().unwrap_or_else(Dec::zero);

    let failure = |commodity: &Commodity, asserted: Dec, calculated: Dec| AssertionFailure {
        source_file: transaction.source_file.clone(),
        position: assertion.position,
        transaction_date: transaction.date.clone(),
        transaction_index,
        account: account.clone(),
        commodity: commodity.clone(),
        asserted,
        calculated,
        inclusive: assertion.inclusive,
        total: assertion.total,
        style: styles
            .get(commodity)
            .map_or_else(|| assertion.amount.style.clone(), |style| (*style).clone()),
    };

    // The commodity the user actually named. `Dec` compares by exact numeric
    // value, so `$1.0` and `$1.00` agree and no display rounding is involved.
    let asserted = &assertion.amount;
    let calculated = balance_of(&asserted.commodity);
    if calculated != asserted.quantity {
        out.push(failure(&asserted.commodity, asserted.quantity, calculated));
    }

    // `==`/`==*` also assert that every other commodity is zero. hledger draws
    // that commodity set from the account's OWN balance even when the operator
    // is inclusive, then evaluates each against the (possibly inclusive)
    // subject balance — reproduced here exactly; see the module docs.
    if assertion.total {
        for commodity in own.iter().flat_map(|balance| balance.keys()) {
            if *commodity == asserted.commodity {
                continue;
            }
            let calculated = balance_of(commodity);
            if !calculated.is_zero() {
                // Render the implied zero at the commodity's display precision so
                // the message reads `$0.00`, not `0`, as hledger's does.
                let places = styles.get(commodity).map_or(0, |style| style.precision);
                out.push(failure(commodity, Dec::new(0, places), calculated));
            }
        }
    }
    Ok(())
}

/// Sum `account`'s own balance with every subaccount's, for the `=*`/`==*` forms.
///
/// Subaccounts are matched on the `:` segment boundary, so `a:x` rolls up into
/// `a` but `ab` does not. A full scan rather than a `BTreeMap` range: keys that
/// share a prefix with `account` are not contiguous after it (`a`, `a0`, `a:x`
/// sort in that order, so a range would stop short at `a0`).
fn inclusive_balance(balances: &Balances, account: &str) -> Result<AccountBalance, DecError> {
    let prefix = format!("{account}:");
    balances
        .iter()
        .filter(|(name, _)| name.as_str() == account || name.starts_with(&prefix))
        .try_fold(AccountBalance::new(), |mut total, (_, own)| {
            for (commodity, quantity) in own {
                let running = total.entry(commodity.clone()).or_insert_with(Dec::zero);
                *running = running.add(*quantity)?;
            }
            Ok(total)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AmountStyle;

    fn style() -> AmountStyle {
        AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: 2,
        }
    }

    fn failure(asserted: Dec, calculated: Dec) -> AssertionFailure {
        AssertionFailure {
            source_file: PathBuf::from("/tmp/t.journal"),
            position: SourcePos {
                line: 2,
                column: 15,
            },
            transaction_date: "2024-01-01".to_owned(),
            transaction_index: 0,
            account: AccountName("a".to_owned()),
            commodity: Commodity("$".to_owned()),
            asserted,
            calculated,
            inclusive: false,
            total: false,
            style: style(),
        }
    }

    #[test]
    fn render_dec_lays_out_exact_digits() {
        assert_eq!(render_dec(Dec::new(-100, 2), '.'), "-1.00");
        assert_eq!(render_dec(Dec::new(5, 3), '.'), "0.005");
        assert_eq!(render_dec(Dec::new(1234, 0), '.'), "1234");
        assert_eq!(render_dec(Dec::new(150, 2), ','), "1,50");
        assert_eq!(render_dec(Dec::zero(), '.'), "0");
    }

    #[test]
    fn render_dec_is_total_for_absurd_precision() {
        // A format-width layout would panic here; `str::repeat` plus the clamp
        // keeps it total and bounded.
        let rendered = render_dec(Dec::new(1, u32::MAX), '.');
        assert_eq!(rendered.len(), MAX_RENDER_PLACES as usize + 2);
    }

    #[test]
    fn message_matches_hledgers_wording() {
        let rendered = failure(Dec::new(9900, 2), Dec::new(100, 2)).to_string();
        assert_eq!(
            rendered,
            "/tmp/t.journal:2:15: balance assertion failed in a\n\
             In commodity $ at this point, excluding subaccounts, ignoring costs,\n\
             the asserted balance is:       $99.00\n\
             but the calculated balance is: $1.00\n\
             (difference: $98.00)"
        );
    }

    #[test]
    fn total_message_names_all_commodities() {
        let total = AssertionFailure {
            total: true,
            inclusive: true,
            ..failure(Dec::new(0, 2), Dec::new(1000, 2))
        };
        assert!(
            total
                .message()
                .contains("Across all commodities at this point, including subaccounts"),
            "{}",
            total.message()
        );
        assert_eq!(total.operator(), "==*");
    }

    #[test]
    fn difference_is_asserted_minus_calculated() {
        // hledger reports `asserted - calculated`, so an under-count is positive.
        assert!(
            failure(Dec::new(0, 2), Dec::new(1000, 2))
                .message()
                .contains("(difference: $-10.00)")
        );
    }

    #[test]
    fn overflowing_difference_drops_the_line_rather_than_panicking() {
        let extreme = failure(Dec::new(i128::MAX, 0), Dec::new(i128::MIN, 0));
        let message = extreme.message();
        assert!(!message.contains("difference"), "{message}");
        assert!(message.contains("balance assertion failed in a"));
    }

    #[test]
    fn commodityless_amounts_render_bare() {
        let bare = AssertionFailure {
            commodity: Commodity(String::new()),
            ..failure(Dec::new(0, 0), Dec::new(100, 2))
        };
        assert!(
            bare.message()
                .contains("the asserted balance is:       0\n")
        );
        assert!(
            bare.message()
                .contains("but the calculated balance is: 1.00")
        );
    }
}
