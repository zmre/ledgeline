//! The grouped, valued income statement (`plans/13-income-statement-redesign.md`)
//! against real journals and hledger 1.52 ground truth.
//!
//! Every expected number below was read off the `hledger` binary in the dev
//! shell — never off our own output — and the command that produced it is quoted
//! beside the assertion. Note `-e` is EXCLUSIVE in hledger while our `to` is
//! INCLUSIVE, so `to=2026-07-08` is checked with `-e 2026-07-09`.
//!
//! Four claims are being defended:
//!
//! 1. **An untagged journal gets the simple two-box statement**, and its numbers
//!    are `hledger is -V`'s. `fixtures/sample.journal` carries no `issection:`
//!    at all, so it is the whole personal-finance experience.
//! 2. **The ladder materialises line by line**, each rung only when the sections
//!    it needs exist — and a section with no members is omitted entirely rather
//!    than printed empty. `fixtures/reports/is-sections.journal` is the tagged
//!    business book that turns every rung on.
//! 3. **The prior column is a real join.** Sections, groups and names are
//!    resolved over the UNION of both windows, so a line present in only one
//!    period appears with a zero on the other side, and each window is valued at
//!    its OWN end (which is what `hledger is -V` over that range reports).
//! 4. **Totals are summed over MEMBERS** (RPT-1/RPT-4) and membership is decided
//!    before valuation (RPT-2).

mod common;

use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    Amounts, DateRange, GroupSource, IncomeStatementReport, IsGroup, IsOpts, IsSection,
    IsSectionKind, IsSubtotalKind, MixedAmount, ReportError, Valuation, account_decls,
    account_sections, account_sections_from, declared_groups, declared_types,
    income_statement_grouped, parse_is_section_tag,
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

fn report(journal: &Journal, from: &str, to: &str, value: Valuation, compare: bool) -> Report {
    Report(
        income_statement_grouped(
            &journal.transactions,
            &journal.prices,
            &IsOpts {
                from,
                to,
                value,
                value_in: None,
                compare,
            },
            &declared_types(&account_decls(journal)),
            &account_sections(journal).expect("the fixtures declare only valid `issection:` codes"),
            &declared_groups(journal, ledgeline_core::reports::IS_GROUP_TAG),
        )
        .expect("grouped income statement"),
    )
}

/// A report plus the accessors every assertion below reaches for.
struct Report(IncomeStatementReport);

impl std::ops::Deref for Report {
    type Target = IncomeStatementReport;
    fn deref(&self) -> &IncomeStatementReport {
        &self.0
    }
}

impl Report {
    fn section(&self, kind: IsSectionKind) -> &IsSection {
        self.0
            .sections
            .iter()
            .find(|section| section.kind == kind)
            .unwrap_or_else(|| panic!("section {kind:?} in {:?}", self.kinds()))
    }

    fn group(&self, kind: IsSectionKind, name: &str) -> &IsGroup {
        self.section(kind)
            .groups
            .iter()
            .find(|group| group.name == name)
            .unwrap_or_else(|| panic!("group {name} in {kind:?}"))
    }

    /// The boxes that rendered, in presentation order.
    fn kinds(&self) -> Vec<IsSectionKind> {
        self.0.sections.iter().map(|section| section.kind).collect()
    }

    /// `(title, [subtotal labels])` per box, in presentation order — the ladder
    /// as it appears on screen.
    fn ladder(&self) -> Vec<(String, Vec<String>)> {
        self.0
            .sections
            .iter()
            .map(|section| {
                (
                    section.title.clone(),
                    section
                        .trailing
                        .iter()
                        .map(|subtotal| subtotal.label.clone())
                        .collect(),
                )
            })
            .collect()
    }

    fn subtotal(&self, kind: IsSubtotalKind) -> &Amounts {
        self.0
            .sections
            .iter()
            .flat_map(|section| &section.trailing)
            .find(|subtotal| subtotal.kind == kind)
            .map(|subtotal| &subtotal.total)
            .unwrap_or_else(|| panic!("subtotal {kind:?}"))
    }

    /// `(name, source)` per group of a box, in presentation order.
    fn group_names(&self, kind: IsSectionKind) -> Vec<(String, GroupSource)> {
        self.section(kind)
            .groups
            .iter()
            .map(|group| (group.name.clone(), group.source))
            .collect()
    }
}

/// `$x.yz` from hledger's own printed figure.
fn usd(cents: i128) -> MixedAmount {
    MixedAmount::single(Commodity("$".into()), Dec::new(cents, 2))
}

fn commodity(symbol: &str) -> Commodity {
    Commodity(symbol.into())
}

/// One mixed amount canonicalized: trailing zeros stripped, zero commodities
/// dropped.
///
/// Valuation keeps the UNROUNDED scale — `210,00 EUR × $1.16` is `$243.6000`,
/// four places — while hledger prints two. The VALUE has to match hledger
/// exactly, and every figure asserted below does; the trailing zeros behind it
/// are an artefact of exact-decimal multiplication and asserting them would pin
/// an internal representation rather than an answer. (Where an exact value is
/// genuinely finer than two places, this comparison still fails — it strips
/// zeros, it does not round.)
fn canon(ma: &MixedAmount) -> BTreeMap<String, (i128, u32)> {
    ma.iter()
        .filter(|(_, dec)| !dec.is_zero())
        .map(|(symbol, dec)| {
            let (mut mantissa, mut places) = (dec.mantissa, dec.places);
            while places > 0 && mantissa % 10 == 0 {
                mantissa /= 10;
                places -= 1;
            }
            (symbol.0.clone(), (mantissa, places))
        })
        .collect()
}

#[track_caller]
fn assert_money(actual: &MixedAmount, want: &MixedAmount, what: &str) {
    assert_eq!(canon(actual), canon(want), "{what}");
}

/// Assert both columns of an [`Amounts`] at once, in cents.
#[track_caller]
fn assert_amounts(actual: &Amounts, current: i128, prior: Option<i128>, what: &str) {
    assert_money(&actual.current, &usd(current), &format!("{what} (current)"));
    match (actual.prior.as_ref(), prior) {
        (Some(actual), Some(want)) => assert_money(actual, &usd(want), &format!("{what} (prior)")),
        (None, None) => {}
        (actual, want) => panic!("{what}: prior shape mismatch — {actual:?} vs {want:?}"),
    }
}

/// Every valuation basis, so an invariant claimed "always" is tested that way.
const BASES: [Valuation; 3] = [Valuation::Market, Valuation::Cost, Valuation::None];

// ===========================================================================
// fixtures/sample.journal — untagged, so the SIMPLE two-box statement
// ===========================================================================

