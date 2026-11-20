//! Criterion coverage of the bill-table engine's interactive paths
//! (ARCHITECTURE §5.6; PLAN Phase 1a's budget is under 1 ms each).
//!
//! | Bench | What it is | Budget |
//! |---|---|---|
//! | `table/recompute_100` | classify, compute and rewrite a parsed 100-row table | < 1 ms |
//! | `table/recalc_markdown_100` | the same from the document text: find, parse, recompute, verified edits, splice | < 1 ms |
//! | `table/rewrite_one_cell_100` | set one cell and produce its verified edits | < 1 ms |
//! | `table/keystroke_100` | one cell typed, then the whole document recomputed (the editor's keystroke) | < 1 ms |
//!
//! ```text
//! nix develop -c cargo bench -p invoicer-core --bench table
//! ```

use criterion::{Criterion, criterion_group, criterion_main};
use invoicer_core::table::gfm::apply_edits;
use invoicer_core::table::money::MoneyStyle;
use invoicer_core::table::words::Words;
use invoicer_core::table::{BillTable, recalc_markdown};
use std::fmt::Write as _;
use std::hint::black_box;

const ROWS: usize = 100;

/// A 100-row invoice: items with quantities and rates, a discount under
/// every fourth item, a subtotal every ten rows, then tax, a deposit and
/// the total. Amounts start blank, so the first recompute writes them all.
fn document() -> String {
    let mut doc = String::from(
        "Hi, the work for November.\n\n\
         | Description | Qty | Rate | Amount | *Account* |\n\
         |:--|--:|--:|--:|:--|\n",
    );
    let body = ROWS - 3;
    for i in 0..body {
        let row = if i % 10 == 9 {
            "| | | **Subtotal** | | |".to_string()
        } else if i % 4 == 3 {
            "| Loyalty discount | | -7.5% | | |".to_string()
        } else {
            format!(
                "| Service line {i} | {}.{} | ${},{:03}.{:02} | | revenue:line-{} |",
                1 + i % 40,
                i % 10,
                1 + i % 9,
                (i * 37) % 1000,
                (i * 13) % 100,
                i % 5
            )
        };
        let _ = writeln!(doc, "{row}");
    }
    doc.push_str("| Sales tax | | 8.25% | | liabilities:sales-tax |\n");
    doc.push_str(
        "| Less deposit (Invoice 104) | | | -$5,000.00 | liabilities:customer-deposits |\n",
    );
    doc.push_str("| | | **Total due** | | |\n\nThanks!\n");
    doc
}

fn money() -> MoneyStyle {
    let journal = ledgeline_core::parse_journal("commodity $1,000.00\n", "books.journal")
        .unwrap_or_else(|e| panic!("journal: {e}"));
    MoneyStyle::of_journal(&journal, "$")
}

fn benches(c: &mut Criterion) {
    let words = Words::default();
    let money = money();
    // Recompute once so the steady state (amounts present, one changes) is
    // what the editor sees on a keystroke.
    let blank = document();
    let doc = recalc_markdown(&blank, &words, &money)
        .ok()
        .flatten()
        .unwrap_or(blank.clone());
    let table = BillTable::find(&doc, &words)
        .ok()
        .flatten()
        .unwrap_or_else(|| panic!("the bench document has a bill table"));
    assert_eq!(table.table().grid.rows.len(), ROWS);

    let mut group = c.benchmark_group("table");
    group.bench_function("recompute_100", |b| {
        b.iter(|| black_box(&table).recompute(&words, &money));
    });
    group.bench_function("recalc_markdown_100", |b| {
        b.iter(|| recalc_markdown(black_box(&blank), &words, &money));
    });
    group.bench_function("rewrite_one_cell_100", |b| {
        b.iter(|| {
            black_box(&table)
                .with_cell(40, 1, "17")
                .and_then(|edited| edited.edits(&doc))
        });
    });
    group.bench_function("keystroke_100", |b| {
        b.iter(|| {
            let edits = table
                .with_cell(40, 1, "17")
                .and_then(|edited| edited.edits(black_box(&doc)))
                .unwrap_or_default();
            let typed = apply_edits(&doc, &edits);
            recalc_markdown(&typed, &words, &money)
        });
    });
    group.finish();
}

criterion_group!(table, benches);
criterion_main!(table);
