//! Spelling a journal's declared commodity styles back out, and proving a
//! re-styling changed nothing but the spelling (WP-11).
//!
//! # The bug this exists for
//!
//! A journal declares `commodity $1,000.00` — comma thousands, two decimals —
//! and an imported statement lands in it as `$165.2` and `$-405`. Verified
//! against hledger 1.52, and none of it is a Ledgeline bug:
//!
//! * **`hledger import` does not apply a commodity's declared decimal places.**
//!   It applies the digit-group separator (`12345.6` is written `$12,345.6`) and
//!   stops there; the fractional digits are whatever the CSV cell held. There is
//!   no flag that changes this — `-c/--commodity-style` does not, and `--round`
//!   is rejected outright (`Unknown flag`) by the one subcommand that writes.
//! * **`hledger print` does apply it**, given `--round` and the directive in
//!   scope. `print --round=soft` over the same entries writes `$-405.00` and
//!   `$165.20`.
//!
//! So the styling cannot be done by the command that writes, and the command
//! that can style does not write. Ledgeline therefore takes the write over
//! itself: preview, re-style, append, and let `hledger import --catchup` record
//! the dedup state. This module is the two pure halves of that — what to
//! prepend, and how to know the result is still the same transactions.
//!
//! # Why `--round=soft` and not `--round=hard`
//!
//! Both produce the padding this is about. `hard` also **rounds**, and hledger's
//! own `--help` says it "can unbalance transactions": with `commodity $1,000.00`
//! in scope, a statement row of `12345.678` is written into the user's books as
//! `$12,345.68`. A number that does not match the bank's is a considerably worse
//! outcome than a number with a missing zero, and the whole point of this change
//! is cosmetic. `soft` only adds or removes decimal **zeros**, so it can never
//! change a value. The caller passes the flag; this note is here because this is
//! where the argument lives.
//!
//! # Why the directives are re-spelled rather than copied
//!
//! The declarations are frequently not in the file being written to. The
//! motivating layout is a root that `include`s an `accounts.journal` holding
//! every `account` and `commodity` line, with transactions split by year — so
//! the file an import appends to declares nothing at all. Ledgeline has already
//! parsed the whole tree, and [`Journal::commodity_styles`] is that parse's
//! answer, so the directives are spelled back out of the model rather than
//! scraped out of however many files they were spread across.
//!
//! Each candidate line is then **re-parsed and compared to the style it came
//! from**, and dropped unless it round-trips exactly. That is what makes
//! [`commodity_directives`] safe to write without enumerating every shape a
//! declared style can take: an exotic digit-group mark or a symbol this module
//! would spell wrongly produces a line that does not parse back to itself, and a
//! line that does not parse back to itself is not emitted.
//!
//! # Why a re-styling has to be checked at all
//!
//! Prepending a directive changes how the entries **parse**, not only how they
//! print, and that is a live hazard rather than a theoretical one. Verified
//! against hledger 1.52:
//!
//! ```text
//! $ printf '2026-02-01 A\n    a  1234.5 EUR\n    b  -1234.5 EUR\n' | hledger -f- print
//!     a           1234.5 EUR
//!
//! $ printf 'commodity 1.000,00 EUR\n\n2026-02-01 A\n …' | hledger -f- print --round=soft
//!     a        12.345,00 EUR
//! ```
//!
//! Exit zero, ten thousand times the money. It cannot arise from a directive the
//! entries were already written under, but the entries are written by an
//! `import` that read a *fragment* — which in a split layout is exactly the file
//! that does not carry the declaration. So [`preserves_entries`] compares the
//! two texts by **value** before either is written anywhere, and the caller
//! keeps hledger's own unstyled output when they disagree.

use crate::model::{AmountStyle, Commodity, DigitGroups, Journal, Posting, Transaction};
use crate::parse::{is_commodity_char, parse_journal};

/// The widest digit group this module will spell, in digits.
///
/// A group size is a `u8` off the parser, so a corrupt or exotic declaration
/// could ask for 255 zeros. Nothing real groups past a handful; the cap keeps a
/// specimen a specimen.
const MAX_GROUP_DIGITS: u8 = 12;