/// `hledger -f fixtures/sample.journal is -V -b 2026-01-01 -e 2026-07-09 --depth 2`
/// ```text
///  Revenues            income:salary $33,960.00, income:dividends $50.00
///                                                          $34,010.00
///  Expenses            depreciation $3,500.00, food $1,654.38,
///                      housing $13,125.00, taxes $8,760.00, transport $186.54,
///                      travel $656.40, unknown $75.00, utilities $669.16
///                                                          $28,626.48
///  Net:                                                     $5,383.52
/// ```
/// `--depth 2` is what the shared-prefix rule reproduces on this chart: every
/// member of each section shares one root segment and the shortest has two, so
/// the group segment is the second.
///
/// `expenses:depreciation` is the `plans/14-other-holdings.md` account, and the
/// $3,500.00 is the 2026-06-30 vehicle write-down — the one way a dollar-booked
/// asset can change value, and the only one of the two new asset accounts that
/// touches this statement at all. The house is revalued by `P` directives, which
/// are a balance-sheet event and post nothing.
#[test]
fn untagged_journal_is_two_boxes_and_matches_hledger_is_v() {
    let journal = common::fixture_journal();
    let report = report(
        &journal,
        "2026-01-01",
        "2026-07-08",
        Valuation::Market,
        false,
    );

    assert_eq!(
        report.kinds(),
        [IsSectionKind::Revenue, IsSectionKind::Opex],
        "an untagged journal asks for no ladder at all"
    );
    assert!(!report.multi_step);
    assert_eq!(
        report.ladder(),
        [
            ("Revenue".to_string(), vec![]),
            // "Expenses", not "Operating expenses": there is nothing to be
            // operating AS DISTINCT FROM.
            ("Expenses".to_string(), vec![]),
        ]
    );

    assert_amounts(
        &report.section(IsSectionKind::Revenue).total,
        3_401_000,
        None,
        "Revenue",
    );
    assert_amounts(
        &report.section(IsSectionKind::Opex).total,
        2_862_648,
        None,
        "Expenses",
    );
    assert_amounts(&report.net_income, 538_352, None, "Net income");

    assert_eq!(report.from, "2026-01-01");
    assert_eq!(report.to, "2026-07-08");
    assert_eq!(
        report.prior, None,
        "`compare: false` reports no prior window"
    );
    assert_eq!(report.base, Some(commodity("$")));
    assert!(report.meta.unpriced.is_empty());
}

/// The same run's per-group figures, which are `hledger is -V --depth 2`'s
/// per-account rows: Salary `$33,960.00`, Dividends `$50.00`; Depreciation
/// `$3,500.00`, Food `$1,654.38`, Housing `$13,125.00`, Taxes `$8,760.00`,
/// Transport `$186.54`, Travel `$656.40`, Unknown `$75.00`, Utilities `$669.16`.
///
/// Travel is the one that proves the valuation ran: `210,00 EUR` of Munich
/// lodging at the 2026-06-30 rate of `$1.16` is `$243.60`, and `$412.80` of
/// flights makes `$656.40`.
///
/// Depreciation is the one that proves the fallback is a rule and not a table:
/// nothing about `expenses:depreciation` was declared beyond `type: X`, and it
/// takes its line from the same second-segment rule as every neighbour — landing
/// first because the list is alphabetical, which is hledger's own row order too.
#[test]
fn untagged_groups_are_the_humanized_second_segment() {
    let journal = common::fixture_journal();
    let report = report(
        &journal,
        "2026-01-01",
        "2026-07-08",
        Valuation::Market,
        false,
    );

    assert_eq!(
        report.group_names(IsSectionKind::Revenue),
        [
            ("Dividends".to_string(), GroupSource::Segment),
            ("Salary".to_string(), GroupSource::Segment),
        ]
    );
    assert_eq!(
        report
            .group_names(IsSectionKind::Opex)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        [
            "Depreciation",
            "Food",
            "Housing",
            "Taxes",
            "Transport",
            "Travel",
            "Unknown",
            "Utilities",
        ]
    );

    for (kind, name, cents) in [
        (IsSectionKind::Revenue, "Salary", 3_396_000),
        (IsSectionKind::Revenue, "Dividends", 5_000),
        (IsSectionKind::Opex, "Depreciation", 350_000),
        (IsSectionKind::Opex, "Food", 165_438),
        (IsSectionKind::Opex, "Housing", 1_312_500),
        (IsSectionKind::Opex, "Taxes", 876_000),
        (IsSectionKind::Opex, "Transport", 18_654),
        (IsSectionKind::Opex, "Travel", 65_640),
        (IsSectionKind::Opex, "Unknown", 7_500),
        (IsSectionKind::Opex, "Utilities", 66_916),
    ] {
        assert_amounts(&report.group(kind, name).total, cents, None, name);
    }

    // The rows behind a group are its accounts at full depth — there is no depth
    // control on this report, so a group expands to all of itself. Only the two
    // with postings IN THE WINDOW appear: `expenses:travel:activities` is the
    // Berlin museum, which was 2025.
    assert_eq!(
        report
            .group(IsSectionKind::Opex, "Travel")
            .rows
            .iter()
            .map(|row| (row.account.as_str(), row.depth))
            .collect::<Vec<_>>(),
        [
            ("expenses:travel:flights", 3),
            ("expenses:travel:lodging", 3),
        ]
    );

    // And a group whose segment IS its only account: `expenses:depreciation` has
    // exactly two segments, so the group name and the leaf are the same word. The
    // line still expands to a real row rather than to nothing.
    assert_eq!(
        report
            .group(IsSectionKind::Opex, "Depreciation")
            .rows
            .iter()
            .map(|row| (row.account.as_str(), row.depth))
            .collect::<Vec<_>>(),
        [("expenses:depreciation", 2)]
    );
}

/// `hledger -f fixtures/sample.journal is -V -b 2024-07-01 -e 2026-07-09 --depth 2`
/// ```text
///  Revenues  $132,851.25   Expenses  $98,434.91   Net:  $34,416.34
/// ```
/// (`expenses:depreciation $7,500.00` — both write-downs — is the whole of the
/// difference from the pre-`plans/14` Expenses figure.)
///
/// Note the valued net income is NOT the balance sheet's at-cost retained
/// earnings (`$35,498.91`); the difference is exactly what the Valuation
/// adjustment line absorbs there.
#[test]
fn untagged_journal_over_the_whole_history() {
    let journal = common::fixture_journal();
    let report = report(
        &journal,
        "2024-07-01",
        "2026-07-08",
        Valuation::Market,
        false,
    );
    assert_amounts(
        &report.section(IsSectionKind::Revenue).total,
        13_285_125,
        None,
        "Revenue",
    );
    assert_amounts(
        &report.section(IsSectionKind::Opex).total,
        9_843_491,
        None,
        "Expenses",
    );
    assert_amounts(&report.net_income, 3_441_634, None, "Net income");
}

// ===========================================================================
// The prior column
// ===========================================================================

