//! The grouped, valued balance sheet (`plans/12-balance-sheet-redesign.md`)
//! against real journals and hledger 1.52 ground truth.
//!
//! Every expected number below was read off the `hledger` binary in the dev
//! shell — never off our own output — and the command that produced it is quoted
//! beside the assertion. Three claims are being defended:
//!
//! 1. **`A − L − E` is exact.** Not rounded, and nothing about presentation
//!    survives into it: a journal whose every posting is written to the
//!    commodity's own precision produces the empty mixed amount on every
//!    valuation basis and at every depth. `bs-unbalanced.journal` proves the
//!    line is a real detector and not a tautology, and `bs-cost-dust.journal`
//!    proves the converse — a VALID journal whose priced lots leave sub-cent
//!    residue is reported exactly and still reads `balanced`.
//! 2. **Groups are chosen by type, commodity and tree position — never by an
//!    English name.** `bs-groups.journal` has no English in it at all.
//! 3. **Totals do not move with `depth`** (RPT-1/RPT-4), because they are summed
//!    over members rather than over displayed rows.

mod common;

use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    AccountType, BalanceSheetReport, BsGroup, BsOpts, BsSectionKind, CASH_GROUP, GroupSource,
    INVESTMENTS_GROUP, MixedAmount, RETAINED_EARNINGS_GROUP, VALUATION_ADJUSTMENT_GROUP, Valuation,
    account_decls, account_groups, balance_sheet_grouped, declared_types,
};
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal_fixture(relative: &str) -> Journal {
    let path = common::fixtures_dir().join(relative);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"));
    parse_journal(&text, &path.to_string_lossy())
        .unwrap_or_else(|e| panic!("parse {relative}: {e}"))
}

fn report(
    journal: &Journal,
    as_of: &str,
    depth: Option<usize>,
    value: Valuation,
) -> BalanceSheetReport {
    let declared: BTreeMap<String, AccountType> = declared_types(&account_decls(journal));
    balance_sheet_grouped(
        &journal.transactions,
        &journal.prices,
        &BsOpts {
            as_of,
            depth,
            value,
            value_in: None,
        },
        &declared,
        &account_groups(journal),
    )
    .expect("grouped balance sheet")
}

fn usd(mantissa: i128, places: u32) -> MixedAmount {
    MixedAmount::single(Commodity("$".into()), Dec::new(mantissa, places))
}

fn commodity(symbol: &str) -> Commodity {
    Commodity(symbol.into())
}

fn group<'a>(report: &'a BalanceSheetReport, kind: BsSectionKind, name: &str) -> &'a BsGroup {
    report
        .sections
        .iter()
        .find(|section| section.kind == kind)
        .unwrap_or_else(|| panic!("section {kind:?}"))
        .groups
        .iter()
        .find(|group| group.name == name)
        .unwrap_or_else(|| panic!("group {name} in {kind:?}"))
}

/// `(name, source)` for every group of a section, in presentation order.
fn group_names(report: &BalanceSheetReport, kind: BsSectionKind) -> Vec<(String, GroupSource)> {
    report
        .sections
        .iter()
        .find(|section| section.kind == kind)
        .unwrap_or_else(|| panic!("section {kind:?}"))
        .groups
        .iter()
        .map(|group| (group.name.clone(), group.source))
        .collect()
}

/// Every valuation basis, so an invariant claimed "always" is tested that way.
const BASES: [Valuation; 3] = [Valuation::Market, Valuation::Cost, Valuation::None];

/// Depths spanning "totals only", every real level of `sample.journal`
/// (deepest: `assets:broker:taxable:aapl`, 4 segments) and deeper than anything.
const DEPTHS: [Option<usize>; 8] = [
    Some(0),
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(9),
    None,
];

// ---------------------------------------------------------------------------
// fixtures/sample.journal — the numbers in the plan's ground-truth table
// ---------------------------------------------------------------------------

