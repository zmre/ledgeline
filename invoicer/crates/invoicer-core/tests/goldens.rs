//! Golden tests: every row kind, column orders, tax placement, hand-written
//! oddities, word overrides, the resolver, and errors that name the row.

mod common;

use common::{amounts, bill, cents, table_text, usd, worked_example_3_3};
use invoicer_core::booking::{AccountDefaults, AccountSource, BookingError, resolve, split};
use invoicer_core::table::books::{AccountName, BooksEntry};
use invoicer_core::table::gfm::apply_edits;
use invoicer_core::table::money::MoneyStyle;
use invoicer_core::table::words::Words;
use invoicer_core::table::{BillTable, ItemCode, RowKind, TableError, recalc_markdown};
use ledgeline_core::Dec;

fn d(whole: i128) -> Dec {
    Dec::new(whole, 0)
}

fn account(name: &str) -> AccountName {
    AccountName::new(name).expect("valid")
}

fn recalc(doc: &str) -> String {
    recalc_markdown(doc, &Words::default(), &usd())
        .expect("recomputes")
        .unwrap_or_else(|| doc.to_string())
}

#[test]
fn every_row_kind_golden() {
    let doc = "\
| Description | Qty | Rate | Amount | *Account* |
|:--|--:|--:|--:|:--|
| Widget | 3 | $10.50 | | revenue:goods |
| Discount | | -10% | | |
| Shipping | | | $12 | revenue:shipping |
| Credit | | | -$5.00 | |
| &nbsp; | | | | |
| | | **Total** | | |
";
    let table = bill(doc);
    let evaluation = table
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let kinds: Vec<RowKind> = evaluation.rows.iter().map(|r| r.kind).collect();
    assert!(matches!(
        kinds.as_slice(),
        [
            RowKind::Item,
            RowKind::Adjustment(_),
            RowKind::Item, // unsigned, no qty/rate: an item, as written
            RowKind::FixedAdjustment,
            RowKind::Spacer,
            RowKind::Total,
        ]
    ));
    assert_eq!(recalc(doc), expected_every_kind());
}

/// The aligned rewrite of [`every_row_kind_golden`]'s table.
fn expected_every_kind() -> String {
    "\
| Description | Qty | Rate      | Amount     | *Account*        |
|:------------|----:|----------:|-----------:|:-----------------|
| Widget      | 3   | $10.50    | $31.50     | revenue:goods    |
| Discount    |     | -10%      | -$3.15     |                  |
| Shipping    |     |           | $12        | revenue:shipping |
| Credit      |     |           | -$5.00     |                  |
| &nbsp;      |     |           |            |                  |
|             |     | **Total** | **$35.35** |                  |
"
    .to_string()
}

#[test]
fn column_order_does_not_change_the_arithmetic() {
    let orders = [
        "| Description | Qty | Rate | Amount | *Account* |",
        "| Amount | Rate | Qty | Description | *Account* |",
        "| Qty | Description | Amount | Rate | *Account* |",
        "| Rate | Amount | Description | Qty |",
    ];
    let row = |cells: [&str; 5], header: &str| -> String {
        let names = ["Description", "Qty", "Rate", "Amount", "*Account*"];
        let line: Vec<&str> = header
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .map(|h| cells[names.iter().position(|n| *n == h).expect("known")])
            .collect();
        format!("| {} |", line.join(" | "))
    };
    let results: Vec<Vec<Option<Dec>>> = orders
        .iter()
        .map(|header| {
            let cols = header.matches('|').count() - 1;
            let doc = [
                header.to_string(),
                format!("|{}", "---|".repeat(cols)),
                row(["Hours", "10", "$250", "", "revenue:x"], header),
                row(["Discount", "", "-10%", "", ""], header),
                row(["", "", "**Subtotal**", "", ""], header),
                row(["Tax", "", "8.25%", "", "liabilities:tax"], header),
                row(["", "", "**Total due**", "", ""], header),
            ]
            .join("\n");
            amounts(&bill(&doc), &usd())
        })
        .collect();
    let expected = vec![
        Some(d(2_500)),
        Some(d(-250)),
        Some(d(2_250)),
        Some(cents(18_563)),
        Some(cents(243_563)),
    ];
    assert!(results.iter().all(|r| *r == expected), "{results:?}");
}

#[test]
fn tax_below_a_subtotal_taxes_everything_after_discounts() {
    let doc = "\
| Description | Qty | Rate | Amount |
|---|---|---|---|
| A | 1 | $100 | |
| B | 1 | $200 | |
| Discount | | -10% | |
| | | **Subtotal** | |
| Sales tax | | 10% | |
| | | **Total due** | |
";
    assert_eq!(
        amounts(&bill(doc), &usd()),
        [
            Some(d(100)),
            Some(d(200)),
            Some(d(-20)),
            Some(d(280)),
            Some(d(28)),
            Some(d(308)),
        ]
    );
}