/// `prior_to = from − 1 day`, `prior_from = prior_to − (to − from)`, and each
/// window valued at its OWN end.
///
/// `hledger -f fixtures/sample.journal is -V -b 2025-06-26 -e 2026-01-01 --depth 2`
/// ```text
///  Revenues  $39,397.50   Expenses  $28,516.71   Net:  $10,880.79
/// ```
/// (2026-01-01 .. 2026-07-08 is 188 days, so the prior window is the 188 days
/// ending 2025-12-31 — which is 2025-06-26.)
///
/// The two windows land one depreciation entry each — 2025-06-30's `$4,000.00`
/// in the prior, 2026-06-30's `$3,500.00` in the current — which is a useful
/// accident: an off-by-one in the boundary arithmetic would move a four-figure
/// sum across the column, not a rounding artefact.
#[test]
fn the_prior_window_is_the_preceding_equal_length_range() {
    let journal = common::fixture_journal();
    let report = report(
        &journal,
        "2026-01-01",
        "2026-07-08",
        Valuation::Market,
        true,
    );

    assert_eq!(
        report.prior,
        Some(DateRange {
            from: "2025-06-26".to_string(),
            to: "2025-12-31".to_string(),
        })
    );
    assert_amounts(
        &report.section(IsSectionKind::Revenue).total,
        3_401_000,
        Some(3_939_750),
        "Revenue",
    );
    assert_amounts(
        &report.section(IsSectionKind::Opex).total,
        2_862_648,
        Some(2_851_671),
        "Expenses",
    );
    assert_amounts(&report.net_income, 538_352, Some(1_088_079), "Net income");
}

/// A full calendar year yields the prior calendar year — with ONE documented
/// exception the plan's example does not reach.
///
/// The rule is pure arithmetic, deliberately: `prior_to = from − 1 day`,
/// `prior_from = prior_to − (to − from)`. When the preceding year has the same
/// length that lands exactly on Jan 1, which is the common case and the one the
/// plan quotes (`2026 → 2025`). When the preceding year is a LEAP year it does
/// not: 2025 is 365 days and 2024 is 366, so `2025 → 2024-01-02..2024-12-31`,
/// an equal-length window that is one day short of the calendar year.
///
/// That is the honest trade the plan chose ("no calendar special-casing"): the
/// comparison is apples-to-apples in DURATION, its dates are reported in
/// `prior` for the column header to show, and nothing silently pretends a
/// 365-day window is a 366-day year. Pinned here so the behaviour is a decision
/// rather than a surprise.
#[test]
fn a_calendar_year_compares_against_the_calendar_year_before() {
    let journal = common::fixture_journal();
    for (from, to, want_from, want_to) in [
        ("2026-01-01", "2026-12-31", "2025-01-01", "2025-12-31"),
        // The leap-year exception: 2024 has 366 days, so an equal-length window
        // ending 2024-12-31 starts on Jan 2.
        ("2025-01-01", "2025-12-31", "2024-01-02", "2024-12-31"),
        // A single day compares against the day before.
        ("2026-03-01", "2026-03-01", "2026-02-28", "2026-02-28"),
        // And a month against the equal-length span ending the day before it.
        ("2026-03-01", "2026-03-31", "2026-01-29", "2026-02-28"),
    ] {
        let report = report(&journal, from, to, Valuation::Market, true);
        assert_eq!(
            report.prior,
            Some(DateRange {
                from: want_from.to_string(),
                to: want_to.to_string(),
            }),
            "prior of {from}..{to}"
        );
    }
}

/// Each window is valued at its own end, matching the `hledger is -V` you would
/// have run over that range at the time.
///
/// EUR travel is the probe — the only group whose figure a rate can move:
/// ```text
/// $ hledger -f fixtures/sample.journal bal -V -b 2026-01-01 -e 2027-01-01 expenses:travel
///     $243.60  expenses:travel:lodging      (210,00 EUR at the 2026-06-30 $1.16)
///     $412.80  expenses:travel:flights
///     $656.40
/// $ hledger -f fixtures/sample.journal bal -V -b 2025-01-01 -e 2026-01-01 expenses:travel
///     $709.50  expenses:travel:lodging      (645,00 EUR at the 2025-12-31 $1.10)
///      $39.60  expenses:travel:activities   ( 36,00 EUR at the same rate)
///     $749.10
/// ```
#[test]
fn each_window_is_valued_at_its_own_end() {
    let journal = common::fixture_journal();
    // 2026 reported, 2025 prior: the prior column values Berlin at 2025-12-31.
    let compared = report(
        &journal,
        "2026-01-01",
        "2026-12-31",
        Valuation::Market,
        true,
    );
    assert_amounts(
        &compared.group(IsSectionKind::Opex, "Travel").total,
        65_640,       // 2026, valued at 2026-06-30's $1.16
        Some(74_910), // 2025, valued at 2025-12-31's $1.10
        "Travel",
    );

    // Asking for 2025 directly must give that same prior figure — which is what
    // "the prior column agrees with the report you actually ran" means.
    let standalone = report(
        &journal,
        "2025-01-01",
        "2025-12-31",
        Valuation::Market,
        false,
    );
    assert_amounts(
        &standalone.group(IsSectionKind::Opex, "Travel").total,
        74_910,
        None,
        "Travel, reported directly",
    );
}

/// `compare: false` leaves `prior` ABSENT everywhere, not zero — a report with
/// no comparison must not claim the prior period was empty.
#[test]
fn without_compare_no_figure_carries_a_prior() {
    let journal = common::fixture_journal();
    let report = report(
        &journal,
        "2026-01-01",
        "2026-07-08",
        Valuation::Market,
        false,
    );
    assert!(report.net_income.prior.is_none());
    for section in &report.sections {
        assert!(section.total.prior.is_none(), "{}", section.title);
        for group in &section.groups {
            assert!(group.total.prior.is_none(), "{}", group.name);
            for row in &group.rows {
                assert!(row.amounts.prior.is_none(), "{}", row.account);
            }
        }
    }
}

// ===========================================================================
// fixtures/reports/is-sections.journal — the tagged book, all seven boxes
// ===========================================================================

fn sections_journal() -> Journal {
    journal_fixture("reports/is-sections.journal")
}

/// The whole ladder, in order, with every guard satisfied.
///
/// EBITDA sits ABOVE the D&A box and Operating income below it, so each subtotal
/// is a running total of everything printed above it and no line is ever the sum
/// of things both above and below.
#[test]
fn the_full_ladder_renders_in_order_with_every_rung() {
    let journal = sections_journal();
    let report = report(&journal, "2026-01-01", "2026-12-31", Valuation::None, false);

    assert!(report.multi_step);
    assert_eq!(
        report.ladder(),
        [
            ("Revenue".to_string(), vec![]),
            ("Cost of revenue".to_string(), vec!["Gross profit".into()]),
            // Retitled, because the ladder materialised. Same section, same
            // accounts — only the label moved.
            ("Operating expenses".to_string(), vec!["EBITDA".into()]),
            (
                "Depreciation & amortization".to_string(),
                vec!["Operating income".into()],
            ),
            ("Other income & expense".to_string(), vec![]),
            ("Interest".to_string(), vec!["Income before taxes".into()]),
            ("Income taxes".to_string(), vec![]),
        ]
    );
    assert_eq!(
        report.kinds(),
        [
            IsSectionKind::Revenue,
            IsSectionKind::Cogs,
            IsSectionKind::Opex,
            IsSectionKind::Depreciation,
            IsSectionKind::Other,
            IsSectionKind::Interest,
            IsSectionKind::Tax,
        ]
    );
}

