//! Shared test helpers.
//!
//! The spec examples are copies of `invoicer/ARCHITECTURE.md`'s, kept under
//! `tests/fixtures/` because the flake's source filter keeps `fixtures/` but
//! not markdown docs (so `nix build .#tests` can compile them).
//! `spec_examples::fixtures_match_the_architecture_doc` keeps the copies in
//! step with the doc.

#![allow(dead_code)]

use invoicer_core::table::BillTable;
use invoicer_core::table::money::MoneyStyle;
use invoicer_core::table::words::Words;
use ledgeline_core::Dec;

/// The `n`th (0-based) ```` ```markdown ```` block of `doc`, without its
/// fences, with a trailing newline.
#[must_use]
pub fn markdown_block(doc: &str, n: usize) -> Option<String> {
    let start = doc
        .match_indices("```markdown\n")
        .nth(n)
        .map(|(at, fence)| at + fence.len())?;
    let len = doc[start..].find("\n```")?;
    Some(format!("{}\n", &doc[start..start + len]))
}

/// §3.2's whole invoice.
#[must_use]
pub fn invoice_3_2() -> String {
    include_str!("../fixtures/invoice-3-2.md").to_string()
}

/// §3.3's worked example table.
#[must_use]
pub fn worked_example_3_3() -> String {
    include_str!("../fixtures/worked-example-3-3.md").to_string()
}

/// `$` as a journal declaring `commodity $1,000.00` writes it, read through
/// ledgeline-core's `JournalStyles`.
#[must_use]
pub fn usd() -> MoneyStyle {
    let journal = ledgeline_core::parse_journal("commodity $1,000.00\n", "books.journal")
        .expect("journal parses");
    MoneyStyle::of_journal(&journal, "$")
}

/// `$` in cents.
#[must_use]
pub fn cents(c: i128) -> Dec {
    Dec::new(c, 2)
}

/// The bill table of `markdown` under the default words.
#[must_use]
pub fn bill(markdown: &str) -> BillTable {
    BillTable::find(markdown, &Words::default())
        .expect("reads")
        .expect("has a bill table")
}

/// The table's text in `markdown` (its source range).
#[must_use]
pub fn table_text<'a>(markdown: &'a str, table: &BillTable) -> &'a str {
    let range = table.table().source.as_ref().expect("parsed").range.clone();
    &markdown[range]
}

/// Every body row's amount.
#[must_use]
pub fn amounts(table: &BillTable, money: &MoneyStyle) -> Vec<Option<Dec>> {
    table
        .evaluate(&Words::default(), money)
        .expect("evaluates")
        .rows
        .iter()
        .map(|row| row.amount)
        .collect()
}
