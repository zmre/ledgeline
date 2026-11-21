//! Properties of the table engine (ARCHITECTURE §5.6, PLAN Phase 1a).

mod common;

use common::usd;
use invoicer_core::booking::{AccountDefaults, BookingError, split};
use invoicer_core::table::books::AccountName;
use invoicer_core::table::gfm::{Alignment, Grid, Layout, Table, apply_edits, tables};
use invoicer_core::table::words::Words;
use invoicer_core::table::{BillTable, RowKind, recalc_markdown};
use ledgeline_core::Dec;
use proptest::prelude::*;

// ---------------------------------------------------------------- grids

fn cell_text() -> impl Strategy<Value = String> {
    let tokens = prop::sample::select(vec![
        "a", "Z", "9", " ", "$", ".", ",", "*", "_", "→", "漢", "-", "%", "\\|", "&nbsp;", "é",
    ]);
    prop::collection::vec(tokens, 0..8).prop_map(|parts| parts.concat().trim().to_string())
}

fn alignment() -> impl Strategy<Value = Alignment> {
    prop::sample::select(vec![
        Alignment::None,
        Alignment::Left,
        Alignment::Right,
        Alignment::Center,
    ])
}

fn grid() -> impl Strategy<Value = Grid> {
    (1usize..6).prop_flat_map(|cols| {
        (
            prop::collection::vec(cell_text(), cols),
            prop::collection::vec(alignment(), cols),
            prop::collection::vec(prop::collection::vec(cell_text(), cols), 0..8),
        )
            .prop_map(|(header, alignments, rows)| Grid {
                header,
                alignments,
                rows,
            })
    })
}

proptest! {
    #[test]
    fn parse_of_write_is_the_table(grid in grid(), indent in 0usize..4, crlf: bool) {
        let table = Table {
            grid,
            layout: Layout { indent: " ".repeat(indent), crlf, ..Layout::fresh() },
            source: None,
        };
        let text = table.render();
        let parsed = tables(&text);
        prop_assert_eq!(parsed.len(), 1);
        prop_assert_eq!(&parsed[0].grid, &table.grid);
        // Writing what was read changes nothing.
        prop_assert_eq!(parsed[0].render(), text.clone());
        prop_assert_eq!(parsed[0].edits(&text).expect("writes"), vec![]);
    }
}

// ----------------------------------------------------------- bill tables

/// One generated body row.
#[derive(Debug, Clone)]
enum Spec {
    Item {
        qty: Dec,
        rate: i128,
        books: Option<usize>,
    },
    Written {
        cents: i128,
        books: Option<usize>,
    },
    Percent {
        hundredths: i128,
        books: Option<usize>,
    },
    Fixed {
        cents: i128,
        books: Option<usize>,
    },
    Spacer,
    Total(usize),
}

const BOOKS: [&str; 4] = [
    "revenue:a",
    "revenue:b",
    "liabilities:sales-tax",
    "liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01",
];
const TOTALS: [&str; 3] = ["Subtotal", "Total", "Total due"];

fn books() -> impl Strategy<Value = Option<usize>> {
    prop::option::of(0..BOOKS.len())
}

fn item() -> impl Strategy<Value = Spec> {
    ((1i128..2_000, 0u32..3), 1i128..1_000_000, books()).prop_map(|((q, places), rate, books)| {
        Spec::Item {
            qty: Dec::new(q, places),
            rate,
            books,
        }
    })
}

fn spec() -> impl Strategy<Value = Spec> {
    prop_oneof![
        3 => item(),
        1 => (1i128..1_000_000, books()).prop_map(|(cents, books)| Spec::Written { cents, books }),
        2 => (-5_000i128..5_000, books()).prop_map(|(hundredths, books)| Spec::Percent { hundredths, books }),
        1 => ((1i128..500_000), any::<bool>(), books())
            .prop_map(|(c, neg, books)| Spec::Fixed { cents: if neg { -c } else { c }, books }),
        1 => Just(Spec::Spacer),
        1 => (0..TOTALS.len()).prop_map(Spec::Total),
    ]
}