/// The most decimal places this module will spell.
///
/// Matches the clamp `edit.rs` and `assertions.rs` already hold to: 255 is
/// hledger's own maximum displayed precision, so this cannot refuse a style
/// hledger could have declared.
const MAX_PLACES: u32 = 255;

/// The journal's declared commodity styles, spelled back as `commodity`
/// directive lines — newline-terminated, empty when it declares none.
///
/// [`Journal::commodity_styles`] holds only what a `commodity` or `D` directive
/// **declared**; a style inferred from an amount's first occurrence never
/// reaches it. That is exactly the distinction wanted here: an inferred style is
/// a description of the text, and re-imposing it on the text would be circular.
///
/// The empty string is a real answer and the caller's cue to leave hledger's
/// output alone: a journal that declares no style has not asked for one, and
/// `print --round` over such a journal invents a canonical precision from
/// whatever happens to be in the batch (three rows ending `.678` would pad the
/// other two to three places). Doing nothing is what keeps a journal with no
/// `commodity` directive importing exactly as it did before.
#[must_use]
pub fn commodity_directives(journal: &Journal) -> String {
    journal
        .commodity_styles
        .iter()
        .filter_map(|(commodity, style)| directive_line(commodity, style))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// One `commodity` directive line, or `None` when this module will not commit
/// to spelling that style.
///
/// The refusal is **measured, not enumerated**: the line is parsed back with the
/// journal parser and kept only if it yields the very `(commodity, style)` pair
/// it was built from. So the question "can we spell this?" is answered by the
/// only thing that could answer it correctly — the parser that will read it —
/// and a style shape nobody here anticipated fails safe instead of producing a
/// directive that says something else.
fn directive_line(commodity: &Commodity, style: &AmountStyle) -> Option<String> {
    let line = format!("commodity {}", specimen(commodity, style)?);
    let parsed = parse_journal(&line, "commodity directives").ok()?;
    match parsed.commodity_styles.as_slice() {
        [(round_tripped, round_tripped_style)]
            if round_tripped == commodity && round_tripped_style == style =>
        {
            Some(line)
        }
        _ => None,
    }
}

/// The specimen amount that declares `style`: `$1,000.00`, `1.000,00 EUR`,
/// `1000 JPY`.
///
/// `None` for anything this module declines to write bare — a symbol needing
/// quotes, a group mark that is not `,` or `.`, an absurd width. Every one of
/// those would also be caught by the round-trip in [`directive_line`]; they are
/// refused here as well so the refusal has a reason a reader can see.
fn specimen(commodity: &Commodity, style: &AmountStyle) -> Option<String> {
    let symbol = &commodity.0;
    if symbol.is_empty() || !symbol.chars().all(is_commodity_char) {
        return None;
    }
    if style.precision > MAX_PLACES {
        return None;
    }
    let integer = integer_specimen(style.digit_groups.as_ref())?;
    let number = match style.decimal_mark {
        // An integer-only commodity (`commodity 1000 JPY`) declares no mark, and
        // writing one would declare a precision the journal never asked for.
        None if style.precision == 0 => integer,
        None => return None,
        Some(mark) => format!("{integer}{mark}{}", "0".repeat(style.precision as usize)),
    };
    let separator = if style.spaced { " " } else { "" };
    Some(match style.side {
        crate::model::CommoditySide::Left => format!("{symbol}{separator}{number}"),
        crate::model::CommoditySide::Right => format!("{number}{separator}{symbol}"),
    })
}

/// The integer part of a specimen: `1000` ungrouped, `1,000` for simple
/// thousands, `1,00,000` for the Indian grouping the parser also models.
///
/// [`DigitGroups::sizes`] runs **right to left** (`1,000.00` is `[3]`, `1,00,000`
/// is `[3, 2]`), and the leading partial group is dropped on the way in — so a
/// specimen is rebuilt as a leading `1` followed by each group from the left.
/// Re-analysing the result recovers the same sizes, which the round-trip in
/// [`directive_line`] is what actually proves.
fn integer_specimen(groups: Option<&DigitGroups>) -> Option<String> {
    let Some(groups) = groups else {
        return Some("1000".to_string());
    };
    let usable = !groups.sizes.is_empty()
        && groups
            .sizes
            .iter()
            .all(|size| *size > 0 && *size <= MAX_GROUP_DIGITS)
        && matches!(groups.mark, ',' | '.');
    if !usable {
        return None;
    }
    Some(
        groups
            .sizes
            .iter()
            .rev()
            .fold(String::from("1"), |built, size| {
                format!("{built}{}{}", groups.mark, "0".repeat(usize::from(*size)))
            }),
    )
}

/// Do `restyled` and `original` describe **exactly the same transactions**?
///
/// The guard on the whole re-styling. Compared by value, so `$165.2` and
/// `$165.20` are the same amount and `1234.5 EUR` and `12.345,00 EUR` are not —
/// see the module docs for how the second one arises and why it exits zero.
///
/// Both texts are parsed **on their own**, with no directive in scope, which is
/// the point: prepending the same directives to both would reproduce a misparse
/// in both and make the comparison agree with itself. Read bare, the re-styled
/// text says what it will say when it is read back out of the journal.
///
/// Anything either parser refuses answers `false`. A re-styling that cannot be
/// re-read is not one to write.
#[must_use]
pub fn preserves_entries(original: &str, restyled: &str) -> bool {
    let (Ok(before), Ok(after)) = (
        parse_journal(original, "proposed"),
        parse_journal(restyled, "restyled"),
    ) else {
        return false;
    };
    before.transactions.len() == after.transactions.len()
        && before
            .transactions
            .iter()
            .zip(&after.transactions)
            .all(|(before, after)| same_transaction(before, after))
}

/// Everything about a transaction that is not its spelling.
///
/// Deliberately **not** `Transaction: PartialEq`: the source spans, the file
/// name and the amount *styles* all differ between the two texts by
/// construction, and every one of them differing is the whole point. What may
/// not differ is what the transaction means.
fn same_transaction(before: &Transaction, after: &Transaction) -> bool {
    before.date == after.date
        && before.date2 == after.date2
        && before.status == after.status
        && before.code == after.code
        && before.description == after.description
        && before.tags == after.tags
        && before.postings.len() == after.postings.len()
        && before
            .postings
            .iter()
            .zip(&after.postings)
            .all(|(before, after)| same_posting(before, after))
}

/// The same, one posting down: account, amounts, and any balance assertion.
///
/// The assertion is included because a rules file's `balance` field writes one
/// into the proposed entries, and an assertion is a claim about money in exactly
/// the way an amount is — a re-styling that moved `= $880.00` would be a
/// re-styling that changed what the journal asserts.
fn same_posting(before: &Posting, after: &Posting) -> bool {
    before.account == after.account
        && before.ptype == after.ptype
        && before.status == after.status
        && same_amounts(&before.amounts, &after.amounts)
        && match (&before.balance_assertion, &after.balance_assertion) {
            (None, None) => true,
            (Some(before), Some(after)) => {
                before.inclusive == after.inclusive
                    && before.total == after.total
                    && same_amount(&before.amount, &after.amount)
            }
            _ => false,
        }
}

/// Two mixed amounts, compared commodity by commodity in order.
fn same_amounts(before: &[crate::model::Amount], after: &[crate::model::Amount]) -> bool {
    before.len() == after.len()
        && before
            .iter()
            .zip(after)
            .all(|(before, after)| same_amount(before, after))
}

/// One amount: its commodity, its quantity **by value**, and its cost.
///
/// [`Dec`](crate::Dec) compares by value rather than by scale, which is exactly
/// the question here — `165.2` and `165.20` are one number written two ways, and
/// re-writing it the second way is the entire feature.
fn same_amount(before: &crate::model::Amount, after: &crate::model::Amount) -> bool {
    before.commodity == after.commodity
        && before.quantity == after.quantity
        && match (&before.cost, &after.cost) {
            (None, None) => true,
            (Some(before), Some(after)) => {
                before.kind == after.kind && same_amount(&before.amount, &after.amount)
            }
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CommoditySide;

    /// Build a journal from text, so the styles under test are the ones the
    /// parser actually produces rather than ones hand-assembled here.
    fn journal(text: &str) -> Journal {
        parse_journal(text, "test.journal").expect("the fixture parses")
    }

    #[test]
    fn a_declared_dollar_style_is_spelled_back_verbatim() {
        let journal = journal("commodity $1,000.00  ; comma thousands, 2 decimals\n");
        assert_eq!(commodity_directives(&journal), "commodity $1,000.00\n");
    }

    /// The user's own file declares two, one of each side. Both come back, in
    /// declaration order.
    #[test]
    fn every_declared_style_is_spelled_in_declaration_order() {
        let journal = journal("commodity $1,000.00\ncommodity 1,000.00 AAPL\n");
        assert_eq!(
            commodity_directives(&journal),
            "commodity $1,000.00\ncommodity 1,000.00 AAPL\n"
        );
    }

    /// A European declaration must come back European — the decimal mark and
    /// the group mark swap, and getting them the wrong way round is a 1000x
    /// misreading rather than a cosmetic slip.
    #[test]
    fn a_european_style_keeps_its_marks() {
        let journal = journal("commodity 1.000,00 EUR\n");
        assert_eq!(commodity_directives(&journal), "commodity 1.000,00 EUR\n");
    }

    #[test]
    fn an_ungrouped_style_declares_no_group() {
        let journal = journal("commodity USD 1000.00\n");
        assert_eq!(commodity_directives(&journal), "commodity USD 1000.00\n");
    }

    #[test]
    fn an_integer_only_style_declares_no_decimal_mark() {
        let journal = journal("commodity 1000 JPY\n");
        assert_eq!(commodity_directives(&journal), "commodity 1000 JPY\n");
    }

    /// A symbol-only `commodity $` declares no style, so there is nothing to
    /// spell — and nothing to restyle by.
    #[test]
    fn a_symbol_only_declaration_produces_nothing() {
        let journal = journal("commodity $\n");
        assert_eq!(commodity_directives(&journal), "");
    }

    /// **The case that keeps a plain journal importing exactly as it did.** No
    /// declaration means no directives, which means the caller does not restyle
    /// at all.
    #[test]
    fn a_journal_that_declares_nothing_produces_nothing() {
        let journal = journal("2026-01-01 opening\n    a  $1000.00\n    b\n");
        assert_eq!(commodity_directives(&journal), "");
    }

    /// A `D` directive declares a style too, and hledger reads a `commodity`
    /// line as declaring the same thing.
    #[test]
    fn a_default_commodity_directive_is_a_declared_style() {
        let journal = journal("D $1,000.00\n");
        assert_eq!(commodity_directives(&journal), "commodity $1,000.00\n");
    }

    /// Every emitted line must parse back to the style it came from. Asserted
    /// over the whole corpus rather than per-case, because the property is what
    /// makes the function safe.
    #[test]
    fn every_emitted_directive_round_trips() {
        for declaration in [
            "commodity $1,000.00",
            "commodity 1.000,00 EUR",
            "commodity 1,000.00 AAPL",
            "commodity USD 1000.00",
            "commodity 1000 JPY",
            "commodity $1,00,000.00",
        ] {
            let source = journal(&format!("{declaration}\n"));
            let spelled = commodity_directives(&source);
            let reparsed = journal(&spelled);
            assert_eq!(
                reparsed.commodity_styles, source.commodity_styles,
                "{declaration} must survive being spelled back out, got {spelled:?}"
            );
        }
    }

    /// A group mark that is not `,` or `.` — hledger allows whitespace — is not
    /// spelled at all rather than spelled as something else.
    #[test]
    fn an_unspellable_group_mark_is_dropped() {
        assert_eq!(
            integer_specimen(Some(&DigitGroups {
                mark: ' ',
                sizes: vec![3],
            })),
            None
        );
    }

    /// A zero-wide group would produce `1,` — refused rather than emitted.
    #[test]
    fn a_zero_width_group_is_dropped() {
        assert_eq!(
            integer_specimen(Some(&DigitGroups {
                mark: ',',
                sizes: vec![0],
            })),
            None
        );
    }

    /// A symbol that would need quoting is not spelled bare.
    #[test]
    fn a_symbol_needing_quotes_is_dropped() {
        let style = AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: 2,
        };
        assert_eq!(specimen(&Commodity("MY FUND".to_string()), &style), None);
        assert_eq!(specimen(&Commodity(String::new()), &style), None);
    }

    // -----------------------------------------------------------------------
    // preserves_entries
    // -----------------------------------------------------------------------

    const PROPOSED: &str = "2026-02-01 GROCERY STORE\n\
                            \x20   assets:bank:checking   $-405\n\
                            \x20   expenses:unknown        $405\n";

    #[test]
    fn padding_decimal_zeros_preserves_the_entries() {
        let styled = "2026-02-01 GROCERY STORE\n\
                      \x20   assets:bank:checking   $-405.00\n\
                      \x20   expenses:unknown        $405.00\n";
        assert!(preserves_entries(PROPOSED, styled));
    }

    /// **The 10x hazard.** A European `commodity` directive re-reads `1234.5` as
    /// twelve thousand, hledger says so with exit zero, and the only thing that
    /// catches it is comparing the values.
    #[test]
    fn a_reparsed_decimal_mark_does_not_preserve_the_entries() {
        let original = "2026-02-01 A\n    a   1234.5 EUR\n    b  -1234.5 EUR\n";
        let mangled = "2026-02-01 A\n    a   12.345,00 EUR\n    b  -12.345,00 EUR\n";
        assert!(!preserves_entries(original, mangled));
    }

    #[test]
    fn rounding_away_a_digit_does_not_preserve_the_entries() {
        let original = "2026-02-01 A\n    a   $12345.678\n    b  $-12345.678\n";
        let rounded = "2026-02-01 A\n    a   $12,345.68\n    b  $-12,345.68\n";
        assert!(!preserves_entries(original, rounded));
    }

    #[test]
    fn a_dropped_transaction_does_not_preserve_the_entries() {
        assert!(!preserves_entries(PROPOSED, ""));
    }

    #[test]
    fn a_renamed_account_does_not_preserve_the_entries() {
        let renamed = "2026-02-01 GROCERY STORE\n\
                       \x20   assets:bank:savings    $-405.00\n\
                       \x20   expenses:unknown        $405.00\n";
        assert!(!preserves_entries(PROPOSED, renamed));
    }

    #[test]
    fn a_changed_description_does_not_preserve_the_entries() {
        let renamed = "2026-02-01 GROCERY SHOP\n\
                       \x20   assets:bank:checking   $-405.00\n\
                       \x20   expenses:unknown        $405.00\n";
        assert!(!preserves_entries(PROPOSED, renamed));
    }

    /// Status, code, comments and tags survive `print`; a re-styling that lost
    /// the status would be one to refuse.
    #[test]
    fn a_dropped_status_does_not_preserve_the_entries() {
        let original = "2026-02-01 * (99) A  ; note, k:v\n    a  $1\n    b  $-1\n";
        let stripped = "2026-02-01 A\n    a  $1.00\n    b  $-1.00\n";
        assert!(!preserves_entries(original, stripped));
        let kept = "2026-02-01 * (99) A  ; note, k:v\n    a  $1.00\n    b  $-1.00\n";
        assert!(preserves_entries(original, kept));
    }

    /// A rules file's `balance` field writes an assertion into the proposal, and
    /// an assertion is money.
    #[test]
    fn a_moved_balance_assertion_does_not_preserve_the_entries() {
        let original = "2026-02-01 A\n    a  $1 = $880.00\n    b  $-1\n";
        let moved = "2026-02-01 A\n    a  $1.00 = $881.00\n    b  $-1.00\n";
        assert!(!preserves_entries(original, moved));
        let kept = "2026-02-01 A\n    a  $1.00 = $880.00\n    b  $-1.00\n";
        assert!(preserves_entries(original, kept));
    }

    #[test]
    fn text_that_does_not_parse_never_preserves_anything() {
        assert!(!preserves_entries(
            PROPOSED,
            "2026-02-01\n  not a journal ((("
        ));
        assert!(!preserves_entries(
            "2026-02-01\n  not a journal (((",
            PROPOSED
        ));
    }

    #[test]
    fn two_empty_texts_preserve_each_other() {
        assert!(preserves_entries("", ""));
    }
}