/// Every box and every rung, in dollars.
///
/// hledger 1.52 over `fixtures/reports/is-sections.journal`, 2026:
/// ```text
/// $ hledger … bal -b 2026-01-01 -e 2027-01-01 revenue --depth 0            $-150,000.00
/// $ hledger … bal … cogs --depth 0                                          $22,500.00
/// $ hledger … bal … 'acct:^expenses:(salaries|marketing|rent)' --depth 0    $101,000.00
/// $ hledger … bal … expenses:depreciation --depth 0                          $6,000.00
/// $ hledger … bal … 'acct:^(income:grants|expenses:lawsuit)' --depth 0        $3,000.00
/// $ hledger … bal … expenses:interest --depth 0                              $3,000.00
/// $ hledger … bal … expenses:taxes --depth 0                                 $6,200.00
/// $ hledger … is -b 2026-01-01 -e 2027-01-01              Net:               $8,300.00
/// ```
#[test]
fn every_section_and_subtotal_matches_hledger() {
    let journal = sections_journal();
    let report = report(&journal, "2026-01-01", "2026-12-31", Valuation::None, false);

    for (kind, cents, what) in [
        (IsSectionKind::Revenue, 15_000_000, "Revenue"),
        (IsSectionKind::Cogs, 2_250_000, "Cost of revenue"),
        (IsSectionKind::Opex, 10_100_000, "Operating expenses"),
        (IsSectionKind::Depreciation, 600_000, "D&A"),
        // The mixed box prints SIGNED: $5,000 of grants against an $8,000
        // settlement is a $3,000 drag on income, and a statement says so with
        // parentheses rather than by hiding the sign.
        (IsSectionKind::Other, -300_000, "Other income & expense"),
        (IsSectionKind::Interest, 300_000, "Interest"),
        (IsSectionKind::Tax, 620_000, "Income taxes"),
    ] {
        assert_amounts(&report.section(kind).total, cents, None, what);
    }

    // The ladder, each rung a running total of the boxes above it.
    for (kind, cents, what) in [
        (IsSubtotalKind::GrossProfit, 12_750_000, "Gross profit"),
        (IsSubtotalKind::Ebitda, 2_650_000, "EBITDA"),
        (
            IsSubtotalKind::OperatingIncome,
            2_050_000,
            "Operating income",
        ),
        (
            IsSubtotalKind::PretaxIncome,
            1_450_000,
            "Income before taxes",
        ),
    ] {
        assert_amounts(report.subtotal(kind), cents, None, what);
    }
    assert_amounts(&report.net_income, 830_000, None, "Net income");
}

/// `isgroup:` names a line and can merge accounts that share no ancestor at all;
/// the untagged remainder falls back to the shared-prefix segment.
///
/// `$ hledger … bal -b 2026-01-01 -e 2027-01-01 \
///        'acct:^(expenses:marketing:ads|expenses:salaries:sales)$' --depth 0` → `$32,000.00`
#[test]
fn isgroup_merges_unrelated_accounts_onto_one_line() {
    let journal = sections_journal();
    let report = report(&journal, "2026-01-01", "2026-12-31", Valuation::None, false);

    assert_eq!(
        report.group_names(IsSectionKind::Opex),
        [
            ("Growth".to_string(), GroupSource::Tag),
            ("Rent".to_string(), GroupSource::Segment),
            ("Salaries".to_string(), GroupSource::Segment),
        ]
    );
    let growth = report.group(IsSectionKind::Opex, "Growth");
    assert_eq!(
        growth
            .rows
            .iter()
            .map(|row| row.account.as_str())
            .collect::<Vec<_>>(),
        ["expenses:marketing:ads", "expenses:salaries:sales"],
        "two accounts with no common ancestor, on one line"
    );
    assert_amounts(&growth.total, 3_200_000, None, "Growth");

    // A tag on the account itself, and a section tag inherited from the root.
    assert_eq!(
        report.group_names(IsSectionKind::Cogs),
        [
            ("Cloud hosting".to_string(), GroupSource::Tag),
            ("Payment processing".to_string(), GroupSource::Tag),
        ]
    );

    // The `other` box holds `income:grants` and `expenses:lawsuit`, which share
    // no leading segment at all — so the fallback index is 0 and the groups are
    // the ROOTS. Third row of the plan's table, and the reason for the cap.
    assert_eq!(
        report.group_names(IsSectionKind::Other),
        [
            ("Expenses".to_string(), GroupSource::Segment),
            ("Income".to_string(), GroupSource::Segment),
        ]
    );
}

/// The union merge: a line in only ONE period still appears, with a zero on the
/// other side. Both directions are exercised, because dropping either would be
/// silent.
///
/// ```text
/// $ hledger … bal -b 2026-01-01 -e 2027-01-01 expenses:lawsuit --depth 0    $8,000.00
/// $ hledger … bal -b 2025-01-01 -e 2026-01-01 expenses:lawsuit --depth 0            0
/// $ hledger … bal -b 2025-01-01 -e 2026-01-01 expenses:marketing:events     $4,000.00
/// $ hledger … bal -b 2026-01-01 -e 2027-01-01 expenses:marketing:events             0
/// ```
#[test]
fn a_line_present_in_only_one_period_reads_zero_on_the_other_side() {
    let journal = sections_journal();
    let report = report(&journal, "2026-01-01", "2026-12-31", Valuation::None, true);
    assert_eq!(
        report.prior,
        Some(DateRange {
            from: "2025-01-01".to_string(),
            to: "2025-12-31".to_string(),
        })
    );

    // CURRENT only: the settlement did not exist in 2025.
    assert_amounts(
        &report.group(IsSectionKind::Other, "Expenses").total,
        -800_000,
        Some(0),
        "Other / Expenses",
    );
    // PRIOR only: the conference booth did not recur in 2026. Without the union
    // this whole line — and $4,000 of the prior column's operating expenses —
    // would simply not be on the page.
    let marketing = report.group(IsSectionKind::Opex, "Marketing");
    assert_amounts(&marketing.total, 0, Some(400_000), "Opex / Marketing");
    assert_eq!(
        marketing
            .rows
            .iter()
            .map(|row| row.account.as_str())
            .collect::<Vec<_>>(),
        ["expenses:marketing:events"]
    );

    // And the prior column's boxes still add up to the prior net income, which
    // is the thing a dropped line would quietly break.
    // `$ hledger … is -b 2025-01-01 -e 2026-01-01`  Net: $-4,300.00
    assert_amounts(&report.net_income, 830_000, Some(-430_000), "Net income");
}

