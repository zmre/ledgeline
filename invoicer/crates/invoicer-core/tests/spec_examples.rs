//! ARCHITECTURE §3.2 and §3.3, exactly: amounts, booking, and byte-exact
//! write-back.

mod common;

use common::{amounts, bill, cents, invoice_3_2, table_text, usd, worked_example_3_3};
use invoicer_core::booking::{AccountDefaults, split};
use invoicer_core::table::columns::Role;
use invoicer_core::table::words::Words;
use invoicer_core::table::{RowKind, recalc_markdown};
use ledgeline_core::Dec;
use ledgeline_core::accrual::schedule::{Proration, Term};

fn d(whole: i128) -> Dec {
    Dec::new(whole, 0)
}

#[test]
fn fixtures_match_the_architecture_doc() {
    // Read at run time: the nix test build filters docs out of the source,
    // and there the copies stand alone.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ARCHITECTURE.md");
    let Ok(doc) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: {} is not in this source tree", path.display());
        return;
    };
    assert_eq!(common::markdown_block(&doc, 0), Some(invoice_3_2()));
    assert_eq!(common::markdown_block(&doc, 1), Some(worked_example_3_3()));
}

#[test]
fn section_3_2_amounts_are_exact() {
    let doc = invoice_3_2();
    let table = bill(&doc);
    assert_eq!(
        amounts(&table, &usd()),
        [
            Some(d(8_000)),
            Some(d(-800)),
            Some(d(10_000)),
            Some(d(-500)),
            Some(d(16_700)),
            Some(d(-8_350)),
            Some(d(8_350)),
        ]
    );
}

#[test]
fn section_3_2_is_already_right_and_writes_back_byte_exact() {
    let doc = invoice_3_2();
    let table = bill(&doc);
    // `$8,000` matches $8,000.00 by value, so nothing is rewritten.
    assert_eq!(table.check(&Words::default(), &usd()).expect("checks"), []);
    assert_eq!(recalc_markdown(&doc, &Words::default(), &usd()), Ok(None));
    assert_eq!(table.render(), table_text(&doc, &table));
    assert_eq!(table.edits(&doc).expect("writes"), []);
}

#[test]
fn section_3_2_books_as_section_4_2_says() {
    let doc = invoice_3_2();
    let evaluation = bill(&doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let booking = split(&evaluation, &AccountDefaults::default(), &usd()).expect("splits");
    let lines: Vec<(&str, Dec)> = booking
        .lines
        .iter()
        .map(|line| (line.books.account().as_str(), line.amount))
        .collect();
    assert_eq!(
        lines,
        [
            ("revenue:consulting", d(7_200)),
            ("revenue:consulting:delivery", d(9_500)),
            ("liabilities:customer-deposits", d(-8_350)),
        ]
    );
    assert_eq!(booking.total, d(8_350));
}

#[test]
fn section_3_3_amounts_and_kinds_are_exact() {
    let doc = worked_example_3_3();
    let table = bill(&doc);
    let evaluation = table
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let kinds: Vec<RowKind> = evaluation.rows.iter().map(|row| row.kind).collect();
    assert!(matches!(
        kinds.as_slice(),
        [
            RowKind::Item,
            RowKind::Item,
            RowKind::Adjustment(_),
            RowKind::Item,
            RowKind::Spacer,
            RowKind::Total,
            RowKind::Adjustment(_),
            RowKind::Total,
            RowKind::Adjustment(_),
            RowKind::FixedAdjustment,
            RowKind::Total,
        ]
    ));
    assert_eq!(
        amounts(&table, &usd()),
        [
            Some(cents(1_200_000)),
            Some(cents(250_000)),
            Some(cents(-25_000)),
            Some(cents(600_000)),
            None,
            Some(cents(2_025_000)),
            Some(cents(-40_500)),
            Some(cents(1_984_500)),
            Some(cents(163_721)),
            Some(cents(-500_000)),
            Some(cents(1_648_221)),
        ]
    );
    // Loyalty discount → the architect hours; early-payment → the first
    // subtotal; sales tax → the second subtotal.
    let targets: Vec<Option<usize>> = evaluation.rows.iter().map(|r| r.applies_to).collect();
    assert_eq!(targets[2], Some(1));
    assert_eq!(targets[6], Some(5));
    assert_eq!(targets[8], Some(7));
    assert_eq!(evaluation.total, cents(1_648_221));
}

#[test]
fn section_3_3_writes_back_byte_exact_even_from_blank_amounts() {
    let doc = worked_example_3_3();
    let table = bill(&doc);
    assert_eq!(table.render(), table_text(&doc, &table));
    assert_eq!(table.check(&Words::default(), &usd()).expect("checks"), []);

    // Blank every computed amount, then recompute: the invoicer writes the
    // doc's text exactly.
    let amount_col = table.columns().find(Role::Amount).expect("has one");
    let evaluation = table
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let blanks: Vec<(usize, usize, &str)> = evaluation
        .rows
        .iter()
        .filter(|row| row.computed)
        .map(|row| (row.at.index, amount_col, ""))
        .collect();
    assert_eq!(blanks.len(), 9);
    let blanked = table.with_cells(&blanks).expect("in range");
    let blank_doc =
        invoicer_core::table::gfm::apply_edits(&doc, &blanked.edits(&doc).expect("writes"));
    assert_ne!(blank_doc, doc);
    assert_eq!(
        recalc_markdown(&blank_doc, &Words::default(), &usd()),
        Ok(Some(doc))
    );
}

#[test]
fn section_3_3_books_the_exact_postings() {
    let doc = worked_example_3_3();
    let evaluation = bill(&doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let booking = split(&evaluation, &AccountDefaults::default(), &usd()).expect("splits");
    let lines: Vec<(&str, Dec)> = booking
        .lines
        .iter()
        .map(|line| (line.books.account().as_str(), line.amount))
        .collect();
    assert_eq!(
        lines,
        [
            ("revenue:projects", cents(1_176_000)),
            ("revenue:consulting", cents(220_500)),
            ("liabilities:deferred-revenue", cents(588_000)),
            ("liabilities:sales-tax", cents(163_721)),
            ("liabilities:customer-deposits", cents(-500_000)),
        ]
    );
    assert_eq!(booking.total, cents(1_648_221));
    let sum = booking
        .lines
        .iter()
        .try_fold(Dec::zero(), |acc, line| acc.add(line.amount))
        .expect("sums");
    assert_eq!(sum, booking.total);

    let deferral = booking.lines[2].books.deferral().expect("deferred");
    assert_eq!(deferral.target_account, "revenue:support");
    assert_eq!(deferral.term, Term::Months(12));
    assert_eq!(deferral.service_start.as_deref(), Some("2026-12-01"));
    assert_eq!(deferral.proration, Proration::Anniversary);
    // Rows feeding each line: the 2% (row 6) spreads over the first three.
    assert_eq!(booking.lines[0].rows, [0, 6]);
    assert_eq!(booking.lines[1].rows, [1, 2, 6]);
    assert_eq!(booking.lines[2].rows, [3, 6]);
}