#[test]
fn tax_above_a_subtotal_taxes_only_the_item_above() {
    let doc = "\
| Description | Qty | Rate | Amount |
|---|---|---|---|
| A | 1 | $100 | |
| B | 1 | $200 | |
| Sales tax | | 10% | |
| Discount | | -10% | |
| | | **Total due** | |
";
    // The tax applies to B; the discount skips the tax and also applies to B.
    assert_eq!(
        amounts(&bill(doc), &usd()),
        [
            Some(d(100)),
            Some(d(200)),
            Some(d(20)),
            Some(d(-20)),
            Some(d(300)),
        ]
    );
}

#[test]
fn a_missing_alignment_row_is_added_by_role() {
    let doc = "Bill:\n\n| Description | Qty | Rate | Amount |\n| Work | 2 | $5 | |\n\nThanks.\n";
    assert_eq!(
        recalc(doc),
        "Bill:\n\n\
| Description | Qty | Rate | Amount |\n\
|:------------|----:|-----:|-------:|\n\
| Work        | 2   | $5   | $10.00 |\n\nThanks.\n"
    );
}

#[test]
fn extra_spaces_and_ragged_widths_read_normally() {
    let doc = "|   Description |Qty|   Rate   |Amount|\n|---|:-:|---|---|\n|  Work   |   2|$5   |   $10.00 |\n";
    let table = bill(doc);
    assert_eq!(amounts(&table, &usd()), [Some(d(10))]);
    // Nothing disagrees, so the user's layout is left alone.
    assert_eq!(recalc_markdown(doc, &Words::default(), &usd()), Ok(None));
}

#[test]
fn both_negative_spellings_are_fixed_adjustments() {
    for credit in ["$-800", "-$800", "- $800", "−$800"] {
        let doc = format!(
            "| Description | Amount |\n|---|---|\n| Work | $1,000 |\n| Credit | {credit} |\n| **Total** | |\n"
        );
        let table = bill(&doc);
        let evaluation = table
            .evaluate(&Words::default(), &usd())
            .expect("evaluates");
        assert_eq!(
            evaluation.rows[1].kind,
            RowKind::FixedAdjustment,
            "{credit}"
        );
        assert_eq!(evaluation.total, d(200), "{credit}");
        // The written credit keeps its spelling; only the total is written.
        let out = recalc(&doc);
        assert!(out.contains(credit), "{out}");
        assert!(out.contains("**$200.00**"), "{out}");
    }
}