/// Every box and rung in the prior column, from `hledger` over 2025:
/// ```text
/// revenue $-110,000.00   cogs $18,300.00   opex $86,000.00   depreciation $6,000.00
/// other $-2,000.00       interest $3,500.00   taxes $2,500.00   Net: $-4,300.00
/// ```
/// Note `other` is POSITIVE here — 2025 held only the grant — so the same box
/// changes sign between the columns, which is exactly what a mixed section does
/// and why it is not presented as a magnitude.
#[test]
fn the_prior_column_carries_the_whole_ladder() {
    let journal = sections_journal();
    let report = report(&journal, "2026-01-01", "2026-12-31", Valuation::None, true);

    for (kind, current, prior, what) in [
        (IsSectionKind::Revenue, 15_000_000, 11_000_000, "Revenue"),
        (IsSectionKind::Cogs, 2_250_000, 1_830_000, "Cost of revenue"),
        (
            IsSectionKind::Opex,
            10_100_000,
            8_600_000,
            "Operating expenses",
        ),
        (IsSectionKind::Depreciation, 600_000, 600_000, "D&A"),
        (IsSectionKind::Other, -300_000, 200_000, "Other"),
        (IsSectionKind::Interest, 300_000, 350_000, "Interest"),
        (IsSectionKind::Tax, 620_000, 250_000, "Income taxes"),
    ] {
        assert_amounts(&report.section(kind).total, current, Some(prior), what);
    }
    for (kind, current, prior, what) in [
        (
            IsSubtotalKind::GrossProfit,
            12_750_000,
            9_170_000,
            "Gross profit",
        ),
        (IsSubtotalKind::Ebitda, 2_650_000, 570_000, "EBITDA"),
        (
            IsSubtotalKind::OperatingIncome,
            2_050_000,
            -30_000,
            "Operating income",
        ),
        (
            IsSubtotalKind::PretaxIncome,
            1_450_000,
            -180_000,
            "Income before taxes",
        ),
    ] {
        assert_amounts(report.subtotal(kind), current, Some(prior), what);
    }
}

// ===========================================================================
// The ladder's guards — each rung appears only when it is asked for
// ===========================================================================

/// Strip `issection:` tags from the fixture, one code at a time, so a box
/// disappears and the rungs that depend on it disappear with it.
fn without_section(code: &str) -> Journal {
    let text = std::fs::read_to_string(common::fixtures_dir().join("reports/is-sections.journal"))
        .expect("read is-sections.journal")
        .replace(&format!(", issection: {code}"), "");
    parse_journal(&text, "is-sections-trimmed.journal").expect("parse")
}

/// Without a D&A box, EBITDA is suppressed — it would be numerically identical
/// to Operating income, which is the duplicate-total complaint this redesign
/// exists to fix — and Operating income falls to the box above.
#[test]
fn ebitda_is_suppressed_without_a_depreciation_box() {
    // `expenses:depreciation` loses its tag and falls back to its `type: X`,
    // i.e. into operating expenses. Opex therefore grows by exactly $6,000.
    let report = report(
        &without_section("depreciation"),
        "2026-01-01",
        "2026-12-31",
        Valuation::None,
        false,
    );
    assert!(!report.kinds().contains(&IsSectionKind::Depreciation));
    assert_eq!(
        report.ladder(),
        [
            ("Revenue".to_string(), vec![]),
            ("Cost of revenue".to_string(), vec!["Gross profit".into()]),
            // No EBITDA. Operating income has moved up under the box it now
            // follows, rather than floating free of any box.
            (
                "Operating expenses".to_string(),
                vec!["Operating income".into()]
            ),
            ("Other income & expense".to_string(), vec![]),
            ("Interest".to_string(), vec!["Income before taxes".into()]),
            ("Income taxes".to_string(), vec![]),
        ]
    );
    assert_amounts(
        &report.section(IsSectionKind::Opex).total,
        10_700_000, // $101,000 + the $6,000 that is no longer split out
        None,
        "Operating expenses",
    );
    assert_amounts(
        report.subtotal(IsSubtotalKind::OperatingIncome),
        2_050_000,
        None,
        "Operating income is unchanged — only where it PRINTS moved",
    );
    assert_amounts(&report.net_income, 830_000, None, "Net income");
}

/// Every other guard, one at a time. Removing a code both drops its box and
/// drops the subtotal that box was the reason for.
#[test]
fn each_rung_is_guarded_by_the_box_that_justifies_it() {
    for (code, gone, missing_label) in [
        ("cogs", IsSectionKind::Cogs, "Gross profit"),
        ("tax", IsSectionKind::Tax, "Income before taxes"),
    ] {
        let report = report(
            &without_section(code),
            "2026-01-01",
            "2026-12-31",
            Valuation::None,
            false,
        );
        assert!(!report.kinds().contains(&gone), "{code} box still present");
        assert!(
            !report
                .ladder()
                .iter()
                .any(|(_, labels)| labels.iter().any(|label| label == missing_label)),
            "{missing_label} survived the removal of {code}"
        );
        // Nothing moved: the accounts fell back to operating expenses and net
        // income is untouched.
        assert_amounts(&report.net_income, 830_000, None, "Net income");
    }
}

/// `interest` gone but `tax` present: "Income before taxes" has no box of its
/// own to hang from and falls to the previous one rather than vanishing.
#[test]
fn a_subtotal_falls_to_the_previous_box_when_its_own_is_omitted() {
    let report = report(
        &without_section("interest"),
        "2026-01-01",
        "2026-12-31",
        Valuation::None,
        false,
    );
    assert!(!report.kinds().contains(&IsSectionKind::Interest));
    assert_eq!(
        report
            .section(IsSectionKind::Other)
            .trailing
            .iter()
            .map(|subtotal| subtotal.label.as_str())
            .collect::<Vec<_>>(),
        ["Income before taxes"],
        "the subtotal attaches to the last box PRINTED"
    );
    assert_amounts(&report.net_income, 830_000, None, "Net income");
}