/// `hledger -f fixtures/sample.journal bs -V -e 2026-07-09`
/// ```text
///  assets:property:home        $468,000.00
///  assets:vehicles:car          $20,500.00
///  Assets  $548,112.62, 5.0 GLD, -2.0 TSLA
///  liabilities:mortgage        $336,000.00
///  liabilities:cc:visa             $531.15
///  Liabilities  $336,531.15
///  Net:  $211,581.46, 5.0 GLD, -2.0 TSLA
/// ```
/// We keep the unrounded `$548,112.6150`, and it is hledger's own figure rather
/// than an un-rounding of the line above: the same command with
/// `-c '$1000.0000'` prints `$548112.6150` and `$211581.4650`.
#[test]
fn sample_at_market_matches_hledger_bs_v() {
    let journal = common::fixture_journal();
    let report = report(&journal, "2026-07-08", Some(3), Valuation::Market);

    let mut assets = usd(5_481_126_150, 4);
    assets
        .accumulate(&commodity("GLD"), Dec::new(5, 0))
        .unwrap();
    assets
        .accumulate(&commodity("TSLA"), Dec::new(-2, 0))
        .unwrap();
    assert_eq!(report.sections[0].total, assets);
    assert_eq!(report.sections[1].total, usd(33_653_115, 2));

    let mut net = usd(2_115_814_650, 4);
    net.accumulate(&commodity("GLD"), Dec::new(5, 0)).unwrap();
    net.accumulate(&commodity("TSLA"), Dec::new(-2, 0)).unwrap();
    assert_eq!(report.net_worth, net);

    assert_eq!(report.base, Some(commodity("$")));
    assert_eq!(report.as_of, "2026-07-08");
}

/// `hledger -f fixtures/sample.journal bal -V -e 2026-07-09 'type:C'`
/// ```text
///          $28,292.81  assets:bank:checking
///          $13,500.00  assets:bank:savings
///             $657.43  assets:bank:wise:eur
///           $6,609.75  assets:broker:taxable:cash
/// --------------------
///          $49,059.99
/// ```
/// — all four are declared `type: C`, and the same query at `-c '$1000.0000'`
/// prints the `$49059.9900` asserted below. The property and vehicle accounts
/// added for `plans/14-other-holdings.md` are `type: A`, so this total is exactly
/// the one it was before them.
#[test]
fn sample_cash_group_matches_hledger_type_c_query() {
    let journal = common::fixture_journal();
    let report = report(&journal, "2026-07-08", Some(3), Valuation::Market);
    let cash = group(&report, BsSectionKind::Assets, CASH_GROUP);
    assert_eq!(cash.source, GroupSource::Type);
    assert_eq!(cash.total, usd(490_599_900, 4)); // $49,059.99

    // The remaining assets show all three of the non-tag signals disagreeing
    // usefully, and one tag overriding one of them:
    //
    // - the four securities accounts hold a commodity other than the base `$`
    //   (step 4 → Investments);
    // - `assets:property:home` holds `1.0 HOME`, so step 4 would have filed the
    //   house under Investments too — its `bsgroup: Property` (step 1) is what
    //   keeps a house out of the securities bucket;
    // - `assets:vehicles:car` holds only `$` and is a non-cash `type: A`, so
    //   steps 3 and 4 both decline and it falls through to its second path
    //   segment (step 5 → "Vehicles").
    //
    // Order is `group_rank`: the built-ins in balance-sheet order first, then
    // everything else alphabetically — so Investments precedes Property and
    // Vehicles regardless of how the tag was spelled.
    assert_eq!(
        group_names(&report, BsSectionKind::Assets),
        [
            (CASH_GROUP.to_string(), GroupSource::Type),
            (INVESTMENTS_GROUP.to_string(), GroupSource::Commodity),
            ("Property".to_string(), GroupSource::Tag),
            ("Vehicles".to_string(), GroupSource::Segment),
        ]
    );

    // `hledger -f fixtures/sample.journal bal -V -e 2026-07-09 assets:property
    //  assets:vehicles` → `$468,000.00` / `$20,500.00`, total `$488,500.00`.
    assert_eq!(
        group(&report, BsSectionKind::Assets, "Property").total,
        usd(46_800_000, 2)
    );
    assert_eq!(
        group(&report, BsSectionKind::Assets, "Vehicles").total,
        usd(2_050_000, 2)
    );
}