#[test]
fn nbsp_cells_are_blank() {
    let doc = "\
| Description | Qty | Rate | Amount |
|---|---|---|---|
| Work | 2 | $5 | &nbsp; |
| &nbsp; | &nbsp; | &nbsp; | &nbsp; |
| Discount | &nbsp; | -50% | |
";
    let evaluation = bill(doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    assert_eq!(evaluation.rows[1].kind, RowKind::Spacer);
    assert!(matches!(evaluation.rows[2].kind, RowKind::Adjustment(_)));
    assert_eq!(evaluation.total, d(5));
}

#[test]
fn escaped_pipes_stay_in_their_cell() {
    let doc = "\
| Description | Qty | Rate | Amount |
|---|---|---|---|
| Design \\| build | 2 | $5 | |
";
    let table = bill(doc);
    assert_eq!(
        table.evaluate(&Words::default(), &usd()).expect("ok").rows[0]
            .at
            .description,
        "Design \\| build"
    );
    assert_eq!(
        recalc(doc),
        "\
| Description     | Qty | Rate | Amount |
|-----------------|-----|------|--------|
| Design \\| build | 2   | $5   | $10.00 |
"
    );
}

#[test]
fn supporting_tables_and_fenced_tables_are_skipped() {
    let doc = "\
```
| Description | Amount |
|---|---|
| Fenced | $1 |
```

| Date | Hours |
|---|---|
| 2026-11-01 | 3 |

| Item | Amount |
|---|---|
| Real | $7 |
";
    let table = bill(doc);
    assert_eq!(
        table.cell(0, invoicer_core::table::columns::Role::Description),
        "Real"
    );
}

#[test]
fn german_words_and_a_euro_style() {
    let words = Words::from_ledgeline_toml(
        r#"
[invoicer.words]
description = ["Beschreibung", "Leistung"]
quantity = ["Menge", "Stunden"]
rate = ["Preis", "Einzelpreis"]
amount = ["Betrag", "Gesamt"]
subtotal = ["Zwischensumme"]
total = ["Gesamtbetrag", "Summe"]
"#,
    )
    .expect("reads");
    let journal =
        ledgeline_core::parse_journal("commodity 1.000,00 €\n", "books.journal").expect("parses");
    let eur = MoneyStyle::of_journal(&journal, "€");
    let doc = "\
| Leistung | Stunden | Einzelpreis | Betrag | *Konto* |
|:--|--:|--:|--:|:--|
| Beratung | 10 | 125,50 € | | erlöse:beratung |
| Rabatt | | -5% | | |
| | | **Zwischensumme** | | |
| USt. | | 19% | | verbindlichkeiten:ust |
| | | **Gesamtbetrag** | | |
";
    let table = BillTable::find(doc, &words).expect("reads").expect("found");
    let evaluation = table.evaluate(&words, &eur).expect("evaluates");
    assert_eq!(evaluation.total, Dec::new(141_878, 2)); // 1255 − 62,75 = 1192,25; + 226,53
    let out = recalc_markdown(doc, &words, &eur)
        .expect("ok")
        .expect("changed");
    assert!(out.contains("| **1.418,78 €**"), "{out}");
    assert!(out.contains("| -62,75 €"), "{out}");
    // The English defaults don't recognize it.
    assert_eq!(BillTable::find(doc, &Words::default()), Ok(None));
}

#[test]
fn check_names_the_row_and_line() {
    let doc = "Intro\n\n\
| Description | Qty | Rate | Amount |
|---|---|---|---|
| Hours | 10 | $250 | $2,500.00 |
| Loyalty discount | | -10% | -$200.00 |
| | | **Total** | |
";
    let problems = bill(doc).check(&Words::default(), &usd()).expect("checks");
    let messages: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "row 2 \u{201c}Loyalty discount\u{201d} (line 6): the amount says -$200.00, but the arithmetic gives -$250.00",
            "row 3 (line 7): the amount is blank; it should be **$2,250.00**",
        ]
    );
}

#[test]
fn errors_name_the_row() {
    let cases = [
        (
            "| Widget | x | $5 | |",
            "row 2 \u{201c}Widget\u{201d} (line 4): `x` is not a number",
        ),
        (
            "| Widget | 2 | €5 | |",
            "row 2 \u{201c}Widget\u{201d} (line 4): `€5` is not an amount in $",
        ),
        (
            "| Widget | 2 | 5% | |",
            "row 2 \u{201c}Widget\u{201d} (line 4): a percentage rate makes it",
        ),
        (
            "| Note | | | |",
            "row 2 \u{201c}Note\u{201d} (line 4): it has no amount",
        ),
        (
            "| Bad | | | abc |",
            "row 2 \u{201c}Bad\u{201d} (line 4): `abc` is not an amount",
        ),
        (
            "| W | 1 | $1 | | a → b |",
            "row 2 \u{201c}W\u{201d} (line 4): `a → b` is not a deferral",
        ),
        (
            "| W | 1 | $1 | | x | y |",
            "row 2 \u{201c}W\u{201d} (line 4): it has 6 cells but the header has 5",
        ),
    ];
    for (row, needle) in cases {
        let doc = format!(
            "| Description | Qty | Rate | Amount | *Account* |\n|---|---|---|---|---|\n| First | 1 | $1 | | |\n{row}\n"
        );
        let error = BillTable::find(&doc, &Words::default())
            .and_then(|t| t.expect("found").evaluate(&Words::default(), &usd()))
            .expect_err(row)
            .to_string();
        assert!(error.starts_with(needle), "{row}\n  got: {error}");
    }
    let doc = "| Description | Rate | Amount |\n|---|---|---|\n| Discount | -10% | |\n";
    let error = bill(doc)
        .evaluate(&Words::default(), &usd())
        .expect_err("no target");
    assert!(matches!(error, TableError::Row { .. }));
    assert!(
        error.to_string().contains("no item or total above it"),
        "{error}"
    );
}

#[test]
fn rewriting_one_cell_touches_one_line_when_widths_hold() {
    let doc = worked_example_3_3();
    let table = bill(&doc);
    let edited = table.with_cell(1, 1, "12").expect("in range");
    let edits = edited.edits(&doc).expect("writes");
    assert_eq!(edits.len(), 1);
    let out = apply_edits(&doc, &edits);
    assert!(
        out.contains("| Senior architect hours     | 12  | $250 "),
        "{out}"
    );
    // Recomputing then rewrites the dependent amounts, still aligned.
    let out = recalc(&out);
    assert!(out.contains("| $3,000.00      |"), "{out}");
    assert!(out.contains("| -$300.00       |"), "{out}");
    let rebuilt = bill(&out);
    assert_eq!(rebuilt.render(), table_text(&out, &rebuilt));
}

#[test]
fn a_cell_that_widens_its_column_realigns_every_row() {
    let doc = "| Description | Amount |\n|---|---|\n| A | $1 |\n| B | $2 |\n";
    let table = bill(doc);
    let out = apply_edits(
        doc,
        &table
            .with_cell(0, 0, "A much longer description")
            .expect("ok")
            .edits(doc)
            .expect("ok"),
    );
    assert_eq!(
        out,
        "\
| Description               | Amount |
|---------------------------|--------|
| A much longer description | $1     |
| B                         | $2     |
"
    );
}

#[test]
fn blank_accounts_fall_through_the_default_chain() {
    let doc = "\
| SKU | Description | Qty | Rate | Amount | *Account* |
|---|---|---|---|---|---|
| ARCH | Architect | 1 | $800 | | |
| | Ad hoc | 1 | $100 | | |
| SAAS | Platform | 1 | $1,200 | | |
";
    let evaluation = bill(doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let saas = BooksEntry::parse("liabilities:deferred-revenue → revenue:saas, 12 months")
        .expect("parses")
        .expect("present");
    let defaults = AccountDefaults {
        catalog: [
            (
                ItemCode("ARCH".into()),
                BooksEntry::Account(account("revenue:consulting")),
            ),
            (ItemCode("SAAS".into()), saas.clone()),
        ]
        .into_iter()
        .collect(),
        settings: Some(account("revenue:general")),
        inferred: Some(account("revenue")),
        ..AccountDefaults::default()
    };
    let sources: Vec<(String, AccountSource)> = evaluation
        .rows
        .iter()
        .map(|row| {
            let (entry, source) = resolve(row, &defaults).expect("resolves");
            (entry.account().to_string(), source)
        })
        .collect();
    assert_eq!(
        sources,
        [
            ("revenue:consulting".into(), AccountSource::Catalog),
            ("revenue:general".into(), AccountSource::Settings),
            (
                "liabilities:deferred-revenue".into(),
                AccountSource::Catalog
            ),
        ]
    );
    let customer = AccountDefaults {
        customer: Some(BooksEntry::Account(account("revenue:customer-x"))),
        ..defaults.clone()
    };
    assert_eq!(
        resolve(&evaluation.rows[1], &customer).map(|(_, s)| s),
        Some(AccountSource::Customer)
    );
    let inferred = AccountDefaults {
        settings: None,
        ..defaults
    };
    assert_eq!(
        resolve(&evaluation.rows[1], &inferred).map(|(_, s)| s),
        Some(AccountSource::Inferred)
    );
    let booking = split(&evaluation, &inferred, &usd()).expect("splits");
    assert_eq!(booking.lines[2].books, saas);
    let error = split(&evaluation, &AccountDefaults::default(), &usd()).expect_err("unresolved");
    assert!(matches!(error, BookingError::Unresolved(_)));
    assert!(
        error
            .to_string()
            .starts_with("row 1 \u{201c}Architect\u{201d} (line 3): no account"),
        "{error}"
    );
}

#[test]
fn a_discount_account_takes_blank_discounts_but_not_blank_surcharges() {
    let doc = "\
| Description | Qty | Rate | Amount | *Account* |
|---|---|---|---|---|
| Work | 1 | $1,000 | | revenue:work |
| Discount | | -10% | | |
| Rush fee | | 5% | | |
| | | **Total** | | |
";
    let evaluation = bill(doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let defaults = AccountDefaults {
        discount: Some(account("revenue:discounts")),
        ..AccountDefaults::default()
    };
    let booking = split(&evaluation, &defaults, &usd()).expect("splits");
    let lines: Vec<(String, Dec)> = booking
        .lines
        .iter()
        .map(|l| (l.books.account().to_string(), l.amount))
        .collect();
    assert_eq!(
        lines,
        [
            ("revenue:work".into(), d(1_050)),
            ("revenue:discounts".into(), d(-100)),
        ]
    );
    assert_eq!(booking.total, d(950));
}

#[test]
fn rows_below_the_final_total_cannot_be_booked() {
    let doc = "\
| Description | Amount | *Account* |
|---|---|---|
| Work | $10 | revenue |
| **Total** | | |
| Late | $5 | revenue |
";
    let evaluation = bill(doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    let error = split(&evaluation, &AccountDefaults::default(), &usd()).expect_err("stray");
    assert_eq!(
        error.to_string(),
        "row 3 \u{201c}Late\u{201d} (line 5): it is below the final total, which doesn't include it"
    );
}

#[test]
fn a_blank_adjustment_with_no_revenue_to_spread_over_is_refused() {
    let books = |doc: &str| {
        let evaluation = bill(doc)
            .evaluate(&Words::default(), &usd())
            .expect("evaluates");
        split(&evaluation, &AccountDefaults::default(), &usd())
    };
    // No item rows at all above the subtotal.
    let none = "\
| Description | Rate | Amount | *Account* |
|---|---|---|---|
| Pass-through | | +$10 | liabilities:pass-through |
| **Subtotal** | | | |
| Discount | -10% | | |
| **Total** | | | |
";
    let error = books(none).expect_err("nothing to spread over");
    assert!(matches!(error, BookingError::CannotSpread(_)));
    assert!(
        error
            .to_string()
            .starts_with("row 3 \u{201c}Discount\u{201d} (line 5)"),
        "{error}"
    );
    // Item revenue that nets to zero across its lines.
    let netted = "\
| Description | Qty | Rate | Amount | *Account* |
|---|---|---|---|---|
| Work | 1 | $10 | | revenue:a |
| Credit | 1 | $-10 | | revenue:b |
| **Subtotal** | | | | |
| Fee | | | +$5 | |
| **Total** | | | | |
";
    assert!(matches!(books(netted), Err(BookingError::CannotSpread(_))));
    // An own-account row on the same account doesn't count toward the
    // weights: the item's $10 does, so the fee spreads onto it.
    let own = "\
| Description | Amount | *Account* |
|---|---|---|
| Work | $10 | revenue |
| Refund | -$10 | revenue |
| **Subtotal** | | |
| Fee | +$5 | |
| **Total** | | |
";
    let booking = books(own).expect("splits");
    assert_eq!(booking.lines.len(), 1);
    assert_eq!(booking.lines[0].amount, d(5));
    assert_eq!(booking.total, d(5));
}

#[test]
fn a_subtotal_discount_spreads_over_revenue_lines_only() {
    let doc = "\
| Description | Qty | Rate | Amount | *Account* |
|---|---|---|---|---|
| Design | 1 | $1,000 | | revenue:a |
| Build | 1 | $500 | | revenue:b |
| Sales tax | | 10% | | liabilities:sales-tax |
| Less deposit (Invoice 104) | | | -$200.00 | liabilities:customer-deposits |
| Support | 1 | $600 | | liabilities:deferred-revenue → revenue:support, 12 months |
| | | **Subtotal** | | |
| Early-payment discount | | -10% | | |
| | | **Total due** | | |
";
    let evaluation = bill(doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    assert_eq!(evaluation.rows[5].amount, Some(d(1_950)));
    assert_eq!(evaluation.total, d(1_755));
    let booking = split(&evaluation, &AccountDefaults::default(), &usd()).expect("splits");
    let lines: Vec<(String, Dec)> = booking
        .lines
        .iter()
        .map(|l| (l.books.account().to_string(), l.amount))
        .collect();
    // -$195 over the item revenue only, 1,000 : 500 : 600 (the deferred item
    // included): -92.86 / -46.43 / -55.71. The tax and the deposit keep
    // exactly their own amounts.
    assert_eq!(
        lines,
        [
            ("revenue:a".into(), cents(90_714)),
            ("revenue:b".into(), cents(45_357)),
            ("liabilities:sales-tax".into(), d(50)),
            ("liabilities:customer-deposits".into(), d(-200)),
            ("liabilities:deferred-revenue".into(), cents(54_429)),
        ]
    );
    assert_eq!(booking.lines[2].rows, [2]);
    assert_eq!(booking.lines[3].rows, [3]);
    assert_eq!(booking.total, d(1_755));
}

#[test]
fn a_header_with_two_columns_for_one_role_names_both() {
    let doc = "Intro\n\n| Item | Description | Amount |\n|---|---|---|\n| A | B | $1 |\n";
    let error = BillTable::find(doc, &Words::default()).expect_err("ambiguous");
    assert_eq!(
        error.to_string(),
        "the bill table's header (line 3): `Item` and `Description` are both description columns; rename one"
    );
    // The catalog's `Code` header is the item column.
    let doc = "| Code | Description | Amount |\n|---|---|---|\n| ARCH | Architect | $1 |\n";
    let evaluation = bill(doc)
        .evaluate(&Words::default(), &usd())
        .expect("evaluates");
    assert_eq!(evaluation.rows[0].item, Some(ItemCode("ARCH".into())));
}