/// A window in which a tagged section has no activity omits that box entirely —
/// no empty headings — and the statement reads simple again if what is left is
/// only revenue and expenses.
#[test]
fn a_window_with_no_activity_in_a_box_omits_it() {
    let journal = sections_journal();
    // 2026-03-01..2026-05-31: subscriptions, services, hosting and processing
    // fees only. Verified: `hledger … is -b 2026-03-01 -e 2026-06-01` shows
    // exactly those four accounts.
    let report = report(&journal, "2026-03-01", "2026-05-31", Valuation::None, false);
    assert_eq!(
        report.kinds(),
        [IsSectionKind::Revenue, IsSectionKind::Cogs]
    );
    assert!(report.multi_step, "a cogs box is already a ladder");
    assert_eq!(
        report.ladder(),
        [
            ("Revenue".to_string(), vec![]),
            (
                "Cost of revenue".to_string(),
                // Gross profit fires; so does Operating income, since this is a
                // multi-step statement — and with no Opex box it correctly
                // equals gross profit.
                vec!["Gross profit".into(), "Operating income".into()]
            ),
        ]
    );
    assert_amounts(&report.net_income, 12_750_000, None, "Net income");

    // An empty window is an empty statement, not a page of empty boxes.
    let quiet = report_of(&journal, "2026-01-01", "2026-01-31");
    assert!(quiet.sections.is_empty());
    assert!(!quiet.multi_step);
    assert_amounts(&quiet.net_income, 0, None, "Net income");
}

fn report_of(journal: &Journal, from: &str, to: &str) -> Report {
    report(journal, from, to, Valuation::None, false)
}

// ===========================================================================
// The shared-prefix rule, in isolation
// ===========================================================================

/// The four rows of the plan's table, each driven through a real journal.
///
/// The rule: let `common` be the longest leading segment run every member of the
/// section shares and `min_segs` the fewest segments any member has; the group
/// is the segment at `min(common, min_segs − 1)`. The cap is what makes it
/// total — a direct posting to a section root still gets a line rather than an
/// empty name.
#[test]
fn the_untagged_group_falls_back_to_the_first_unshared_segment() {
    for (accounts, want) in [
        // shared `income`, shortest has 2 segments → index 1.
        (
            vec!["income:salary", "income:dividends"],
            vec!["Dividends", "Salary"],
        ),
        // shared `expenses`, shortest has 3 → index min(1, 2) = 1.
        (
            vec!["expenses:food:groceries", "expenses:housing:rent"],
            vec!["Food", "Housing"],
        ),
        // NOTHING shared → index min(0, 1) = 0, so the roots are the groups.
        (
            vec!["cogs:materials", "expenses:rent"],
            vec!["Cogs", "Expenses"],
        ),
        // Shared `expenses` but a one-segment member → capped to 0, and both
        // accounts land on the same line.
        (
            vec!["expenses", "expenses:food:groceries"],
            vec!["Expenses"],
        ),
    ] {
        let journal = one_expense_per_account(&accounts);
        let report = report_of(&journal, "2026-01-01", "2026-12-31");
        let names: Vec<String> = report
            .sections
            .iter()
            .flat_map(|section| &section.groups)
            .map(|group| group.name.clone())
            .collect();
        assert_eq!(names, want, "groups for {accounts:?}");
    }
}

/// A journal with one `$100.00` posting to each named account, tagged so every
/// one of them lands in the SAME section (which is what the shared-prefix rule
/// is computed over).
fn one_expense_per_account(accounts: &[&str]) -> Journal {
    let declarations: String = accounts
        .iter()
        .map(|account| format!("account {account}  ; type: X, issection: opex\n"))
        .collect();
    let postings: String = accounts
        .iter()
        .enumerate()
        .map(|(index, account)| {
            format!(
                "2026-0{}-01 entry\n    {account}  $100.00\n    assets:bank\n\n",
                index + 1
            )
        })
        .collect();
    parse_journal(
        &format!("account assets:bank  ; type: C\n{declarations}\n{postings}"),
        "prefix-probe.journal",
    )
    .expect("parse")
}

/// The alias table applies on this statement too, and it renames WITHOUT
/// deciding membership — the same guarantee, and the same one table, as the
/// balance sheet's.
#[test]
fn the_group_label_reuses_the_balance_sheets_alias_table() {
    let journal = one_expense_per_account(&["gastos:cc:visa", "gastos:oficina:papel"]);
    let report = report_of(&journal, "2026-01-01", "2026-12-31");
    assert_eq!(
        report
            .group_names(IsSectionKind::Opex)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["Credit cards", "Oficina"],
        "`cc` is aliased; a segment no alias knows keeps its own name"
    );
}

// ===========================================================================
// `issection:` is a closed vocabulary, and a typo is a hard error
// ===========================================================================

#[test]
fn parses_every_section_code_and_nothing_else() {
    for (tag, want) in [
        ("revenue", IsSectionKind::Revenue),
        ("cogs", IsSectionKind::Cogs),
        ("opex", IsSectionKind::Opex),
        ("depreciation", IsSectionKind::Depreciation),
        ("interest", IsSectionKind::Interest),
        ("tax", IsSectionKind::Tax),
        ("other", IsSectionKind::Other),
    ] {
        assert_eq!(parse_is_section_tag(tag), Some(want), "tag {tag}");
        assert_eq!(
            parse_is_section_tag(&format!("  {} ", tag.to_uppercase())),
            Some(want),
            "tag {tag}, padded and upper-cased"
        );
    }
    // No forgiving superset, unlike `type:` — see `parse_is_section_tag`'s docs.
    // The plurals `type:` had to accept, the English names the rejected design
    // would have matched, and plain typos all land in the same place: `None`,
    // which `account_sections_from` turns into a named error.
    for tag in [
        "revenues",
        "taxes",
        "opexes",
        "cost of goods sold",
        "cost-of-goods-sold",
        "Coûts des marchandises vendues",
        "expenses",
        "income",
        "",
        "  ",
        "x",
    ] {
        assert_eq!(parse_is_section_tag(tag), None, "tag {tag:?}");
    }
}

/// **The cautionary tale, headed off.** A misspelt code must NOT fall back to
/// the type-inferred section: the box the user spelled would read zero and
/// nothing on screen would say why. This is the `account-type-not-name` failure
/// with a fresh cause, and it is refused by name.
#[test]
fn an_unrecognized_issection_value_is_a_hard_error() {
    let text = std::fs::read_to_string(common::fixtures_dir().join("reports/is-sections.journal"))
        .expect("read is-sections.journal")
        .replace("issection: cogs", "issection: cost-of-goods-sold");
    let journal =
        parse_journal(&text, "is-sections-typo.journal").expect("the FILE is still valid");

    let error = account_sections(&journal).expect_err("a bad code must not be tolerated");
    assert_eq!(
        error,
        ReportError::UnknownIsSection {
            account: "cogs".to_string(),
            value: "cost-of-goods-sold".to_string(),
        }
    );
    // The message names the account, the value and the way out.
    let message = error.to_string();
    for expected in ["cogs", "cost-of-goods-sold", "revenue", "depreciation"] {
        assert!(message.contains(expected), "{message}");
    }
}

/// An EMPTY value reads as no declaration, exactly as `bsgroup:`/`isgroup:` do —
/// the two tag readers must not answer differently about the same syntax.
#[test]
fn an_empty_issection_value_is_not_a_declaration() {
    let journal = parse_journal(
        "account expenses:rent  ; type: X, issection:\naccount income:fees  ; type: R\n",
        "empty-tag.journal",
    )
    .expect("parse");
    assert_eq!(account_sections(&journal), Ok(BTreeMap::new()));
    assert!(account_sections_from(&journal.accounts).is_ok());
}