/// `hledger -f fixtures/sample.journal bse -B -e 2026-07-09`:
/// ```text
///  Assets  $498,580.06, -933,25 EUR, 5.0 GLD
///  Liabilities  $336,531.15
///  Equity  $126,550.00, 5.0 GLD  (opening $126,550.00 + transfers 5.0 GLD)
///  Net:  $35,498.91, -933,25 EUR
/// ```
/// and `hledger … is -B` reports the SAME Net — `$35,498.91, -933,25 EUR` — which
/// is the identity the retained-earnings line is built on. The house enters at
/// its `1 HOME @ $420,000.00` COST here rather than at the 2026-06-30 price, and
/// the two `expenses:depreciation` entries are the whole of the $7,500.00 by
/// which this Net sits below the pre-`plans/14` figure.
#[test]
fn sample_at_cost_matches_hledger_bse_b() {
    let journal = common::fixture_journal();
    let report = report(&journal, "2026-07-08", Some(4), Valuation::Cost);

    let mut assets = usd(49_858_006, 2);
    assets
        .accumulate(&commodity("EUR"), Dec::new(-93_325, 2))
        .unwrap();
    assets
        .accumulate(&commodity("GLD"), Dec::new(5, 0))
        .unwrap();
    assert_eq!(report.sections[0].total, assets);
    assert_eq!(report.sections[1].total, usd(33_653_115, 2));

    // `is -B` Net, per commodity.
    let mut retained = usd(3_549_891, 2);
    retained
        .accumulate(&commodity("EUR"), Dec::new(-93_325, 2))
        .unwrap();
    assert_eq!(
        group(&report, BsSectionKind::Equity, RETAINED_EARNINGS_GROUP).total,
        retained
    );

    // Declared equity, `bse -B`'s own split.
    assert_eq!(
        group(&report, BsSectionKind::Equity, "Opening").total,
        usd(12_655_000, 2)
    );
    assert_eq!(
        group(&report, BsSectionKind::Equity, "Transfers").total,
        MixedAmount::single(commodity("GLD"), Dec::new(5, 0))
    );

    // Nothing is unbooked at cost, so there is no valuation-adjustment line.
    assert!(
        !group_names(&report, BsSectionKind::Equity)
            .iter()
            .any(|(name, _)| name == VALUATION_ADJUSTMENT_GROUP)
    );

    // The identity, arithmetic and all, exactly as hledger reports it:
    //   $498,580.06 − $336,531.15 − $126,550.00 = $35,498.91
    //   EUR: −933,25 − 0 − 0 = −933,25       GLD: 5 − 0 − 5 = 0
    // The DECLARED equity here is $126,550.00 + 5 GLD — the at-cost figure, not
    // the $127,550.00 an unvalued `bal type:E` reports. Feeding the unvalued one
    // into this line throws the check off by exactly $1,000.00 and 5 GLD.
    let declared_equity = report.sections[2]
        .groups
        .iter()
        .filter(|group| group.source != GroupSource::Computed)
        .try_fold(MixedAmount::new(), |acc, group| acc.ma_add(&group.total))
        .unwrap();
    let mut want_equity = usd(12_655_000, 2);
    want_equity
        .accumulate(&commodity("GLD"), Dec::new(5, 0))
        .unwrap();
    assert_eq!(declared_equity, want_equity);
    assert_eq!(
        report.sections[0]
            .total
            .ma_add(&report.sections[1].total.ma_neg().unwrap())
            .unwrap()
            .ma_add(&declared_equity.ma_neg().unwrap())
            .unwrap(),
        retained,
        "A − L − E(declared) == retained earnings, per commodity"
    );
    assert_eq!(report.check, MixedAmount::new());
}

/// `hledger -f fixtures/sample.journal bse -e 2026-07-09` (unvalued):
/// ```text
///  Assets  $68,902.56, 19.5000 AAPL, 566,75 EUR, 5.0 GLD, 1.0 HOME, -2.0 TSLA, 17.0 VTI
///  Equity  $127,550.00
/// ```
/// Unvalued is where the two ways a non-stock asset moves come apart: the house
/// is still the bare `1.0 HOME` it was booked as, because no `P` directive is
/// consulted, while the car is `$20,500.00` — the depreciation entries are
/// postings, so they are already in the balance on every basis.
#[test]
fn sample_unvalued_matches_hledger_bse() {
    let journal = common::fixture_journal();
    let report = report(&journal, "2026-07-08", Some(4), Valuation::None);

    let mut assets = usd(6_890_256, 2);
    for (symbol, mantissa, places) in [
        ("AAPL", 195, 1),
        ("EUR", 56_675, 2),
        ("GLD", 5, 0),
        ("HOME", 1, 0),
        ("TSLA", -2, 0),
        ("VTI", 17, 0),
    ] {
        assets
            .accumulate(&commodity(symbol), Dec::new(mantissa, places))
            .unwrap();
    }
    assert_eq!(report.sections[0].total, assets);
    assert_eq!(report.sections[2].groups.len(), 4);
    assert_eq!(
        group(&report, BsSectionKind::Equity, "Opening")
            .total
            .ma_add(&group(&report, BsSectionKind::Equity, "Transfers").total)
            .unwrap(),
        usd(12_755_000, 2),
        "declared equity as `bse` prints it"
    );
    assert_eq!(report.base, None, "nothing is valued, so there is no base");
}