/// Column order: a permutation of the four core roles, then the books
/// column.
fn order() -> impl Strategy<Value = Vec<usize>> {
    Just(vec![0usize, 1, 2, 3]).prop_shuffle()
}

/// `[description, qty, rate, amount, books]` for one row. `noise` puts a
/// wrong value in the amount cell, which recompute must fix.
fn cells(spec: &Spec, i: usize, noise: bool) -> [String; 5] {
    let money = |cents: i128| usd().format(Dec::new(cents, 2));
    let book = |b: &Option<usize>| b.map_or(String::new(), |b| BOOKS[b].to_string());
    let wrong = if noise {
        "$1.23".to_string()
    } else {
        String::new()
    };
    match spec {
        Spec::Item { qty, rate, books } => [
            format!("Item {i}"),
            render_dec(*qty),
            money(*rate),
            wrong,
            book(books),
        ],
        Spec::Written { cents, books } => [
            format!("Fee {i}"),
            String::new(),
            String::new(),
            money(*cents),
            book(books),
        ],
        Spec::Percent { hundredths, books } => [
            format!("Adj {i}"),
            String::new(),
            format!("{}%", render_dec(Dec::new(*hundredths, 2))),
            wrong,
            book(books),
        ],
        Spec::Fixed { cents, books } => {
            let text = money(*cents);
            let signed = if *cents > 0 { format!("+{text}") } else { text };
            [
                format!("Fixed {i}"),
                String::new(),
                String::new(),
                signed,
                book(books),
            ]
        }
        Spec::Spacer => Default::default(),
        Spec::Total(t) => [
            String::new(),
            String::new(),
            format!("**{}**", TOTALS[*t]),
            wrong,
            String::new(),
        ],
    }
}

fn render_dec(d: Dec) -> String {
    let negative = d.mantissa < 0;
    let digits = format!(
        "{:0>width$}",
        d.mantissa.unsigned_abs(),
        width = d.places as usize + 1
    );
    let (int, frac) = digits.split_at(digits.len() - d.places as usize);
    format!(
        "{}{int}{}{frac}",
        if negative { "-" } else { "" },
        if frac.is_empty() { "" } else { "." }
    )
}