/// The tag reader takes only `issection:`, and it inherits down the tree.
#[test]
fn account_sections_reads_only_issection_tags() {
    let journal = sections_journal();
    assert_eq!(
        account_sections(&journal).expect("valid codes"),
        [
            ("cogs", IsSectionKind::Cogs),
            ("expenses:depreciation", IsSectionKind::Depreciation),
            ("expenses:interest", IsSectionKind::Interest),
            ("expenses:lawsuit", IsSectionKind::Other),
            ("expenses:marketing", IsSectionKind::Opex),
            ("expenses:rent", IsSectionKind::Opex),
            ("expenses:salaries", IsSectionKind::Opex),
            ("expenses:taxes", IsSectionKind::Tax),
            ("income:grants", IsSectionKind::Other),
            ("revenue", IsSectionKind::Revenue),
        ]
        .into_iter()
        .map(|(account, kind)| (account.to_string(), kind))
        .collect::<BTreeMap<_, _>>(),
        "accounts declaring only `type:` or `isgroup:` must not appear"
    );
    // sample.journal declares no sections at all, and must still report.
    assert_eq!(
        account_sections(&common::fixture_journal()),
        Ok(BTreeMap::new())
    );

    // The inherited cases really are inherited rather than declared: neither of
    // these accounts carries an `issection:` of its own.
    let report = report(
        &sections_journal(),
        "2026-01-01",
        "2026-12-31",
        Valuation::None,
        false,
    );
    assert_eq!(
        report
            .group(IsSectionKind::Tax, "Federal")
            .rows
            .iter()
            .map(|row| row.account.as_str())
            .collect::<Vec<_>>(),
        ["expenses:taxes:federal"],
        "`expenses:taxes:federal` inherits `tax` from `expenses:taxes`"
    );
}

// ===========================================================================
// Invariants (RPT-1/RPT-2/RPT-4)
// ===========================================================================

/// Every total is summed over MEMBERS: a section's total is its groups', a
/// group's total is its rows', and net income is the whole statement's. Checked
/// on both fixtures, on every basis, with and without a comparison.
#[test]
fn totals_are_summed_over_members_at_every_level() {
    for (journal, from, to) in [
        (common::fixture_journal(), "2026-01-01", "2026-07-08"),
        (sections_journal(), "2026-01-01", "2026-12-31"),
    ] {
        for value in BASES {
            for compare in [false, true] {
                let report = report(&journal, from, to, value, compare);
                let mut running = Amounts::default();
                for section in &report.sections {
                    let from_groups = section
                        .groups
                        .iter()
                        .fold(Amounts::default(), |acc, group| add(&acc, &group.total));
                    assert_amounts_eq(
                        &from_groups,
                        &section.total,
                        &format!(
                            "{} = sum of its groups ({value:?}, compare {compare})",
                            section.title
                        ),
                    );
                    for group in &section.groups {
                        let from_rows = group
                            .rows
                            .iter()
                            .fold(Amounts::default(), |acc, row| add(&acc, &row.amounts));
                        assert_amounts_eq(
                            &from_rows,
                            &group.total,
                            &format!("{} = sum of its rows ({value:?})", group.name),
                        );
                    }
                    // The section contributes `−sum(members)` whatever its
                    // displayed sign — which is why `flip` is one field.
                    let flip =
                        matches!(section.kind, IsSectionKind::Revenue | IsSectionKind::Other);
                    running = add(&running, &negated(&section.total, !flip));
                }
                assert_amounts_eq(
                    &running,
                    &report.net_income,
                    &format!(
                        "net income = every box's contribution ({value:?}, compare {compare})"
                    ),
                );
                // Each subtotal is a PREFIX of that running total, so the last
                // one plus the boxes below it is net income.
                if let Some(last) = report
                    .sections
                    .iter()
                    .flat_map(|section| &section.trailing)
                    .last()
                {
                    assert!(
                        !last.total.current.is_zero() || report.net_income.current.is_zero(),
                        "a subtotal must be a real running total"
                    );
                }
            }
        }
    }
}

fn add(a: &Amounts, b: &Amounts) -> Amounts {
    Amounts {
        current: a.current.ma_add(&b.current).expect("no overflow"),
        prior: match (&a.prior, &b.prior) {
            (Some(x), Some(y)) => Some(x.ma_add(y).expect("no overflow")),
            (Some(only), None) | (None, Some(only)) => Some(only.clone()),
            (None, None) => None,
        },
    }
}

fn negated(a: &Amounts, negate: bool) -> Amounts {
    if !negate {
        return a.clone();
    }
    Amounts {
        current: a.current.ma_neg().expect("no overflow"),
        prior: a
            .prior
            .as_ref()
            .map(|prior| prior.ma_neg().expect("no overflow")),
    }
}

#[track_caller]
fn assert_amounts_eq(actual: &Amounts, want: &Amounts, what: &str) {
    assert_money(&actual.current, &want.current, &format!("{what} (current)"));
    assert_eq!(
        actual.prior.is_some(),
        want.prior.is_some(),
        "{what}: prior shape"
    );
    if let (Some(actual), Some(want)) = (&actual.prior, &want.prior) {
        assert_money(actual, want, &format!("{what} (prior)"));
    }
}

/// RPT-2: membership is decided on the DIRECT per-account totals, so a parent
/// can never net children that belong to different boxes.
///
/// `expenses:salaries` is tagged `opex` and `expenses:lawsuit` `other`; both sit
/// under `expenses`, which is not itself a member of anything. If membership
/// were decided after a roll-up, `expenses` would appear as a fabricated row in
/// whichever box won — a number hledger never prints.
#[test]
fn membership_is_decided_before_anything_is_rolled_up() {
    let journal = sections_journal();
    let report = report(&journal, "2026-01-01", "2026-12-31", Valuation::None, false);
    let accounts: Vec<&str> = report
        .sections
        .iter()
        .flat_map(|section| &section.groups)
        .flat_map(|group| &group.rows)
        .map(|row| row.account.as_str())
        .collect();
    assert!(
        !accounts.contains(&"expenses"),
        "a shared ancestor is not a member of any box: {accounts:?}"
    );
    assert!(
        !accounts.contains(&"expenses:taxes") && accounts.contains(&"expenses:taxes:federal"),
        "only the accounts with postings are rows: {accounts:?}"
    );
    // No account appears twice anywhere on the statement — a row printed in two
    // boxes would double-count into net income.
    let mut sorted = accounts.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), accounts.len(), "duplicate row: {accounts:?}");
}