/// GLD and TSLA carry no `P` directive on purpose. `hledger bs -V` leaves them
/// as share counts rather than pricing them from a stale cost annotation, and so
/// must we — with `meta.unpriced` saying which, so the UI can warn.
#[test]
fn sample_unpriced_commodities_are_kept_and_named() {
    let journal = common::fixture_journal();
    let market = report(&journal, "2026-07-08", Some(4), Valuation::Market);
    assert_eq!(
        market.meta.unpriced,
        vec![commodity("GLD"), commodity("TSLA")]
    );

    // Still on the row, exactly as `bs -V` prints them.
    let investments = group(&market, BsSectionKind::Assets, INVESTMENTS_GROUP);
    assert_eq!(
        investments.total.get(&commodity("GLD")),
        Some(Dec::new(5, 0))
    );
    assert_eq!(
        investments.total.get(&commodity("TSLA")),
        Some(Dec::new(-2, 0))
    );
    let gld = investments
        .rows
        .iter()
        .find(|row| row.account == "assets:broker:taxable:gld")
        .expect("the GLD account is a row of its own at depth 4");
    assert_eq!(
        gld.own,
        MixedAmount::single(commodity("GLD"), Dec::new(5, 0))
    );

    // Priced commodities really were valued, so this is not "nothing happened".
    assert_eq!(
        investments.total.get(&commodity("AAPL")),
        None,
        "AAPL has a P directive and must be valued into $"
    );

    // The other bases never value anything, so nothing can be unpriced there.
    for value in [Valuation::Cost, Valuation::None] {
        let unvalued = report(&journal, "2026-07-08", Some(4), value);
        assert!(unvalued.meta.unpriced.is_empty(), "{value:?}");
    }
}

// ---------------------------------------------------------------------------
// The check line
// ---------------------------------------------------------------------------

/// `bse -B` Net == `is -B` Net on `sample.journal`, so `A − L − E` must be
/// EMPTY — on every basis, at every depth, at every as-of date.
#[test]
fn check_is_empty_on_a_balanced_journal() {
    let journal = common::fixture_journal();
    for as_of in [
        "2024-06-30", // before the first transaction
        "2024-12-31",
        "2025-08-20", // the GLD equity transfer's own date
        "2026-06-30",
        "2026-07-08",
    ] {
        for value in BASES {
            for depth in DEPTHS {
                let report = report(&journal, as_of, depth, value);
                assert_eq!(
                    report.check,
                    MixedAmount::new(),
                    "check at {as_of}, {value:?}, depth {depth:?}"
                );
                assert!(
                    report.balanced,
                    "balanced at {as_of}, {value:?}, depth {depth:?}"
                );
                // Restated as the identity it stands for.
                let sections = &report.sections;
                assert_eq!(
                    sections[0].total,
                    sections[1].total.ma_add(&sections[2].total).unwrap(),
                    "A == L + E at {as_of}, {value:?}, depth {depth:?}"
                );
            }
        }
    }
}

/// … and it is not a tautology. hledger refuses this file outright:
/// `The real postings' sum should be 0 but is: $10.00`. We open it and report
/// the same `$10.00` from the other end.
///
/// The sub-cent tolerance behind `balanced` must not soften this: `$10.00` is a
/// thousand times one unit of the precision the file writes dollars at, and it
/// is a sum of WRITTEN amounts, which can never land under the threshold at all.
#[test]
fn check_reports_the_residual_of_an_unbalanced_journal() {
    let journal = journal_fixture("reports/errors/bs-unbalanced.journal");
    for value in BASES {
        for depth in DEPTHS {
            let report = report(&journal, "2026-12-31", depth, value);
            assert_eq!(report.check, usd(1000, 2), "{value:?}, depth {depth:?}");
            assert!(!report.balanced, "{value:?}, depth {depth:?}");
        }
    }
}

/// The other half of the story: a journal both tools call VALID whose at-cost
/// sum is still not zero, because `26.2690 VTI @ $289.7713` costs
/// `$7,612.00227970` and no posting can carry the surplus digits.
///
/// This reproduces the user-reported figure exactly. hledger 1.52 agrees on
/// both counts — `check` is silent, and `bal -B -c '$1000.00000000'` totals
/// `$0.00227970` — so the residue is arithmetic, not a defect in the file, and
/// `check` must keep reporting it while `balanced` stops calling it an error.
#[test]
fn sub_cent_cost_dust_is_reported_exactly_and_still_balances() {
    let journal = journal_fixture("reports/bs-cost-dust.journal");
    // The parser agrees with hledger that the file is fine: its own balance test
    // tolerates half a unit at the written precision, and this is far under.
    assert!(
        ledgeline_core::parse::check_transaction_balances(&journal)
            .expect("balance check")
            .is_empty(),
        "hledger accepts this journal, so the parser must too"
    );
    for value in BASES {
        for depth in DEPTHS {
            let report = report(&journal, "2026-12-31", depth, value);
            assert_eq!(
                report.check,
                usd(22797, 7),
                "the exact residual survives, undiminished ({value:?}, {depth:?})"
            );
            assert!(
                report.balanced,
                "$0.0022797 is under a cent, so no posting could be it ({value:?}, {depth:?})"
            );
        }
    }
}

