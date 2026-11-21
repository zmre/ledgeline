//! The invoicer fixtures (`invoicer/fixtures/{accrual,cash}`, PLAN 0d) read
//! cleanly end to end through the public API: every document, customer,
//! catalog row, snippet and template, with no problems, and every booked
//! invoice's total equal to its receivable posting in the books.

use invoicer_core::catalog;
use invoicer_core::doc::{Money, Paid, Status};
use invoicer_core::index::{DocumentIndex, ListFilter};
use invoicer_core::settings::Settings;
use invoicer_core::snippets;
use invoicer_core::templates;
use ledgeline_core::accrual::ops::Books;
use ledgeline_core::vault::LocalVault;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .canonicalize()
        .expect("fixture exists")
}

fn open(
    name: &str,
) -> (
    LocalVault,
    DocumentIndex,
    ledgeline_core::accrual::ops::Loaded,
) {
    let dir = fixture(name);
    let main = dir.join("books.journal");
    let loaded = Books::open(&main).expect("opens").load().expect("loads");
    let vault = LocalVault::new(&dir);
    let settings = Settings::read(&vault).expect("settings");
    let money = Money::of_journal(loaded.journal());
    let index = DocumentIndex::open(&vault, settings, money);
    (vault, index, loaded)
}

fn statuses(index: &DocumentIndex) -> Vec<(String, Option<Status>, Option<Paid>)> {
    index
        .list(&ListFilter::default())
        .iter()
        .map(|d| {
            (
                d.number().unwrap_or("").to_string(),
                d.header.status,
                d.header.paid.clone(),
            )
        })
        .collect()
}

#[test]
fn the_accrual_fixture_is_clean_and_matches_its_books() {
    let (vault, index, loaded) = open("accrual");
    let problems = index.check(&vault);
    assert!(problems.is_empty(), "{problems:#?}");
    assert!(
        index.site().warnings().is_empty(),
        "{:?}",
        index.site().warnings()
    );
    let numbers: Vec<String> = statuses(&index).into_iter().map(|(n, _, _)| n).collect();
    assert_eq!(
        numbers,
        ["108", "105", "107", "104", "103", "102", "101", "106"]
    );
    assert_eq!(
        index
            .list(&ListFilter {
                status: Some(Status::Draft),
                ..ListFilter::default()
            })
            .len(),
        1
    );
    assert_eq!(
        index
            .list(&ListFilter {
                status: Some(Status::Void),
                ..ListFilter::default()
            })
            .len(),
        1
    );
    let partial = index.by_number("107");
    assert!(matches!(partial[0].header.paid, Some(Paid::Partial { .. })));
    // The files alone give 109; the books' plan-45 invoices (2001-2004) give
    // 2005 once Phase 2 passes them in.
    assert_eq!(index.next_number([]).expect("next").as_str(), "109");
    assert_eq!(index.next_number(["2004"]).expect("next").as_str(), "2005");

    // Every booked (sent, not void) invoice's total is its AR posting.
    for doc in index
        .docs()
        .filter(|d| d.header.status == Some(Status::Sent))
    {
        let number = doc.number().expect("number");
        let found = loaded
            .find_document(number)
            .unwrap_or_else(|| panic!("invoice {number} is booked"));
        let posting = &found.transaction.postings[found.settlement.posting];
        let booked = posting.amounts[0].quantity;
        let total = doc.summary.as_ref().expect("summary").total;
        assert_eq!(booked, total, "invoice {number}");
        let tag = found
            .transaction
            .tags
            .iter()
            .find(|(k, _)| k == "document")
            .map(|(_, v)| v.as_str());
        assert_eq!(
            tag,
            Some(doc.path.as_str()),
            "invoice {number}'s document: tag"
        );
    }
    // The draft is not booked (I2).
    assert!(loaded.find_document("108").is_none());

    assert_eq!(index.customers().len(), 6);
    let (catalogs, catalog_problems) =
        catalog::load(&vault, index.site(), &index.settings().words, index.money());
    assert!(catalog_problems.is_empty(), "{catalog_problems:?}");
    assert_eq!(catalogs[0].items.len(), 4);
    assert_eq!(snippets::snippets(index.site()).len(), 2);
    let templates = templates::discover(&vault, index.site());
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].name, "MyCo default");
    assert!(index.rebuild_and_diff(&vault).is_empty());
}

#[test]
fn the_cash_fixture_is_clean() {
    let (vault, index, loaded) = open("cash");
    let problems = index.check(&vault);
    assert!(problems.is_empty(), "{problems:#?}");
    let all = statuses(&index);
    assert_eq!(all.len(), 7);
    assert_eq!(index.next_number([]).expect("next").as_str(), "208");
    // Cash books book nothing at send: no invoice has a receivable posting.
    assert!(
        index
            .docs()
            .all(|d| loaded.find_document(d.number().unwrap_or("")).is_none())
    );
    let total = |n: &str| {
        index.by_number(n)[0]
            .summary
            .as_ref()
            .expect("summary")
            .total_text
            .clone()
    };
    assert_eq!(total("201"), "$3,000.00");
    assert_eq!(total("207"), "$7,000.00");
}