/// Regrouping must not move a number: tagging accounts into different lines
/// changes only which line they print on.
#[test]
fn tagging_groups_cannot_move_a_total() {
    let journal = sections_journal();
    let tagged = report(&journal, "2026-01-01", "2026-12-31", Valuation::None, true);

    // Same journal with every `isgroup:` stripped, so the fallback names every
    // line instead.
    let text = std::fs::read_to_string(common::fixtures_dir().join("reports/is-sections.journal"))
        .expect("read")
        .replace(", isgroup: Cloud hosting", "")
        .replace(", isgroup: Payment processing", "")
        .replace(", isgroup: Growth", "");
    let untagged_journal = parse_journal(&text, "is-sections-ungrouped.journal").expect("parse");
    let untagged = report(
        &untagged_journal,
        "2026-01-01",
        "2026-12-31",
        Valuation::None,
        true,
    );

    assert_eq!(untagged.kinds(), tagged.kinds());
    for (a, b) in untagged.sections.iter().zip(&tagged.sections) {
        assert_amounts_eq(&a.total, &b.total, &a.title);
    }
    assert_amounts_eq(&untagged.net_income, &tagged.net_income, "net income");
    // … and the lines really did move, so the test is not vacuous.
    assert_eq!(
        untagged.group_names(IsSectionKind::Cogs),
        [
            ("Hosting".to_string(), GroupSource::Segment),
            ("Payments".to_string(), GroupSource::Segment),
        ]
    );
}

/// An unclassifiable account is on no statement at all, and must not silently
/// reach net income. `assets:`/`liabilities:`/`equity:` are the obvious cases;
/// `mystery:` is the one that resolves to no type at all.
#[test]
fn only_revenue_and_expense_accounts_reach_the_statement() {
    let journal = parse_journal(
        "account assets:bank    ; type: C\n\
         account income:fees    ; type: R\n\
         account expenses:rent  ; type: X\n\
         \n\
         2026-01-01 fees\n    assets:bank  $500.00\n    income:fees\n\
         \n\
         2026-02-01 rent\n    expenses:rent  $200.00\n    assets:bank\n\
         \n\
         2026-03-01 mystery\n    mystery:pot  $50.00\n    assets:bank\n",
        "unclassifiable.journal",
    )
    .expect("parse");
    let report = report_of(&journal, "2026-01-01", "2026-12-31");
    let accounts: Vec<&str> = report
        .sections
        .iter()
        .flat_map(|section| &section.groups)
        .flat_map(|group| &group.rows)
        .map(|row| row.account.as_str())
        .collect();
    assert_eq!(accounts, ["income:fees", "expenses:rent"]);
    assert_amounts(&report.net_income, 30_000, None, "Net income");
}

// ===========================================================================
// Valuation
// ===========================================================================

/// A commodity with no price is left on the line — never dropped — and named in
/// `meta.unpriced`, exactly as `hledger is -V` prints it and as the balance
/// sheet already does.
#[test]
fn unpriced_commodities_stay_on_the_line_and_are_reported() {
    let journal = parse_journal(
        "account assets:broker  ; type: A\n\
         account income:gifts   ; type: R\n\
         \n\
         2026-01-01 gift of shares\n    assets:broker  5 GLD\n    income:gifts  -5 GLD\n",
        "unpriced.journal",
    )
    .expect("parse");
    let baseless = report(
        &journal,
        "2026-01-01",
        "2026-12-31",
        Valuation::Market,
        false,
    );
    // No `P` directive anywhere, so there is no base commodity to value into and
    // nothing is unpriced — there is no target to be unpriced RELATIVE TO.
    assert_eq!(baseless.base, None);
    assert_money(
        &baseless.section(IsSectionKind::Revenue).total.current,
        &MixedAmount::single(commodity("GLD"), Dec::new(5, 0)),
        "the shares stay on the line",
    );

    // Give it a base commodity that cannot reach GLD, and the shares still stay
    // — now with `meta.unpriced` saying so.
    let priced = parse_journal(
        "account assets:broker  ; type: A\n\
         account income:gifts   ; type: R\n\
         account income:fees    ; type: R\n\
         \n\
         P 2026-01-01 EUR $1.10\n\
         \n\
         2026-01-01 gift of shares\n    assets:broker  5 GLD\n    income:gifts  -5 GLD\n\
         \n\
         2026-02-01 fees\n    assets:broker  $100.00\n    income:fees\n",
        "unpriced-with-base.journal",
    )
    .expect("parse");
    let valued = report(
        &priced,
        "2026-01-01",
        "2026-12-31",
        Valuation::Market,
        false,
    );
    assert_eq!(valued.base, Some(commodity("$")));
    assert_eq!(valued.meta.unpriced, vec![commodity("GLD")]);
    let mut want = MixedAmount::single(commodity("$"), Dec::new(10_000, 2));
    want.accumulate(&commodity("GLD"), Dec::new(5, 0)).unwrap();
    assert_money(
        &valued.section(IsSectionKind::Revenue).total.current,
        &want,
        "valued dollars beside the shares no price reaches",
    );
}

/// `hledger -f fixtures/sample.journal is -B -b 2024-07-01 -e 2026-07-09`
/// ```text
///  Revenues  $132,851.25  (salary $132,720.00 + dividends $131.25)
///  Expenses   $97,352.34, 933,25 EUR  (incl. depreciation $7,500.00)
///  Net:       $35,498.91, -933,25 EUR
/// ```
/// The at-cost net is the balance sheet's Retained earnings, which is the whole
/// reason that line can exist — so this is the two reports tying out. The same
/// figure is asserted from the other side by
/// `balance_sheet_grouped::sample_at_cost_matches_hledger_bse_b`, off the same
/// `bse -B` / `is -B` pair; if only one of the two is ever updated, they stop
/// agreeing and both say so.
#[test]
fn at_cost_net_income_is_the_balance_sheets_retained_earnings() {
    let journal = common::fixture_journal();
    let report = report(&journal, "2024-07-01", "2026-07-08", Valuation::Cost, false);
    let mut want = usd(3_549_891);
    want.accumulate(&commodity("EUR"), Dec::new(-93_325, 2))
        .unwrap();
    assert_money(&report.net_income.current, &want, "at-cost net income");
    assert_eq!(report.base, None, "cost collapses to no one commodity");
    assert!(report.meta.unpriced.is_empty(), "nothing is valued at all");
}

/// Flipping the valuation basis must not move an account between boxes or
/// between lines — the two are decided by tags and tree position, neither of
/// which a price can touch.
#[test]
fn valuation_cannot_move_an_account_between_boxes_or_lines() {
    let journal = common::fixture_journal();
    let membership = |value| {
        let report = report(&journal, "2024-07-01", "2026-07-08", value, true);
        report
            .sections
            .iter()
            .map(|section| {
                (
                    section.kind,
                    section
                        .groups
                        .iter()
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
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(membership(Valuation::Market), membership(Valuation::Cost));
    assert_eq!(membership(Valuation::Market), membership(Valuation::None));
}