/// **Regression, shipped bug.** The same journal, plus the one thing Patrick's
/// real book has that the tidy fixture did not: `$0.0327` of brokerage interest,
/// dollars written to the ten-thousandth.
///
/// Under the first rule — one unit of the commodity's own written precision —
/// that single line moved `p($)` from 2 to 4, dropped the threshold to
/// `$0.0001`, and made the UNCHANGED `$0.0022797` of cost dust read as an
/// imbalance. The interest entry balances exactly, so it cannot have moved the
/// residual; the tolerance must not be a function of the finest posting in the
/// book. The one-hundredth floor is what stops it.
#[test]
fn a_finely_written_dollar_posting_cannot_tighten_the_tolerance_below_a_cent() {
    let journal = journal_fixture("reports/bs-cent-floor.journal");
    // `hledger -f fixtures/reports/bs-cent-floor.journal check` exits 0: its
    // balance test is per ENTRY, and the fractional lot is still written to the
    // cent whatever the interest line does.
    assert!(
        ledgeline_core::parse::check_transaction_balances(&journal)
            .expect("balance check")
            .is_empty(),
        "hledger accepts this journal, so the parser must too"
    );
    for value in BASES {
        for depth in DEPTHS {
            let report = report(&journal, "2026-12-31", depth, value);
            assert_eq!(
                report.check,
                usd(22797, 7),
                "the interest entry balances, so the residual is untouched \
                 ({value:?}, {depth:?})"
            );
            assert!(
                report.balanced,
                "a $0.0327 posting must not make $0.0022797 an imbalance \
                 ({value:?}, {depth:?})"
            );
        }
    }
}

/// The floor's honest cost, and the thing that pays it.
///
/// A one-cent floor really does give up this report's ability to notice a
/// SUB-cent entry imbalance in a journal that writes dollars finer than cents —
/// the old precision rule's proof ("a sum of written amounts is at least one
/// written unit") no longer reaches below two places. Break the interest entry
/// by seven ten-thousandths and the balance sheet now says balanced.
///
/// It is not lost, though, and this is the load-bearing half of the safety
/// argument in [`is_balanced`]'s doc comment: the entry is caught upstream, per
/// transaction, at hledger's own tolerance. hledger 1.52 refuses the same file —
/// `The real postings' sum should be 0 but is: $0.0007` — and
/// `check_transaction_balances` reproduces it. This report is the second net for
/// that class of failure, never the only one.
#[test]
fn a_sub_cent_entry_imbalance_is_caught_by_the_parser_not_by_the_floor() {
    let text =
        std::fs::read_to_string(common::fixtures_dir().join("reports/bs-cent-floor.journal"))
            .expect("read bs-cent-floor.journal")
            .replace("$-0.0327", "$-0.0320");
    let journal = parse_journal(&text, "bs-cent-floor-broken.journal").expect("parse");

    let unbalanced =
        ledgeline_core::parse::check_transaction_balances(&journal).expect("balance check");
    assert_eq!(
        unbalanced.len(),
        1,
        "the parser judges the entry on its own, at half of $0.0001"
    );

    let report = report(&journal, "2026-12-31", None, Valuation::Cost);
    assert_eq!(
        report.check,
        usd(29797, 7),
        "$0.0022797 of dust plus the $0.0007 that was posted short"
    );
    assert!(
        report.balanced,
        "under a cent, so the balance sheet stays quiet — the diagnostic does not"
    );
}

// ---------------------------------------------------------------------------
// fixtures/reports/bs-groups.journal — grouping without a word of English
// ---------------------------------------------------------------------------

/// All five resolution steps, in precedence order, on a chart of accounts a
/// name-matching implementation cannot read.
#[test]
fn groups_resolve_by_tag_then_type_then_commodity_then_segment() {
    let journal = journal_fixture("reports/bs-groups.journal");
    let report = report(&journal, "2026-06-30", Some(3), Valuation::Market);

    assert_eq!(
        group_names(&report, BsSectionKind::Assets),
        [
            // Built-in groups first, in balance-sheet order …
            (CASH_GROUP.to_string(), GroupSource::Type),
            (INVESTMENTS_GROUP.to_string(), GroupSource::Commodity),
            // … then everything else alphabetically.
            ("Bienes inmuebles".to_string(), GroupSource::Tag),
            ("Flota".to_string(), GroupSource::Tag),
        ]
    );
    assert_eq!(
        group_names(&report, BsSectionKind::Liabilities),
        [
            // `cc` is aliased to a label; membership came from the tree.
            ("Credit cards".to_string(), GroupSource::Segment),
            ("Hipoteca".to_string(), GroupSource::Segment),
        ]
    );

    // `activo:inmueble:casa` is declared with no `bsgroup:` of its own — the tag
    // on `activo:inmueble` inherits down, exactly as `type:` does.
    //
    // `activo` is NOT a row here: it is shared with every other asset group, so
    // its rolled total inside this one would be a partial subtotal restating the
    // group header. `activo:inmueble` owns its whole subtree, so it survives.
    assert_eq!(
        group(&report, BsSectionKind::Assets, "Bienes inmuebles")
            .rows
            .iter()
            .map(|row| row.account.as_str())
            .collect::<Vec<_>>(),
        ["activo:inmueble", "activo:inmueble:casa"]
    );
}

