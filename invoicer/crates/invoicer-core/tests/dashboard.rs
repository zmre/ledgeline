//! The receivables dashboard (PLAN 5a) on the fixtures, read in place
//! through the public API (nothing is written), on the e2e suite's date.

use invoicer_core::books::Basis;
use invoicer_core::doc::Money;
use invoicer_core::index::DocumentIndex;
use invoicer_core::reconcile::BookStates;
use invoicer_core::settings::Settings;
use invoicer_core::views::dashboard::{Dashboard, DashboardOptions, dashboard, recognition_view};
use ledgeline_core::accrual::ops::Books;
use ledgeline_core::vault::LocalVault;
use std::path::PathBuf;

/// The e2e suite's fixed date (`FIXED_NOW`).
const TODAY: &str = "2026-11-17";

fn on(name: &str, today: &str, options: Option<DashboardOptions>) -> Dashboard {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .canonicalize()
        .expect("fixture exists");
    let loaded = Books::open(&dir.join("books.journal"))
        .expect("opens")
        .load()
        .expect("loads");
    let vault = LocalVault::new(&dir);
    let settings = Settings::read(&vault).expect("settings");
    let options = options.unwrap_or_else(|| DashboardOptions::of(&settings));
    let index = DocumentIndex::open(&vault, settings, Money::of_journal(loaded.journal()));
    let states = BookStates::build(loaded.journal(), Basis::of(&loaded));
    let recognition = loaded
        .recognition_exists()
        .then(|| recognition_view(&loaded.due(today), today));
    dashboard(&index, &states, recognition, today, &options)
}

fn texts(d: &Dashboard) -> Vec<(String, String, usize)> {
    d.commodities[0]
        .buckets
        .iter()
        .map(|b| (b.key.clone(), b.text.clone(), b.count))
        .collect()
}

fn row(key: &str, text: &str, count: usize) -> (String, String, usize) {
    (key.to_string(), text.to_string(), count)
}

#[test]
fn the_accrual_dashboard() {
    let d = on("accrual", TODAY, None);
    assert_eq!(d.basis, Basis::Accrual);
    assert_eq!(d.invoices, 8);
    assert_eq!(d.commodities.len(), 1);
    let usd = &d.commodities[0];
    assert_eq!(usd.commodity, "$");
    assert_eq!(
        (usd.outstanding.text.as_str(), usd.outstanding.count),
        ("$22,850.00", 7)
    );
    assert_eq!(
        (usd.overdue.text.as_str(), usd.overdue.count),
        ("$12,700.00", 5)
    );
    assert_eq!(
        (usd.not_due.text.as_str(), usd.not_due.count),
        ("$10,150.00", 2)
    );
    // Receivables booked in the journal alone (no invoice file) count, as
    // in Ledgeline; the SPA says so beside the bar.
    assert_eq!(
        (usd.unfiled.text.as_str(), usd.unfiled.count),
        ("$11,300.00", 4)
    );
    assert_eq!(
        usd.scale.text, "$22,850",
        "above $10,000 the scale is the total"
    );
    assert_eq!(
        texts(&d),
        [
            row("current", "$10,150.00", 2),
            row("1-30", "$7,000.00", 2),
            row("31-60", "$1,500.00", 1),
            row("61-90", "$1,200.00", 1),
            row("91-", "$3,000.00", 1),
        ]
    );
    let oldest = usd.oldest_overdue.as_ref().expect("something is overdue");
    assert_eq!(
        (
            oldest.number.as_deref(),
            oldest.customer.as_deref(),
            oldest.due.as_str()
        ),
        (Some("2001"), Some("Tailspin"), "2026-07-15")
    );
    assert_eq!(oldest.days_past_due, 125);
    let dso = usd.dso.as_ref().expect("billed in the window");
    assert_eq!((dso.value.as_deref(), dso.days), (Some("58.9"), 90));
    assert_eq!(
        (
            d.average_days_to_pay.invoices,
            d.average_days_to_pay.average.as_deref()
        ),
        (3, Some("25.3"))
    );
    assert_eq!(
        (d.drafts.count, d.drafts.totals[0].text.as_str()),
        (1, "$13,600.00")
    );
    let recognition = d.recognition.expect("the fixture has schedules");
    assert_eq!(
        (
            recognition.due,
            recognition.closed,
            recognition.earliest.as_deref()
        ),
        (10, 1, Some("2026-11-01"))
    );
}

#[test]
fn the_cash_dashboard() {
    let d = on("cash", TODAY, None);
    assert_eq!(d.basis, Basis::Cash);
    assert_eq!(d.invoices, 7);
    let usd = &d.commodities[0];
    assert_eq!(
        (usd.outstanding.text.as_str(), usd.outstanding.count),
        ("$9,700.00", 3)
    );
    assert_eq!(usd.overdue.count, 0);
    assert_eq!(usd.unfiled.count, 0, "cash books owe by the markdown");
    assert_eq!(usd.scale.text, "$10,000", "the scale's minimum");
    assert!(usd.oldest_overdue.is_none());
    assert_eq!(
        usd.dso.as_ref().and_then(|d| d.value.as_deref()),
        Some("48.0")
    );
    assert_eq!(
        (
            d.average_days_to_pay.invoices,
            d.average_days_to_pay.average.as_deref()
        ),
        (2, Some("10.5"))
    );
    assert_eq!(
        (d.drafts.count, d.drafts.totals[0].text.as_str()),
        (1, "$3,200.00")
    );
    assert_eq!(d.recognition, None, "no recognition file");
}

#[test]
fn bucket_edges_and_the_dso_window_follow_the_options() {
    let options = DashboardOptions {
        aging_edges: vec![45],
        dso_days: 30,
    };
    let d = on("accrual", TODAY, Some(options));
    assert_eq!(d.aging_buckets, [45]);
    assert_eq!(d.dso_days, 30);
    assert_eq!(
        texts(&d),
        [
            row("current", "$10,150.00", 2),
            row("1-45", "$8,500.00", 3),
            row("46-", "$4,200.00", 2),
        ]
    );
    assert_eq!(d.commodities[0].dso.as_ref().map(|d| d.days), Some(30));
}

#[test]
fn a_later_today_ages_everything() {
    // A year on, with nothing paid, everything sent is 91+ and nothing is current.
    let d = on("accrual", "2027-11-17", None);
    let buckets = texts(&d);
    assert_eq!(buckets[0], row("current", "$0.00", 0));
    assert_eq!(buckets[4].1, d.commodities[0].outstanding.text);
}
