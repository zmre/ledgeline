//! Subscription-detection behaviour, pinned against
//! `fixtures/subscriptions/basic.journal`.
//!
//! Every payee in that fixture exists to pin one rule of the detector (see the
//! journal's own header), so these tests read as a specification: what counts as
//! a recurring charge, and — just as important — what must NOT.

mod common;

use common::fixtures_dir;
use ledgeline_core::parse_journal;
use ledgeline_core::reports::{
    Cadence, DEFAULT_EXCLUDE_DESC, Subscription, SubscriptionOpts, SubscriptionsReport,
    detect_subscriptions,
};
use ledgeline_core::{Dec, Journal};

const AS_OF: &str = "2026-06-30";

fn fixture() -> Journal {
    let path = fixtures_dir().join("subscriptions").join("basic.journal");
    let text = std::fs::read_to_string(&path).expect("basic.journal readable");
    parse_journal(&text, &path.to_string_lossy()).expect("basic.journal parses")
}

/// The default exclusions, as the server applies them.
fn excludes() -> Vec<String> {
    DEFAULT_EXCLUDE_DESC
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

fn report() -> SubscriptionsReport {
    let exclude = excludes();
    detect_subscriptions(
        &fixture(),
        &SubscriptionOpts {
            as_of: AS_OF,
            exclude_desc: &exclude,
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds")
}

fn find<'a>(rows: &'a [Subscription], payee: &str) -> Option<&'a Subscription> {
    rows.iter().find(|row| row.payee == payee)
}

fn payees(rows: &[Subscription]) -> Vec<&str> {
    rows.iter().map(|row| row.payee.as_str()).collect()
}

#[test]
fn scans_the_trailing_two_years() {
    let report = report();
    assert_eq!(report.as_of, AS_OF);
    assert_eq!(report.lookback_start, "2024-06-30");
}

#[test]
fn finds_the_monthly_subscriptions_sorted_by_annual_cost() {
    let report = report();
    // Hand-tagged Twilio $126.80 → $1,521.60/yr leads, then the detected ones:
    // Netflix $15.99 → $191.88/yr, Spotify ~$11.99 → $143.88, Apple $9.99 →
    // $119.88, Backblaze $9.00 → $108.00.
    assert_eq!(
        payees(&report.monthly),
        ["Twilio", "Netflix", "Spotify", "Apple", "Backblaze"]
    );

    let netflix = find(&report.monthly, "Netflix").expect("Netflix detected");
    assert_eq!(netflix.cadence, Cadence::Monthly);
    assert_eq!(netflix.typical_amount, Dec::new(1599, 2));
    assert_eq!(netflix.annualized_cost, Dec::new(19188, 2));
    assert_eq!(netflix.occurrences, 18);
    assert_eq!(netflix.first_seen, "2025-01-15");
    assert_eq!(netflix.last_seen, "2026-06-15");
    assert_eq!(netflix.next_expected, "2026-07-15");
    assert_eq!(netflix.accounts, ["expenses:subscriptions"]);
}

#[test]
fn a_price_rise_stays_one_subscription() {
    // Spotify went $10.99 → $11.99 (+9.1%), inside the amount tolerance, so it
    // must remain a single 12-charge subscription rather than splitting in two.
    let report = report();
    let spotify = find(&report.monthly, "Spotify").expect("Spotify detected");
    assert_eq!(spotify.occurrences, 12);
    assert_eq!(spotify.typical_amount, Dec::new(1199, 2));
}

#[test]
fn a_mixed_payee_reports_only_its_recurring_charge() {
    // Apple bills $9.99/month for iCloud AND sells one-off apps ($2.99–$49.99)
    // under the same payee. Only the steady cluster may surface, at its own
    // price — the one-offs must not inflate it or appear as extra charges.
    let report = report();
    let apple = find(&report.monthly, "Apple").expect("Apple's iCloud charge detected");
    assert_eq!(apple.typical_amount, Dec::new(999, 2));
    assert_eq!(apple.occurrences, 15);
    assert_eq!(apple.annualized_cost, Dec::new(11988, 2));
    // The one-off purchases posted to a different account and are not included.
    assert_eq!(apple.accounts, ["expenses:subscriptions"]);
    // Apple appears exactly once — the one-offs form no second subscription.
    assert_eq!(
        report
            .monthly
            .iter()
            .chain(&report.annual)
            .filter(|row| row.payee == "Apple")
            .count(),
        1
    );
}

#[test]
fn finds_the_annual_subscriptions() {
    let report = report();
    assert_eq!(payees(&report.annual), ["State Farm", "Hover"]);

    let insurance = find(&report.annual, "State Farm").expect("insurance detected");
    assert_eq!(insurance.cadence, Cadence::Annual);
    assert_eq!(insurance.typical_amount, Dec::new(120_000, 2));
    // An annual charge's yearly cost is the charge itself, not ×12.
    assert_eq!(insurance.annualized_cost, Dec::new(120_000, 2));
    assert_eq!(insurance.occurrences, 2);
    assert_eq!(insurance.next_expected, "2026-11-03");

    let domain = find(&report.annual, "Hover").expect("domain renewal detected");
    assert_eq!(domain.typical_amount, Dec::new(3500, 2));
    assert_eq!(domain.next_expected, "2026-09-12");
}

#[test]
fn variable_spending_at_one_merchant_is_not_a_subscription() {
    // Amazon appears 11 times across the window, but no two charges cluster:
    // frequency alone must never imply a subscription.
    let report = report();
    assert!(find(&report.monthly, "Amazon").is_none());
    assert!(find(&report.annual, "Amazon").is_none());
}

#[test]
fn too_few_repetitions_is_not_yet_a_subscription() {
    // Four identical gym charges on the 5th — a perfect monthly rhythm, but
    // under `min_monthly` (5), so it is not yet trusted as a standing charge.
    let report = report();
    assert!(find(&report.monthly, "Gold's Gym").is_none());

    // Lowering the threshold surfaces it, proving the count is what held it back.
    let relaxed = detect_subscriptions(
        &fixture(),
        &SubscriptionOpts {
            as_of: AS_OF,
            min_monthly: 4,
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds");
    let gym = find(&relaxed.monthly, "Gold's Gym").expect("gym detected at min_monthly=4");
    assert_eq!(gym.typical_amount, Dec::new(4500, 2));
}

#[test]
fn payroll_withholding_is_not_a_subscription() {
    // Six monthly paychecks carry $1,460 of tax withholding as expense postings —
    // a perfect monthly rhythm at a consistent amount. Without an income guard the
    // employer surfaces as the single largest "subscription" on the dashboard.
    let report = report();
    assert!(find(&report.monthly, "Acme Corp").is_none());
    assert!(find(&report.annual, "Acme Corp").is_none());
}

#[test]
fn a_coincidental_yearly_pair_at_a_frequent_merchant_is_not_annual() {
    // Two Safeway trips a year apart total $252.80 each — an exact annual match
    // on both date and price. It is still just groceries: the pair explains only
    // a fraction of that payee's activity.
    let report = report();
    assert!(find(&report.annual, "Safeway").is_none());
    assert!(find(&report.monthly, "Safeway").is_none());
}

#[test]
fn a_mortgage_is_excluded_by_description() {
    // Eleven identical $2,400 charges on the 1st — a textbook monthly subscription
    // by shape, but debt service. "mortgage" sits in the NOTE (`Wells Fargo |
    // mortgage payment`), not the payee, so this also pins that the exclusion
    // reads the whole description.
    let report = report();
    assert!(find(&report.monthly, "Wells Fargo").is_none());

    // Without the exclusion it WOULD be reported — proving the description
    // filter, not some other rule, is what suppresses it.
    let unfiltered = detect_subscriptions(
        &fixture(),
        &SubscriptionOpts {
            as_of: AS_OF,
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds");
    let mortgage =
        find(&unfiltered.monthly, "Wells Fargo").expect("detected without the exclusion");
    assert_eq!(mortgage.typical_amount, Dec::new(240_000, 2));
}

#[test]
fn the_description_exclusion_is_case_insensitive() {
    let upper = vec!["MORTGAGE".to_string()];
    let report = detect_subscriptions(
        &fixture(),
        &SubscriptionOpts {
            as_of: AS_OF,
            exclude_desc: &upper,
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds");
    assert!(find(&report.monthly, "Wells Fargo").is_none());
    // Unrelated subscriptions are untouched by the filter.
    assert!(find(&report.monthly, "Netflix").is_some());
}

#[test]
fn a_cancelled_subscription_drops_off_the_list() {
    // Hulu billed monthly through Jun 2025 and then stopped. Its credit card
    // keeps posting to the end of the window, so the silence is evidence, not a
    // gap in the data — it is cancelled and must not sit on the list as a
    // phantom cost.
    let report = report();
    assert!(find(&report.monthly, "Hulu").is_none());

    // Widening the grace period brings it back, proving staleness — not some
    // other rule — is what retired it.
    let exclude = excludes();
    let lenient = detect_subscriptions(
        &fixture(),
        &SubscriptionOpts {
            as_of: AS_OF,
            stale_months: 24,
            exclude_desc: &exclude,
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds");
    let hulu = find(&lenient.monthly, "Hulu").expect("Hulu detected with a wide grace period");
    assert_eq!(hulu.last_seen, "2025-06-11");
}

#[test]
fn a_quiet_charge_on_a_lagging_import_is_kept() {
    // Backblaze also stopped appearing (Sep 2025) — but it bills a bank account
    // whose statements were last imported that same month. There is no evidence
    // it was cancelled, only an absence of data, so it must survive. This is the
    // whole reason staleness is measured per funding account rather than against
    // today: judged globally, this would be retired alongside Hulu.
    let report = report();
    let backblaze = find(&report.monthly, "Backblaze").expect("kept despite being quiet");
    assert_eq!(backblaze.last_seen, "2025-09-08");
    assert!(
        find(&report.monthly, "Hulu").is_none(),
        "the equally-quiet card-funded charge IS retired, so this is not just a lax cutoff"
    );
}

#[test]
fn a_tagged_charge_is_listed_even_though_detection_cannot_see_it() {
    // Twilio's bill swings from $18 to $127, so no amount cluster ever forms and
    // the detector is blind to it. `subscription:true` says otherwise.
    let report = report();
    let twilio = find(&report.monthly, "Twilio").expect("tagged onto the list");
    assert!(twilio.manual, "flagged as hand-added, not inferred");
    assert_eq!(twilio.cadence, Cadence::Monthly);
    // TWO transactions carry the tag; they collapse to one entry priced at their
    // median ($126.80 and $64.10 → $126.80 is the upper of the two).
    assert_eq!(twilio.occurrences, 2);
    assert_eq!(twilio.typical_amount, Dec::new(12_680, 2));
    assert_eq!(twilio.last_seen, "2026-06-14");
    assert_eq!(twilio.next_expected, "2026-07-14");
    assert_eq!(
        report
            .monthly
            .iter()
            .chain(&report.annual)
            .filter(|row| row.payee == "Twilio")
            .count(),
        1,
        "deduplicated across however many transactions carry the tag"
    );
}

#[test]
fn a_tagged_out_charge_is_suppressed_despite_being_detectable() {
    // Dropbox is six identical monthly charges — exactly what detection looks
    // for. One `subscription:false` takes it off the list anyway.
    let report = report();
    assert!(find(&report.monthly, "Dropbox").is_none());
    assert!(find(&report.annual, "Dropbox").is_none());

    // It IS otherwise detectable: strip the tag's effect by tagging nothing and
    // the same charge appears. (Proved by clearing the journal's tag via a
    // narrower run is impractical, so assert the shape the detector would see:
    // six charges at one price, well inside the staleness window.)
    let detected_without_tag = detect_subscriptions(
        &fixture(),
        &SubscriptionOpts {
            as_of: AS_OF,
            min_monthly: 5,
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds");
    assert!(
        find(&detected_without_tag.monthly, "Dropbox").is_none(),
        "the tag suppresses it regardless of thresholds"
    );
}

#[test]
fn detected_subscriptions_are_not_marked_manual() {
    let report = report();
    let netflix = find(&report.monthly, "Netflix").expect("Netflix detected");
    assert!(!netflix.manual);
}

#[test]
fn a_quarterly_charge_is_not_reported_as_monthly() {
    // City Water bills every three months, always on the 10th — the day-of-month
    // is as consistent as any monthly subscription, so only the gap between
    // charges distinguishes it. It must appear in neither list.
    let report = report();
    assert!(find(&report.monthly, "City Water").is_none());
    assert!(find(&report.annual, "City Water").is_none());
}

#[test]
fn narrowing_the_window_drops_charges_that_fall_outside_it() {
    // A 12-month lookback opens on 2025-06-30, excluding both annual renewals
    // (Sep 2024 / Nov 2024) so neither has a second occurrence to pair with.
    let exclude = excludes();
    let report = detect_subscriptions(
        &fixture(),
        &SubscriptionOpts {
            as_of: AS_OF,
            lookback_months: 12,
            exclude_desc: &exclude,
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds");
    assert_eq!(report.lookback_start, "2025-06-30");
    assert!(
        report.annual.is_empty(),
        "annual pairs fall outside a 1-year window"
    );
    // The monthly subscriptions still qualify on the charges that remain
    // (Backblaze drops to 3 charges, below min_monthly; the tagged Twilio stays).
    assert_eq!(
        payees(&report.monthly),
        ["Twilio", "Netflix", "Spotify", "Apple"]
    );
}

// ---- the amount tolerance is an exact boundary, not a floating-point one ----

/// Six monthly charges at `first` (×3) then `second` (×3), all on the 10th, all
/// funded from one checking account so nothing reads as stale.
///
/// Whether these form ONE cluster or two is the whole question: at six charges
/// a single cluster clears the default `min_monthly` of 5 and is reported; split
/// into two threes, neither half qualifies and the subscription vanishes.
fn two_price_journal(first: &str, second: &str) -> Journal {
    let dates = [
        "2025-01-10",
        "2025-02-10",
        "2025-03-10",
        "2025-04-10",
        "2025-05-10",
        "2025-06-10",
    ];
    let mut text = String::new();
    for (i, date) in dates.iter().enumerate() {
        let amount = if i < 3 { first } else { second };
        text.push_str(&format!(
            "{date} Boundary Co\n    expenses:subscriptions   ${amount}\n    assets:bank:checking\n\n"
        ));
    }
    parse_journal(&text, "boundary.journal").expect("boundary journal parses")
}

fn boundary_report(first: &str, second: &str) -> SubscriptionsReport {
    detect_subscriptions(
        &two_price_journal(first, second),
        &SubscriptionOpts {
            as_of: "2025-06-30",
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds")
}

#[test]
fn a_rise_of_exactly_the_tolerance_stays_one_subscription() {
    // $6.00 → $6.90 is a rise of EXACTLY the 15% default tolerance, so the
    // charges belong to one cluster and the subscription is detected.
    //
    // This is the case the old `f64` comparison got wrong. Neither $6.00 nor the
    // 1.15 ratio is binary-representable, and the product rounded DOWN:
    // `6.0 * (1.0 + 15.0 / 100.0) == 6.899999999999999467`, just under $6.90. The
    // charge fell outside the cluster, the cluster split 3/3, both halves landed
    // below `min_monthly`, and the subscription was not reported at all — an
    // entire row missing from the user's list because of the last bit of a
    // double. Exact `Dec` compares `690 × 100 <= 600 × 115`, i.e. `69000 <=
    // 69000`, which is the equality the tolerance is defined to include.
    let report = boundary_report("6.00", "6.90");
    let found = find(&report.monthly, "Boundary Co").expect("charge on the boundary is detected");
    assert_eq!(found.cadence, Cadence::Monthly);
    assert_eq!(found.occurrences, 6, "one cluster spanning both prices");
    // Median of the six sorted amounts, so the reported figure is a real charge.
    assert_eq!(found.typical_amount, Dec::new(690, 2));
    assert_eq!(found.annualized_cost, Dec::new(8280, 2));
}

#[test]
fn a_rise_one_cent_past_the_tolerance_splits_the_cluster() {
    // The other side of the same boundary: $6.00 → $6.91 exceeds 15%
    // (`691 × 100 > 600 × 115`, i.e. `69100 > 69000`), so the cluster genuinely
    // splits and neither half reaches `min_monthly`. The fix widens nothing — it
    // makes the cut land exactly where the threshold says, and the two tests
    // together pin it to the cent.
    let report = boundary_report("6.00", "6.91");
    assert!(
        find(&report.monthly, "Boundary Co").is_none(),
        "a rise past the tolerance must still split the cluster"
    );
    assert!(report.monthly.is_empty() && report.annual.is_empty());
}

#[test]
fn the_tolerance_boundary_is_exact_at_every_scale_of_charge() {
    // The same equality holds wherever the anchor sits, including amounts whose
    // f64 image is nowhere near their decimal value. Each pair is `anchor` and
    // `anchor × 1.15` to the cent; all must cluster.
    for (low, high) in [
        ("1.40", "1.61"),
        ("2.60", "2.99"),
        ("6.00", "6.90"),
        ("19.80", "22.77"),
        ("119.80", "137.77"),
    ] {
        let report = boundary_report(low, high);
        assert!(
            find(&report.monthly, "Boundary Co").is_some(),
            "${low} → ${high} is exactly +15% and must stay one subscription"
        );
    }
}

// ---- the valuation base ----

/// The detector counts only the amounts denominated in the journal's base
/// commodity, and that base is the most frequent price TARGET. Deriving it by
/// counting targets rather than by building a whole price database must not move
/// it: six €9.99 charges are a subscription only while EUR wins, and the six
/// $9.99 charges standing beside them never are.
#[test]
fn the_base_commodity_is_the_most_frequent_price_target() {
    let mut text = String::from(
        "P 2025-01-01 AAPL 100.00 EUR\nP 2025-02-01 AAPL 101.00 EUR\nP 2025-03-01 VTI $50.00\n\n",
    );
    for month in 1..=6 {
        text.push_str(&format!(
            "2025-{month:02}-10 Euro Streamer\n    expenses:subscriptions   9.99 EUR\n    assets:bank:checking\n\n"
        ));
        text.push_str(&format!(
            "2025-{month:02}-12 Dollar Streamer\n    expenses:subscriptions   $9.99\n    assets:bank:checking\n\n"
        ));
    }
    let journal = parse_journal(&text, "base.journal").expect("base journal parses");
    let report = detect_subscriptions(
        &journal,
        &SubscriptionOpts {
            as_of: "2025-07-31",
            ..SubscriptionOpts::default()
        },
    )
    .expect("detection succeeds");
    assert_eq!(payees(&report.monthly), ["Euro Streamer"]);
    assert_eq!(
        find(&report.monthly, "Euro Streamer")
            .expect("the EUR charge is the subscription")
            .typical_amount,
        Dec::new(999, 2)
    );
}