fn document(specs: &[Spec], order: &[usize], noise: &[bool]) -> String {
    let names = ["Description", "Qty", "Rate", "Amount", "*Account*"];
    let line = |row: [String; 5]| {
        let picked: Vec<&str> = order
            .iter()
            .chain(std::iter::once(&4))
            .map(|c| row[*c].as_str())
            .collect();
        format!("| {} |", picked.join(" | "))
    };
    let header = line(names.map(str::to_string));
    let body = specs
        .iter()
        .enumerate()
        .map(|(i, spec)| line(cells(spec, i, noise.get(i).copied().unwrap_or(false))));
    std::iter::once(header)
        .chain(std::iter::once("|---|---|---|---|---|".to_string()))
        .chain(body)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// A table: an item first (so every adjustment has a target), then
/// anything, then a final total.
fn bill_doc() -> impl Strategy<Value = String> {
    (
        item(),
        prop::collection::vec(spec(), 0..25),
        order(),
        prop::collection::vec(any::<bool>(), 30),
    )
        .prop_map(|(first, rest, order, noise)| {
            let specs: Vec<Spec> = std::iter::once(first)
                .chain(rest)
                .chain(std::iter::once(Spec::Total(2)))
                .collect();
            document(&specs, &order, &noise)
        })
}

fn find(doc: &str) -> BillTable {
    BillTable::find(doc, &Words::default())
        .expect("reads")
        .expect("found")
}

fn defaults() -> AccountDefaults {
    AccountDefaults {
        settings: Some(AccountName::new("revenue:default").expect("valid")),
        ..AccountDefaults::default()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn recompute_is_idempotent(doc in bill_doc()) {
        let once = recalc_markdown(&doc, &Words::default(), &usd())
            .expect("recomputes")
            .unwrap_or_else(|| doc.clone());
        prop_assert_eq!(recalc_markdown(&once, &Words::default(), &usd()), Ok(None));
        prop_assert_eq!(find(&once).check(&Words::default(), &usd()).expect("checks"), vec![]);
    }

    #[test]
    fn written_totals_equal_the_rows_above(doc in bill_doc()) {
        let once = recalc_markdown(&doc, &Words::default(), &usd())
            .expect("recomputes")
            .unwrap_or_else(|| doc.clone());
        let table = find(&once);
        let money = usd();
        let rows = table.evaluate(&Words::default(), &money).expect("evaluates").rows;
        let written = |i: usize| {
            money
                .parse(table.cell(i, invoicer_core::table::columns::Role::Amount))
                .map(|w| w.value)
        };
        for (i, row) in rows.iter().enumerate().filter(|(_, r)| r.kind == RowKind::Total) {
            let above = rows[..i]
                .iter()
                .enumerate()
                .filter(|(_, r)| !matches!(r.kind, RowKind::Total | RowKind::Spacer))
                .try_fold(Dec::zero(), |acc, (j, _)| acc.add(written(j).expect("written")))
                .expect("sums");
            prop_assert_eq!(written(i).expect("total written"), above);
            prop_assert!(table.cell(i, invoicer_core::table::columns::Role::Amount).starts_with("**"));
            prop_assert_eq!(row.amount, Some(above));
        }
    }

    #[test]
    fn adjustments_apply_to_the_nearest_item_or_total(doc in bill_doc()) {
        let rows = find(&doc).evaluate(&Words::default(), &usd()).expect("evaluates").rows;
        for (i, row) in rows.iter().enumerate() {
            if let RowKind::Adjustment(_) = row.kind {
                let target = row.applies_to.expect("has a target");
                prop_assert!(target < i);
                prop_assert!(rows[target].kind.is_adjustable());
                prop_assert!(!rows[target].kind.is_adjustment());
                // Nothing adjustable between the target and the adjustment.
                prop_assert!(rows[target + 1..i].iter().all(|r| !r.kind.is_adjustable()));
            }
        }
    }

    #[test]
    fn the_booking_split_sums_to_the_total(doc in bill_doc()) {
        let evaluation = find(&doc).evaluate(&Words::default(), &usd()).expect("evaluates");
        match split(&evaluation, &defaults(), &usd()) {
            Ok(booking) => {
                let sum = booking
                    .lines
                    .iter()
                    .try_fold(Dec::zero(), |acc, line| acc.add(line.amount))
                    .expect("sums");
                prop_assert_eq!(sum, evaluation.total);
                prop_assert_eq!(booking.total, evaluation.total);
                prop_assert!(booking.lines.iter().all(|l| !l.amount.is_zero()));
            }
            // A blank-account adjustment under a zero total is the one
            // legitimate refusal.
            Err(BookingError::CannotSpread(_)) => {}
            Err(other) => prop_assert!(false, "{other}"),
        }
    }

    #[test]
    fn a_rewritten_cell_edits_only_the_table(doc in bill_doc(), row in 0usize..5, text in cell_text()) {
        let prose = format!("Before.\n\n{doc}\nAfter.\n");
        let table = find(&prose);
        let row = row.min(table.table().grid.rows.len() - 1);
        let edited = table.with_cell(row, 0, &text).expect("in range");
        let out = apply_edits(&prose, &edited.edits(&prose).expect("writes"));
        prop_assert!(out.starts_with("Before.\n\n") && out.ends_with("\nAfter.\n"));
        let reread = find(&out);
        prop_assert_eq!(&reread.table().grid, &edited.table().grid);
    }
}