/// No group may open with a row that merely restates its own heading: an
/// ancestor shared with another group is dropped rather than shown as a partial
/// subtotal of a subtree it does not own.
#[test]
fn groups_omit_ancestors_they_do_not_own() {
    let journal = common::fixture_journal();
    for depth in DEPTHS {
        let report = report(&journal, "2026-07-08", depth, Valuation::Market);
        for section in &report.sections {
            for group in &section.groups {
                assert!(
                    !group
                        .rows
                        .iter()
                        .any(|row| row.account == "assets" || row.account == "assets:broker"),
                    "{} / {} at depth {depth:?} kept a shared ancestor: {:?}",
                    section.title,
                    group.name,
                    group
                        .rows
                        .iter()
                        .map(|row| row.account.as_str())
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    // What survives at the default depth: the branches each group really owns.
    // `assets:broker:taxable:cash` is four segments deep, but it is one of the
    // cash group's ROOTS — the clamp may not hide a whole branch of the total.
    let report = report(&journal, "2026-07-08", Some(3), Valuation::Market);
    assert_eq!(
        group(&report, BsSectionKind::Assets, CASH_GROUP)
            .rows
            .iter()
            .map(|row| row.account.as_str())
            .collect::<Vec<_>>(),
        [
            "assets:bank",
            "assets:bank:checking",
            "assets:bank:savings",
            "assets:bank:wise",
            "assets:broker:taxable:cash",
        ]
    );
    assert_eq!(
        group(&report, BsSectionKind::Assets, CASH_GROUP)
            .total
            .get(&commodity("$")),
        Some(Dec::new(490_599_900, 4)),
        "the group total is unchanged by which rows are shown"
    );

    // Every security sits at depth 4 under a parent shared with the brokerage's
    // cash, so absolute clamping alone would expand this group to nothing at the
    // default depth while it reports a five-figure total.
    assert_eq!(
        group(&report, BsSectionKind::Assets, INVESTMENTS_GROUP)
            .rows
            .iter()
            .map(|row| row.account.as_str())
            .collect::<Vec<_>>(),
        [
            "assets:broker:taxable:aapl",
            "assets:broker:taxable:gld",
            "assets:broker:taxable:tsla",
            "assets:broker:taxable:vti",
        ]
    );

    // The converse case: a group that owns its whole branch KEEPS the ancestor,
    // because the rolled row is then the group's own total rather than a partial
    // subtotal of somebody else's subtree. Totals are hledger's:
    // `bal -V -e 2026-07-09 assets:property assets:vehicles` → `$468,000.00`
    // on `assets:property:home` and `$20,500.00` on `assets:vehicles:car`.
    for (name, branch, leaf, total) in [
        (
            "Property",
            "assets:property",
            "assets:property:home",
            usd(46_800_000, 2),
        ),
        (
            "Vehicles",
            "assets:vehicles",
            "assets:vehicles:car",
            usd(2_050_000, 2),
        ),
    ] {
        let owned = group(&report, BsSectionKind::Assets, name);
        assert_eq!(
            owned
                .rows
                .iter()
                .map(|row| row.account.as_str())
                .collect::<Vec<_>>(),
            [branch, leaf]
        );
        assert_eq!(owned.total, total);
    }
}

/// The rows a group opens with — those with no ancestor among its own rows —
/// must sum EXACTLY to its total, on every basis and at every depth. This is
/// what makes the expanded view trustworthy: money can never hide between the
/// heading and the rows beneath it.
#[test]
fn a_groups_top_rows_always_sum_to_its_total() {
    let journal = common::fixture_journal();
    for value in BASES {
        for depth in DEPTHS.iter().filter(|depth| **depth != Some(0)) {
            let report = report(&journal, "2026-07-08", *depth, value);
            for section in &report.sections {
                for group in &section.groups {
                    if group.source == GroupSource::Computed {
                        continue;
                    }
                    let accounts: Vec<&str> =
                        group.rows.iter().map(|row| row.account.as_str()).collect();
                    let top = group.rows.iter().filter(|row| {
                        !accounts.iter().any(|other| {
                            row.account.starts_with(other)
                                && row.account.len() > other.len()
                                && row.account.as_bytes()[other.len()] == b':'
                        })
                    });
                    assert_eq!(
                        top.fold(MixedAmount::new(), |acc, row| acc
                            .ma_add(&row.inclusive)
                            .unwrap()),
                        group.total,
                        "{} / {} at {value:?} depth {depth:?}: {accounts:?}",
                        section.title,
                        group.name
                    );
                }
            }
        }
    }
}

/// `hledger -f fixtures/reports/bs-groups.journal bs -V -e 2026-07-01`:
/// Assets `$172,050.00`, Liabilities `$142,000.00`, Net `$30,050.00`; and
/// `bse -B` / `is -B` both Net `$8,550.00`.
#[test]
fn groups_fixture_totals_match_hledger() {
    let journal = journal_fixture("reports/bs-groups.journal");
    let market = report(&journal, "2026-06-30", Some(3), Valuation::Market);

    assert_eq!(market.sections[0].total, usd(17_205_000, 2));
    assert_eq!(market.sections[1].total, usd(14_200_000, 2));
    assert_eq!(market.net_worth, usd(3_005_000, 2));
    assert_eq!(market.sections[2].total, usd(3_005_000, 2), "A − L == E");

    // `bal -V 'type:C'` → $4,550.00; the securities account is the difference
    // between `bs -V` ($5,500.00) and `bse -B` ($4,000.00) on that row.
    assert_eq!(
        group(&market, BsSectionKind::Assets, CASH_GROUP).total,
        usd(455_000, 2)
    );
    assert_eq!(
        group(&market, BsSectionKind::Assets, INVESTMENTS_GROUP).total,
        usd(550_000, 2)
    );
    assert_eq!(
        group(&market, BsSectionKind::Equity, RETAINED_EARNINGS_GROUP).total,
        usd(855_000, 2), // `is -B` Net
    );
    assert_eq!(
        group(&market, BsSectionKind::Equity, VALUATION_ADJUSTMENT_GROUP).total,
        usd(150_000, 2), // $5,500.00 market − $4,000.00 cost
    );

    // At cost the securities fall back to basis and the adjustment disappears.
    let cost = report(&journal, "2026-06-30", Some(3), Valuation::Cost);
    assert_eq!(cost.sections[0].total, usd(17_055_000, 2)); // `bse -B` Assets
    assert_eq!(
        group(&cost, BsSectionKind::Assets, INVESTMENTS_GROUP).total,
        usd(400_000, 2)
    );
    assert!(
        !group_names(&cost, BsSectionKind::Equity)
            .iter()
            .any(|(name, _)| name == VALUATION_ADJUSTMENT_GROUP)
    );
}

/// Valuing the report must not move an account between groups: the commodity
/// signal reads the AS-WRITTEN balance, where 100 ACC is still 100 ACC even
/// when the row is displayed as `$5,500.00`.
#[test]
fn group_membership_is_stable_across_valuation_bases() {
    let journal = journal_fixture("reports/bs-groups.journal");
    let membership = |value| {
        let report = report(&journal, "2026-06-30", Some(9), value);
        [
            BsSectionKind::Assets,
            BsSectionKind::Liabilities,
            BsSectionKind::Equity,
        ]
        .map(|kind| {
            report
                .sections
                .iter()
                .find(|section| section.kind == kind)
                .unwrap()
                .groups
                .iter()
                .filter(|group| group.source != GroupSource::Computed)
                .map(|group| {
                    (
                        group.name.clone(),
                        group
                            .rows
                            .iter()
                            .map(|row| row.account.clone())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(membership(Valuation::Market), membership(Valuation::Cost));
    assert_eq!(membership(Valuation::Market), membership(Valuation::None));
}

// ---------------------------------------------------------------------------
// Depth independence (RPT-1/RPT-4)
// ---------------------------------------------------------------------------

/// `depth` clamps the expandable ROWS and nothing else. hledger agrees: its own
/// `bs` total does not move with `--depth` either.
#[test]
fn every_total_is_depth_independent() {
    let journal = common::fixture_journal();
    for value in BASES {
        let baseline = report(&journal, "2026-07-08", Some(9), value);
        for depth in DEPTHS {
            let report = report(&journal, "2026-07-08", depth, value);
            assert_eq!(report.net_worth, baseline.net_worth, "{value:?} d{depth:?}");
            assert_eq!(report.check, baseline.check, "{value:?} d{depth:?}");
            for (section, want) in report.sections.iter().zip(&baseline.sections) {
                assert_eq!(
                    section.total, want.total,
                    "{} total at {value:?} d{depth:?}",
                    section.title
                );
                assert_eq!(
                    section
                        .groups
                        .iter()
                        .map(|group| (group.name.clone(), group.total.clone()))
                        .collect::<Vec<_>>(),
                    want.groups
                        .iter()
                        .map(|group| (group.name.clone(), group.total.clone()))
                        .collect::<Vec<_>>(),
                    "{} groups at {value:?} d{depth:?}",
                    section.title
                );
            }
        }
    }
}

/// The rows really are clamped, so the test above is not vacuous: `depth == 0`
/// shows none at all, the count grows monotonically with depth, and every row
/// beyond the clamp is one of its group's roots (which outrank it).
#[test]
fn rows_are_clamped_to_depth_except_for_group_roots() {
    let journal = common::fixture_journal();
    let mut previous = 0;
    for depth in DEPTHS {
        let report = report(&journal, "2026-07-08", depth, Valuation::Market);
        let mut rows = 0;
        for section in &report.sections {
            for group in &section.groups {
                if group.source == GroupSource::Computed {
                    assert!(group.rows.is_empty(), "synthetic lines have no accounts");
                    continue;
                }
                rows += group.rows.len();
                let accounts: Vec<&str> =
                    group.rows.iter().map(|row| row.account.as_str()).collect();
                for row in &group.rows {
                    assert_eq!(row.depth, row.account.split(':').count());
                    let is_root = !accounts.iter().any(|other| {
                        row.account.starts_with(other)
                            && row.account.len() > other.len()
                            && row.account.as_bytes()[other.len()] == b':'
                    });
                    assert!(
                        depth.is_none_or(|clamp| row.depth <= clamp) || is_root,
                        "{} is past depth {depth:?} and is not a group root",
                        row.account
                    );
                }
            }
        }
        assert_eq!(
            rows == 0,
            depth == Some(0),
            "depth {depth:?} row count {rows}"
        );
        assert!(
            rows >= previous,
            "depth {depth:?} lost rows: {rows} < {previous}"
        );
        previous = rows;
    }

    // Depth 1 is the group's own top level, not the journal's: each asset group
    // still opens with the branch it owns, even though those sit deeper than one
    // segment. The alternative — clamping absolutely — expands the securities
    // group to nothing while it reports a five-figure total.
    //
    // Property and Vehicles show the same rule from the other end. Each owns its
    // whole subtree, so its root is the two-segment branch (`assets:property`,
    // `assets:vehicles`) and not the three-segment account beneath it: past the
    // clamp, a group emits its roots and stops. `assets` itself is shared with
    // every other asset group and so is dropped from all four.
    let shallow = report(&journal, "2026-07-08", Some(1), Valuation::Market);
    let assets = &shallow.sections[0];
    assert_eq!(
        assets
            .groups
            .iter()
            .map(|group| group
                .rows
                .iter()
                .map(|row| row.account.as_str())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [
            vec!["assets:bank", "assets:broker:taxable:cash"],
            vec![
                "assets:broker:taxable:aapl",
                "assets:broker:taxable:gld",
                "assets:broker:taxable:tsla",
                "assets:broker:taxable:vti",
            ],
            vec!["assets:property"],
            vec!["assets:vehicles"],
        ]
    );
    assert_eq!(
        assets
            .groups
            .iter()
            .try_fold(MixedAmount::new(), |acc, group| acc.ma_add(&group.total))
            .unwrap(),
        assets.total
    );
    assert_eq!(shallow.check, MixedAmount::new());
}

// ---------------------------------------------------------------------------
// Declared groups
// ---------------------------------------------------------------------------

/// `account_groups` reads `bsgroup:` off the account directives and nothing
/// else — the shape of `declared_types`, without widening `AccountDecl`.
#[test]
fn account_groups_reads_only_bsgroup_tags() {
    let journal = journal_fixture("reports/bs-groups.journal");
    assert_eq!(
        account_groups(&journal),
        [
            ("activo:inmueble", "Bienes inmuebles"),
            ("activo:vehiculo:furgoneta", "Flota"),
        ]
        .into_iter()
        .map(|(account, group)| (account.to_string(), group.to_string()))
        .collect::<BTreeMap<_, _>>(),
        "accounts declared with only a `type:` must not appear"
    );

    // sample.journal declares exactly one, on a directive that also carries two
    // tags this function must ignore:
    //   account assets:property:home  ; type: A, holdings: other,
    //                                   bsgroup: Property, name: Family home
    // so it is also the check that a tag VALUE stops at the next comma rather
    // than swallowing `, name: Family home`.
    assert_eq!(
        account_groups(&common::fixture_journal()),
        [("assets:property:home", "Property")]
            .into_iter()
            .map(|(account, group)| (account.to_string(), group.to_string()))
            .collect::<BTreeMap<_, _>>(),
        "`holdings:` and `name:` are not group tags, and the value ends at the comma"
    );
}
