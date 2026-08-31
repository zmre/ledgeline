//! Balance sheet — port of `web/src/lib/reports/balanceSheet.ts`, plus the
//! grouped/valued three-box report of `plans/12-balance-sheet-redesign.md`.

use super::ReportError;
use super::account_groups::{
    AccountGroups, BsTerm, GroupSource, RETAINED_EARNINGS_GROUP, VALUATION_ADJUSTMENT_GROUP,
    group_rank, resolve_bs_term,
};
use super::account_types::{AccountType, AccountTypes};
use super::aggregate::{PostingFilter, account_totals, at_depth, roll_up};
use super::mixed_amount::MixedAmount;
use super::prices::{PriceDb, ValuationMeta, priced_count, value_at};
use super::sections::build_section;
use super::types::{ReportMeta, ReportRow, SectionedReport};
use crate::decimal::Dec;
use crate::model::{Commodity, PriceDirective, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

/// Asset + liability balances as of `as_of` (INCLUSIVE: postings dated ≤
/// `as_of`). Presentation matches `hledger bs`: liabilities are sign-flipped
/// (positive = owed); `grand_total` = assets − liabilities(displayed).
///
/// `declared` carries the journal's `type:` declarations so sections are keyed
/// on effective account type rather than on root names.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn balance_sheet(
    txns: &[Transaction],
    as_of: &str,
    depth: usize,
    declared: &BTreeMap<String, AccountType>,
) -> Result<SectionedReport, ReportError> {
    let direct = account_totals(
        txns,
        &PostingFilter {
            to: Some(as_of),
            ..PostingFilter::default()
        },
    )?;
    let clamped = at_depth(&roll_up(&direct)?, depth);
    let assets = build_section(
        "Assets",
        AccountType::Asset,
        &direct,
        &clamped,
        declared,
        false,
    )?;
    let liabilities = build_section(
        "Liabilities",
        AccountType::Liability,
        &direct,
        &clamped,
        declared,
        true,
    )?;
    let grand_total = assets.total.ma_add(&liabilities.total.ma_neg()?)?;
    Ok(SectionedReport {
        as_of: Some(as_of.to_string()),
        from: None,
        to: None,
        sections: vec![assets, liabilities],
        grand_total,
    })
}

// ===========================================================================
// Grouped, valued balance sheet — plans/12-balance-sheet-redesign.md
// ===========================================================================

/// The basis a grouped balance sheet reports its numbers on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Valuation {
    /// Market value at `as_of` in the base commodity — hledger's `-V`.
    /// Commodities no chain of prices reaches are left as written (and reported
    /// in `meta.unpriced`), never silently dropped.
    #[default]
    Market,
    /// Cost basis — hledger's `-B`.
    Cost,
    /// As written: no valuation at all — hledger's default.
    None,
}

/// Inputs to [`balance_sheet_grouped`].
#[derive(Debug, Clone)]
pub struct BsOpts<'a> {
    /// Point-in-time date (INCLUSIVE).
    pub as_of: &'a str,
    /// Account-depth clamp for the expandable rows inside each group, or `None`
    /// for no clamp at all. Group and section totals are summed over MEMBERS, so
    /// they do not move with it.
    ///
    /// `None` rather than a sentinel depth: `Some(0)` already means hledger's
    /// "totals only" (no rows), so unlimited cannot be spelled `0`, and spelling
    /// it as some large number would make "unclamped" a number that happens to
    /// exceed every chart of accounts rather than a stated fact.
    pub depth: Option<usize>,
    /// The basis every displayed number is on.
    pub value: Valuation,
    /// Override the valuation target; defaults to `prices.base_commodity()`.
    pub value_in: Option<Commodity>,
}

/// Which of the three boxes a section is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BsSectionKind {
    /// Assets (natural sign).
    Assets,
    /// Liabilities (displayed sign-flipped: positive = owed).
    Liabilities,
    /// Equity (displayed sign-flipped: positive = owner's stake).
    Equity,
}

impl BsSectionKind {
    /// The lowercase plural used inside a subtotal label ("Total current
    /// assets"), which is not always the box title lowercased.
    #[must_use]
    fn noun(self) -> &'static str {
        match self {
            Self::Assets => "assets",
            Self::Liabilities => "liabilities",
            Self::Equity => "equity",
        }
    }
}

/// One collapsible group of accounts, plus the two synthetic equity lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BsGroup {
    /// Display name — a `bsgroup:` tag verbatim, a built-in, or a humanized
    /// path segment.
    pub name: String,
    /// Which resolution step named it.
    pub source: GroupSource,
    /// Which half of the box it prints under, or `None` when the journal
    /// declares no `bsterm:` at all and the split is therefore switched off.
    ///
    /// A group is keyed by (term, name), so one `bsgroup:` spanning both halves
    /// prints as two lines under two subheadings. That is not a defect: a
    /// receivable due this year and one due in five is genuinely two lines on a
    /// real statement.
    pub term: Option<BsTerm>,
    /// The group's accounts, rolled up and clamped to `depth`. Empty for the
    /// synthetic (`Computed`) lines, which stand for no accounts.
    pub rows: Vec<ReportRow>,
    /// Summed over the group's MEMBERS, not over `rows` — so it is
    /// depth-independent (RPT-1/RPT-4).
    pub total: MixedAmount,
}

/// One half of a box: its subheading and its subtotal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BsSubsection {
    /// Current or non-current.
    pub term: BsTerm,
    /// Subheading above the half — `"Current"` / `"Non-current"`.
    pub heading: String,
    /// Subtotal label — `"Total current assets"`, and so on. Built here rather
    /// than in each consumer so the screen and the spreadsheet cannot word it
    /// differently.
    pub label: String,
    /// Summed over this half's MEMBERS, like every other total on the sheet.
    pub total: MixedAmount,
}

/// One of the three boxes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BsSection {
    /// Which box.
    pub kind: BsSectionKind,
    /// Display title.
    pub title: String,
    /// Groups in presentation order: by term (current first), then
    /// `account_groups::group_rank`, then name.
    pub groups: Vec<BsGroup>,
    /// The box's halves, current first. EMPTY when the split is off, which is
    /// the adaptive guarantee: a journal that declares no `bsterm:` gets exactly
    /// the report it got before this existed. Always empty for Equity.
    ///
    /// When non-empty, every group in this section has `Some` term, and groups
    /// sharing a term are contiguous — so a consumer can walk `groups` once and
    /// look each term up here.
    pub subsections: Vec<BsSubsection>,
    /// Summed over the section's MEMBERS, not over `groups`' rows.
    pub total: MixedAmount,
}

/// A grouped, valued balance sheet where `Assets == Liabilities + Equity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceSheetReport {
    /// The point-in-time date (INCLUSIVE).
    pub as_of: String,
    /// The commodity everything is valued into — `Some` only when
    /// [`Valuation::Market`] found a target, since neither of the other bases
    /// collapses to a single commodity.
    pub base: Option<Commodity>,
    /// Always exactly three, in order: Assets, Liabilities, Equity.
    pub sections: Vec<BsSection>,
    /// Assets − Liabilities, both as displayed.
    pub net_worth: MixedAmount,
    /// `A − L − E`, the EXACT residual. Never rounded, never tolerated — this is
    /// the number to show the user when [`Self::balanced`] is false.
    pub check: MixedAmount,
    /// Whether [`Self::check`] is small enough to be arithmetic dust rather than
    /// a real imbalance — see [`is_balanced`] for the rule, and for what the
    /// one-cent floor under it does and does not still catch.
    ///
    /// This, and never `check.is_zero()`, is what a caller should render a
    /// ✓/✗ from.
    pub balanced: bool,
    /// Commodities the market valuation could not reach (sorted, deduped).
    pub meta: ReportMeta,
}

/// The three sections, in presentation order: the type each selects, and
/// whether it is displayed sign-flipped. `Cash` folds into `Asset`, `Conversion`
/// folds into `Equity` (hledger's own subtyping — `bse` files a declared
/// `type: V` account under Equity), and these three types are mutually
/// exclusive, so every balance-sheet account belongs to exactly one of them —
/// which is what lets the check below be a sum.
const SECTIONS: [(BsSectionKind, &str, AccountType, bool); 3] = [
    (BsSectionKind::Assets, "Assets", AccountType::Asset, false),
    (
        BsSectionKind::Liabilities,
        "Liabilities",
        AccountType::Liability,
        true,
    ),
    (BsSectionKind::Equity, "Equity", AccountType::Equity, true),
];

/// A grouped balance sheet in which `Assets == Liabilities + Equity` holds
/// EXACTLY, so the `check` line is a real journal-integrity signal.
///
/// # Why the identity holds, and what breaks it
///
/// Transactions balance **at cost**, not as written: `10 AAPL @ $220.00`
/// against `$-2,200.00` sums to zero only once the cost annotation is applied.
/// So the textbook identity is an at-cost statement, and hledger agrees —
/// on `fixtures/sample.journal` at 2026-07-08, `bse -B` and `is -B` both report
/// a Net of `$42,998.91, -933,25 EUR`, i.e.
/// `Assets − Liabilities − Equity − RetainedEarnings ≡ 0`. Without `-B` they
/// differ, by exactly the cost residue of every priced posting.
///
/// This report therefore keeps COST as its backbone and books the difference:
///
/// - **Retained earnings** = `−(revenues + expenses)` at cost through `as_of` —
///   the same quantity, with the same sign handling, that
///   [`super::income_statement`] calls net income.
/// - **Valuation adjustment** = the balance sheet on the DISPLAY basis minus the
///   same accounts at cost. At [`Valuation::Cost`] that is exactly zero and the
///   line disappears; at [`Valuation::Market`] it is the unbooked revaluation;
///   at [`Valuation::None`] it is the cost residue. In every case it is precisely
///   what valuing the sheet moved, so `A = L + E` survives the valuation toggle.
///   See [`VALUATION_ADJUSTMENT_GROUP`] for why that, and not "unrealized
///   gains", is what the line may be called.
///
/// What is left over is `check = A − L − E`, which reduces algebraically to
/// **the sum of every classifiable account at cost** — nothing about
/// presentation survives into it. Every display-basis term appears once with
/// each sign (the three sections partition the very map the valuation
/// adjustment sums) and cancels EXACTLY, whatever the valuation did: `Dec`
/// addition is exact, `value_at` multiplies by one rate per commodity pair, and
/// multiplication distributes over that addition, so valuing per account and
/// summing equals valuing the sum. A rounded or reciprocal cross-rate is
/// therefore not a source of residue here — see
/// `tests::a_rounded_cross_rate_leaves_no_residue`.
///
/// The residual catches two real failures:
///
/// 1. a transaction that does not balance at cost, and
/// 2. an account whose effective type cannot be resolved — which appears in no
///    section at all, the "report reads zero" failure mode (RPT-1).
///
/// It is *not* reliably empty, though, and [`BalanceSheetReport::balanced`] —
/// not `check.is_zero()` — is the verdict. See [`is_balanced`].
///
/// # Valuation
///
/// `explicit_prices` are the journal's `P` directives, and they are the ONLY
/// price source: this matches `hledger bs -V`, which does not infer prices from
/// cost annotations unless asked. (`net_worth` deliberately does infer them —
/// it is modelling `--infer-market-prices`.) The difference is visible and
/// intended: `sample.journal`'s GLD and TSLA have no `P` directive, so they stay
/// as share counts and are named in `meta.unpriced` rather than being conjured
/// into dollars from a stale cost.
///
/// # Ordering
///
/// Membership is decided on the DIRECT totals and rolled up per group
/// afterwards (RPT-2), and every total is summed over members rather than over
/// displayed rows (RPT-1/RPT-4), so `depth` moves the rows and nothing else.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn balance_sheet_grouped(
    txns: &[Transaction],
    explicit_prices: &[PriceDirective],
    opts: &BsOpts,
    declared: &BTreeMap<String, AccountType>,
    groups: &BTreeMap<String, String>,
    terms: &BTreeMap<String, BsTerm>,
) -> Result<BalanceSheetReport, ReportError> {
    let totals = |at_cost| {
        account_totals(
            txns,
            &PostingFilter {
                to: Some(opts.as_of),
                at_cost,
                ..PostingFilter::default()
            },
        )
    };
    let written = totals(false)?;
    let at_cost = totals(true)?;

    let prices = PriceDb::build(explicit_prices);
    let target: Option<Commodity> = opts
        .value_in
        .clone()
        .or_else(|| prices.base_commodity().cloned());
    let types = AccountTypes::from_declared(declared.clone());
    // The group signal reads the AS-WRITTEN balances, so flipping the valuation
    // never moves an account between groups.
    let account_groups =
        AccountGroups::new(groups.clone(), types.clone(), &written, target.as_ref());

    // Only balance-sheet accounts are ever displayed — revenues and expenses
    // reach the report solely through retained earnings, at cost. Narrowing here
    // also keeps `meta.unpriced` to commodities the user can actually see.
    let on_sheet = |source: &BTreeMap<String, MixedAmount>| -> BTreeMap<String, MixedAmount> {
        source
            .iter()
            .filter(|(account, _)| {
                SECTIONS
                    .iter()
                    .any(|(_, _, category, _)| types.is_type(account, *category))
            })
            .map(|(account, ma)| (account.clone(), ma.clone()))
            .collect()
    };
    let sheet_cost = on_sheet(&at_cost);

    let mut meta = ValuationMeta::default();
    let display: BTreeMap<String, MixedAmount> = match (opts.value, target.as_ref()) {
        (Valuation::Cost, _) => sheet_cost.clone(),
        (Valuation::None, _) | (Valuation::Market, None) => on_sheet(&written),
        (Valuation::Market, Some(base)) => on_sheet(&written)
            .iter()
            .map(|(account, ma)| {
                Ok((
                    account.clone(),
                    valued_keeping_unpriced(ma, base, &prices, opts.as_of, &mut meta)?,
                ))
            })
            .collect::<Result<_, ReportError>>()?,
    };

    let retained = sum_all(&members_of(&at_cost, &types, AccountType::Revenue))?
        .ma_add(&sum_all(&members_of(
            &at_cost,
            &types,
            AccountType::Expense,
        ))?)?
        .ma_neg()?;
    let valuation_adjustment = sum_all(&display)?.ma_add(&sum_all(&sheet_cost)?.ma_neg()?)?;

    // ADAPTIVE: with no `bsterm:` anywhere the split is off entirely and every
    // group's term is `None`, so the report is byte-identical to the one this
    // feature did not exist for. A personal ledger is never handed a
    // classification it did not ask for.
    let bs_terms = if terms.is_empty() { None } else { Some(terms) };

    let mut sections: Vec<BsSection> = Vec::with_capacity(SECTIONS.len());
    for (kind, title, category, flip) in SECTIONS {
        let members = members_of(&display, &types, category);
        let mut total = signed(&sum_all(&members)?, flip)?;
        // Equity is never split — the question the split asks (when does this
        // convert to cash, when does this come due) is not asked of capital.
        let section_terms = if kind == BsSectionKind::Equity {
            None
        } else {
            bs_terms
        };
        let mut section_groups = build_groups(
            &members,
            &display,
            &account_groups,
            opts.depth,
            flip,
            section_terms,
        )?;
        if kind == BsSectionKind::Equity {
            // Retained earnings is a real line even at zero; the valuation
            // adjustment only exists when the display basis moved off cost.
            for (name, amount) in [
                (RETAINED_EARNINGS_GROUP, &retained),
                (VALUATION_ADJUSTMENT_GROUP, &valuation_adjustment),
            ] {
                if name == VALUATION_ADJUSTMENT_GROUP && amount.is_zero() {
                    continue;
                }
                total = total.ma_add(amount)?;
                section_groups.push(BsGroup {
                    name: name.to_string(),
                    source: GroupSource::Computed,
                    term: None,
                    rows: Vec::new(),
                    total: amount.clone(),
                });
            }
            sort_groups(&mut section_groups);
        }
        let subsections = if section_terms.is_some() {
            build_subsections(kind, &section_groups)?
        } else {
            Vec::new()
        };
        sections.push(BsSection {
            kind,
            title: title.to_string(),
            groups: section_groups,
            subsections,
            total,
        });
    }

    let net_worth = sections[0].total.ma_add(&sections[1].total.ma_neg()?)?;
    let check = net_worth.ma_add(&sections[2].total.ma_neg()?)?;
    // Short-circuit: an empty residual is balanced under any precision, so the
    // extra pass over the postings is only paid when there is dust to judge.
    let balanced =
        check.is_zero() || is_balanced(&check, &WrittenPrecision::scan(txns, opts.as_of));
    meta.unpriced.sort();
    meta.unpriced.dedup();

    Ok(BalanceSheetReport {
        as_of: opts.as_of.to_string(),
        base: match opts.value {
            Valuation::Market => target,
            Valuation::Cost | Valuation::None => None,
        },
        sections,
        net_worth,
        check,
        balanced,
        meta: ReportMeta {
            unpriced: meta.unpriced,
        },
    })
}

/// True when valuing this balance sheet in `target` prices at least one of the
/// commodities it would display (vacuously true for an empty sheet).
///
/// The HTTP layer's admission test for an explicit `valueIn` on
/// `/api/reports/balancesheet/grouped` — the question
/// `holdings::prices_any_held` answers for the Holdings tabs, measured over
/// THIS report's own rows: the as-written balances, as of `as_of`, of every
/// account the three boxes display (the same [`SECTIONS`] narrowing
/// [`balance_sheet_grouped`] applies before valuing). The price set is the
/// explicit-`P`-only [`PriceDb`] the report itself values with — never
/// cost-inferred directives, see the valuation notes on
/// [`balance_sheet_grouped`] — so admission can never disagree with the report
/// it admits.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn prices_any_on_sheet(
    txns: &[Transaction],
    explicit_prices: &[PriceDirective],
    as_of: &str,
    declared: &BTreeMap<String, AccountType>,
    target: &Commodity,
) -> Result<bool, ReportError> {
    let types = AccountTypes::from_declared(declared.clone());
    let written = account_totals(
        txns,
        &PostingFilter {
            to: Some(as_of),
            ..PostingFilter::default()
        },
    )?;
    let held: BTreeSet<Commodity> = written
        .iter()
        .filter(|(account, _)| {
            SECTIONS
                .iter()
                .any(|(_, _, category, _)| types.is_type(account, *category))
        })
        .flat_map(|(_, ma)| ma.iter().map(|(commodity, _)| commodity.clone()))
        .collect();
    Ok(
        held.is_empty()
            || priced_count(&held, target, &PriceDb::build(explicit_prices), as_of)? > 0,
    )
}

// ---------------------------------------------------------------------------
// The balanced/not-balanced verdict
// ---------------------------------------------------------------------------

/// One hundredth of a unit — a cent, on a dollar book — as a count of decimal
/// places, which is the form [`is_balanced`] needs.
///
/// A FLOOR under the tolerance, never a ceiling: `10^-p` shrinks as `p` grows,
/// so `10^-min(p, 2)` is exactly `max(10^-p, 0.01)`. Clamping the places can
/// only widen the threshold.
const TOLERANCE_FLOOR_PLACES: u32 = 2;

/// Whether `check` is dust rather than an imbalance: true when EVERY commodity's
/// residual is strictly under that commodity's tolerance,
///
/// ```text
/// tolerance(c) = max(10^-p(c), 0.01)
/// ```
///
/// where `p(c)` is the widest precision the journal WRITES for `c` (see
/// [`WrittenPrecision`]). Equivalently, and as implemented, one unit at
/// `min(p(c), `[`TOLERANCE_FLOOR_PLACES`]`)` places.
///
/// # Why there is a tolerance at all
///
/// `check` is the exact at-cost sum, and on a real journal that is not zero. A
/// priced posting contributes `quantity × unit price`, and `Dec` multiplication
/// adds the scales — so `1.234 VTI @ $302.4567` costs `$373.2315678`, five
/// digits finer than the `$-373.23` written against it. hledger accepts the
/// entry (its balance check tolerates half a unit at the precision the entry was
/// WRITTEN at, which [`crate::parse::check_transaction_balances`] reproduces),
/// and hledger's own `bal -B` carries the same `$0.0015678` — it is simply
/// hidden behind two-decimal display. Ours was not hidden, so a journal both
/// tools call valid reported "should be zero, but it is $0.00227970".
///
/// # Why the written precision, and why a floor under it
///
/// A flat cent alone is meaningless for the share quantities that sit on the
/// same sheet (`5 GLD`), and wrong for a journal that is not dollar-denominated:
/// those commodities need a threshold of a whole unit, and the written precision
/// is what supplies one. So the data still sets the tolerance — upward.
///
/// It may no longer set it downward, and that is the bug this floor fixes. The
/// precision rule ALONE made the tolerance a function of the most finely written
/// posting anywhere in the book, which has nothing to do with how large a cost
/// residue can grow. Patrick's journal writes dollars at three and four places —
/// brokerage interest, FX conversions and dividend reinvestments all do — so
/// `p($)` was 4, the threshold collapsed to `$0.0001`, and the very
/// `$0.00227970` of dust `bs-cost-dust.journal` exists to excuse was called an
/// imbalance again. `bs-cent-floor.journal` is that journal reduced to three
/// transactions.
///
/// One hundredth is a deliberate **product** decision, asked for in those words:
/// the balance sheet ignores imbalances below one cent.
///
/// # What the floor can and cannot hide — the honest version
///
/// The precision rule carried a proof: `Dec` addition takes the *wider* of its
/// operands' scales, so a sum of amounts each written at ≤ `p` places is itself
/// at ≤ `p` places, and any non-zero such sum is therefore at least `10^-p` —
/// above the threshold by construction. Only cost multiplication, which
/// manufactures extra places, could land underneath.
///
/// **That proof does not survive the floor** for a commodity written finer than
/// two places: at `p = 4` a sum of written amounts can be `$0.0001`, and the
/// floor tolerates it. What holds instead, failure by failure:
///
/// - **An account in no section (RPT-1)** contributes that account's WHOLE
///   balance to the residual. The failure mode being detected is a report that
///   quietly reads zero while real money is missing; an account whose entire
///   balance is under one cent is not that.
///   `tests::check_flags_an_account_that_lands_in_no_section` pins `$-50.00`.
/// - **A transaction that does not balance at cost** still fires here when it is
///   worth flagging: `fixtures/reports/errors/bs-unbalanced.journal` is `$10.00`
///   out, a thousand cents clear of the floor. A SUB-cent one, in a journal that
///   writes finer than cents, no longer fires *here* — but it is not lost.
///   [`crate::parse::check_transaction_balances`] judges every entry on its own,
///   at hledger's own tolerance (half a unit at the precision that entry was
///   written at — `$0.00005` for a four-place entry), and raises it as a
///   diagnostic. This report is the second net for that class, never the only
///   one. Pinned by
///   `tests/balance_sheet_grouped.rs::a_sub_cent_entry_imbalance_is_caught_by_the_parser_not_by_the_floor`.
///
/// The one caveat worth stating plainly: the floor is one hundredth **of the
/// commodity**, not of a dollar. On a book denominated in something with a high
/// unit value — a BTC ledger — 0.01 units is a real sum, and an imbalance that
/// size would read as balanced here. The mitigation is only that
/// [`BalanceSheetReport::check`] always carries the exact residual, whatever the
/// verdict, so the number is on screen to be looked at.
fn is_balanced(check: &MixedAmount, precision: &WrittenPrecision) -> bool {
    check.iter().all(|(commodity, residual)| {
        under_one_unit(
            *residual,
            precision.of(commodity).min(TOLERANCE_FLOOR_PLACES),
        )
    })
}

/// `|residual| < 10^-precision`.
///
/// Deliberately STRICT: a residual of exactly one unit is a real cent (or a real
/// share), and something a posting could have been written for.
///
/// Shaped like [`crate::parse::check_transaction_balances`]'s own `looks_zero`,
/// which allows HALF a unit (`|residual| <= 0.5 × 10^-precision`) inclusively.
/// The two differ only in that threshold, and deliberately: that one judges a
/// single entry by what hledger accepts, this one judges a whole report by what
/// a posting could have said.
fn under_one_unit(residual: Dec, precision: u32) -> bool {
    let Some(extra) = residual.places.checked_sub(precision) else {
        // Coarser than the commodity's own precision: only an exact zero can be
        // under one unit of it.
        return residual.mantissa == 0;
    };
    // Past ~38 extra places one unit exceeds every representable `i128`
    // mantissa, so nothing can reach it.
    i128::checked_pow(10, extra)
        .is_none_or(|unit| residual.mantissa.unsigned_abs() < unit.unsigned_abs())
}

/// The widest precision the journal WRITES for each commodity, over the postings
/// this report draws on (effective date ≤ `as_of`, the rule
/// [`account_totals`] applies).
///
/// Posting amounts and cost annotations are kept apart because a cost is a
/// price, not a posting: `1.234 VTI @ $302.4567` says nothing about how finely
/// dollars are POSTED, and letting its four places set the dollar tolerance
/// would re-flag the very dust that annotation creates. The cost places are
/// still the fallback for a commodity that reaches the report only as a cost —
/// the same two-tier rule, for the same reason, as
/// `parse::Residual::tolerance_precision`.
#[derive(Debug, Default)]
struct WrittenPrecision {
    posted: BTreeMap<Commodity, u32>,
    costed: BTreeMap<Commodity, u32>,
}

impl WrittenPrecision {
    fn scan(txns: &[Transaction], as_of: &str) -> Self {
        txns.iter()
            .flat_map(|txn| txn.postings.iter().map(move |posting| (txn, posting)))
            .filter(|(txn, posting)| posting.date.as_deref().unwrap_or(&txn.date) <= as_of)
            .flat_map(|(_, posting)| &posting.amounts)
            .fold(Self::default(), |mut acc, amount| {
                widen(&mut acc.posted, &amount.commodity, amount.quantity.places);
                if let Some(cost) = amount.cost.as_deref() {
                    widen(
                        &mut acc.costed,
                        &cost.amount.commodity,
                        cost.amount.quantity.places,
                    );
                }
                acc
            })
    }

    /// A commodity nothing in the window mentions gets 0 places — whole units,
    /// the most conservative reading available.
    fn of(&self, commodity: &Commodity) -> u32 {
        self.posted
            .get(commodity)
            .or_else(|| self.costed.get(commodity))
            .copied()
            .unwrap_or(0)
    }
}

fn widen(into: &mut BTreeMap<Commodity, u32>, commodity: &Commodity, places: u32) {
    let slot = into.entry(commodity.clone()).or_default();
    *slot = (*slot).max(places);
}

/// Value `ma` into `target`, KEEPING the commodities no price reaches.
///
/// [`super::net_worth`]'s `valued` drops them, because a net-worth series has to
/// be one number per bucket. A balance sheet does not: dropping five GLD shares
/// would understate the assets with nothing on the row to say so, so they stay
/// visible alongside the valued part and are recorded in `meta` as well. That is
/// also exactly what `hledger bs -V` prints.
///
/// `pub(super)` because [`super::income_statement::income_statement_grouped`]
/// values its rows on exactly these terms — `hledger is -V` keeps an unpriced
/// commodity on the line too — and a second copy of the keep-and-record dance is
/// a second chance to drop a holding silently.
pub(super) fn valued_keeping_unpriced(
    ma: &MixedAmount,
    target: &Commodity,
    prices: &PriceDb,
    as_of: &str,
    meta: &mut ValuationMeta,
) -> Result<MixedAmount, ReportError> {
    let mut skipped = ValuationMeta::default();
    let mut out = MixedAmount::single(
        target.clone(),
        value_at(ma, target, prices, as_of, Some(&mut skipped))?,
    );
    for commodity in skipped.unpriced {
        if let Some(qty) = ma.get(&commodity) {
            out.accumulate(&commodity, qty)?;
        }
        meta.note_unpriced(&commodity);
    }
    out.drop_zeros();
    Ok(out)
}

/// The accounts in `totals` whose EFFECTIVE type is `category`.
fn members_of(
    totals: &BTreeMap<String, MixedAmount>,
    types: &AccountTypes,
    category: AccountType,
) -> BTreeMap<String, MixedAmount> {
    totals
        .iter()
        .filter(|(account, _)| types.is_type(account, category))
        .map(|(account, ma)| (account.clone(), ma.clone()))
        .collect()
}

/// Commodity-wise sum of every balance in `members`.
fn sum_all(members: &BTreeMap<String, MixedAmount>) -> Result<MixedAmount, ReportError> {
    Ok(members
        .values()
        .try_fold(MixedAmount::new(), |acc, ma| acc.ma_add(ma))?)
}

/// `ma`, negated when the section is displayed sign-flipped.
pub(super) fn signed(ma: &MixedAmount, flip: bool) -> Result<MixedAmount, ReportError> {
    Ok(if flip { ma.ma_neg()? } else { ma.clone() })
}

/// Known groups in balance-sheet order, then the rest alphabetically, then the
/// synthetic equity lines.
fn sort_groups(groups: &mut [BsGroup]) {
    groups.sort_by_key(|group| {
        (
            // Current before non-current; `None` (split off) is one bucket, so
            // this reduces to the old ordering exactly.
            group.term.map_or(0, BsTerm::rank),
            group_rank(&group.name, group.source),
            group.name.clone(),
        )
    });
}

/// Partition `members` into groups, each rolled up and clamped to `depth`.
///
/// When two accounts resolve to the same NAME by different steps (a `bsgroup:
/// Investments` tag beside a commodity-detected `Investments`), the group keeps
/// the lexically-first member's source. The accounts are grouped either way;
/// only the provenance label is ambiguous, and it is resolved deterministically.
///
/// A group every one of whose members is exactly zero is dropped, matching
/// hledger's own omission of all-zero accounts: a closed position leaves an
/// account behind whose as-written balance is nothing at all, and a header with
/// nothing under it is noise. Since its total is zero by definition, dropping it
/// cannot move the section total (which is summed over members regardless).
/// Zero ROWS inside a non-empty group are kept, exactly as `build_section` keeps
/// them.
fn build_groups(
    members: &BTreeMap<String, MixedAmount>,
    sheet: &BTreeMap<String, MixedAmount>,
    groups: &AccountGroups,
    depth: Option<usize>,
    flip: bool,
    terms: Option<&BTreeMap<String, BsTerm>>,
) -> Result<Vec<BsGroup>, ReportError> {
    // Keyed by (term, name): with the split ON, one `bsgroup:` whose accounts
    // straddle the halves prints as two lines under two subheadings, which is
    // what a statement does with a receivable due this year and one due in five.
    // With it OFF every key's term is `None` and this is the old grouping
    // exactly.
    type Bucket = (GroupSource, BTreeMap<String, MixedAmount>);
    let mut buckets: BTreeMap<(Option<BsTerm>, String), Bucket> = BTreeMap::new();
    for (account, ma) in members {
        let (name, source) = groups.resolve(account);
        let term = terms.map(|declared| resolve_bs_term(account, &name, declared));
        buckets
            .entry((term, name))
            .or_insert_with(|| (source, BTreeMap::new()))
            .1
            .insert(account.clone(), ma.clone());
    }

    let mut out: Vec<BsGroup> = buckets
        .into_iter()
        .filter(|(_, (_, group_members))| group_members.values().any(|ma| !ma.is_zero()))
        .map(|((term, name), (source, group_members))| {
            Ok(BsGroup {
                name,
                source,
                term,
                rows: group_rows(&group_members, sheet, depth, flip)?,
                total: signed(&sum_all(&group_members)?, flip)?,
            })
        })
        .collect::<Result<_, ReportError>>()?;
    sort_groups(&mut out);
    Ok(out)
}

/// The halves of one box, current first, each with its own subtotal.
///
/// Summed over the GROUPS' totals, which are themselves summed over members —
/// so a subtotal is depth-independent for the same reason every other total on
/// this sheet is, and the halves add to the section total by construction.
fn build_subsections(
    kind: BsSectionKind,
    groups: &[BsGroup],
) -> Result<Vec<BsSubsection>, ReportError> {
    let mut out: Vec<BsSubsection> = Vec::new();
    for term in [BsTerm::Current, BsTerm::NonCurrent] {
        let mut total = MixedAmount::new();
        let mut seen = false;
        for group in groups.iter().filter(|group| group.term == Some(term)) {
            total = total.ma_add(&group.total)?;
            seen = true;
        }
        // A half with no groups gets no subheading: an empty "Non-current" above
        // an empty subtotal is the sort of blank scaffolding the adaptive rule
        // exists to avoid.
        if seen {
            out.push(BsSubsection {
                term,
                heading: term.heading().to_string(),
                label: format!("Total {} {}", term.heading().to_lowercase(), kind.noun()),
                total,
            });
        }
    }
    Ok(out)
}

/// One group's rows: its members rolled up into their ancestors, clamped to
/// `depth`, with the ancestors that are not the group's own dropped.
///
/// # Which ancestors survive
///
/// Rolling up inside a group synthesizes every ancestor of every member,
/// including ones the group does not own. `assets` appears in the "Cash and cash
/// equivalents" group only because the cash accounts happen to live under it —
/// and the row it produces is a PARTIAL subtotal of a subtree it shares with
/// "Investments", carrying the group's own total and so merely restating the
/// group header. A row is therefore ELIGIBLE only when it is
///
/// - a member itself (a real account with real postings — its `own` is a fact
///   about the journal, whatever else lives beneath it), or
/// - an ancestor no OTHER balance-sheet account sits beneath, which makes its
///   rolled total unambiguously this group's.
///
/// `assets:bank` passes the second test when every account under it is cash;
/// `assets`, `assets:broker` and `assets:broker:taxable` all fail it.
///
/// # Roots outrank the depth clamp
///
/// The group's ROOTS are the eligible rows with no eligible ancestor. Every
/// member sits under exactly one of them, so **the roots always sum to the group
/// total** — and that is why they are emitted whatever `depth` says. Without
/// that exemption a group whose accounts all live deeper than the clamp would
/// expand to nothing at all: `sample.journal`'s securities sit at
/// `assets:broker:taxable:aapl`, four segments down, under a parent shared with
/// the brokerage's cash, so "Investments" would be empty at a depth of 3 while
/// still reporting a five-figure total. `depth` then means "levels within the
/// group", which is what a collapsed group's disclosure triangle implies.
/// `Some(0)` remains hledger's totals-only: no rows at all. `None` is no clamp,
/// which is what the SPA asks for — expanding a group shows all of it.
///
/// Totals are summed over members and never over rows, so nothing here can move
/// a number. `own` reads from the group's members, so a parent shared with
/// another group contributes only what was posted directly to it here.
fn group_rows(
    members: &BTreeMap<String, MixedAmount>,
    sheet: &BTreeMap<String, MixedAmount>,
    depth: Option<usize>,
    flip: bool,
) -> Result<Vec<ReportRow>, ReportError> {
    if depth == Some(0) {
        return Ok(Vec::new());
    }
    let rolled = roll_up(members)?;
    let eligible: BTreeSet<&str> = rolled
        .keys()
        .filter(|account| {
            members.contains_key(*account) || covers_only_members(sheet, members, account)
        })
        .map(String::as_str)
        .collect();
    let kept: Vec<String> = rolled
        .keys()
        .filter(|account| eligible.contains(account.as_str()))
        .filter(|account| {
            depth.is_none_or(|clamp| account.split(':').count() <= clamp)
                || is_group_root(account, &eligible)
        })
        .cloned()
        .collect();

    kept.into_iter()
        .map(|account| {
            let inclusive = rolled.get(&account).cloned().unwrap_or_default();
            let own = members.get(&account).cloned().unwrap_or_default();
            let segments = account.split(':').count();
            Ok(ReportRow {
                account,
                depth: segments,
                own: signed(&own, flip)?,
                inclusive: signed(&inclusive, flip)?,
            })
        })
        .collect()
}

/// True when no proper ancestor of `account` is itself an eligible row — i.e.
/// `account` is one of the group's topmost rows. Walking up by `rfind(':')` is
/// [`AccountGroups::resolve`]'s own loop.
fn is_group_root(account: &str, eligible: &BTreeSet<&str>) -> bool {
    let mut name = account;
    while let Some(cut) = name.rfind(':') {
        name = &name[..cut];
        if eligible.contains(name) {
            return false;
        }
    }
    true
}

/// True when every balance-sheet account at or below `account` is a member of
/// this group — i.e. the group owns that whole subtree.
///
/// The subtree is a contiguous `BTreeMap` range: `account` itself, then its
/// `account:`-prefixed descendants. Taking while the name is still prefixed
/// spans it, and the `:` test then rejects the siblings that merely share the
/// opening letters (`assetsx` for `assets`) — the same walk
/// `resolve_account_type`'s descendant scan uses, and for the same reason.
fn covers_only_members(
    sheet: &BTreeMap<String, MixedAmount>,
    members: &BTreeMap<String, MixedAmount>,
    account: &str,
) -> bool {
    sheet
        .range::<str, _>((Bound::Included(account), Bound::Unbounded))
        .take_while(|(name, _)| name.starts_with(account))
        .filter(|(name, _)| {
            name.len() == account.len() || name.as_bytes().get(account.len()) == Some(&b':')
        })
        .all(|(name, _)| members.contains_key(name))
}

#[cfg(test)]
mod tests {
    use super::super::account_groups::{CASH_GROUP, INVESTMENTS_GROUP};
    use super::super::test_support::{amount, mixed, price, txn, usd};
    use super::*;
    use crate::model::{Amount, Cost, CostKind};
    use crate::reports::mixed_amount::MixedAmount;

    fn sample() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-01-05",
                vec![
                    ("assets:bank:checking", vec![usd(100_000)]),
                    ("equity:opening", vec![usd(-100_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-10",
                vec![
                    ("expenses:food", vec![usd(2500)]),
                    ("liabilities:cc:visa", vec![usd(-2500)]),
                ],
            ),
            txn(
                3,
                "2026-03-15",
                vec![
                    ("assets:bank:savings", vec![usd(50_000)]),
                    ("assets:bank:checking", vec![usd(-50_000)]),
                ],
            ),
            txn(
                4,
                "2026-04-01",
                vec![
                    ("assets:bank", vec![usd(1000)]),
                    ("income:interest", vec![usd(-1000)]),
                ],
            ),
            txn(
                5,
                "2026-07-01",
                vec![
                    ("assets:bank:checking", vec![usd(99_999)]),
                    ("income:salary", vec![usd(-99_999)]),
                ],
            ),
        ]
    }

    fn usd_ma(cents: i128) -> MixedAmount {
        mixed(&[("$", cents, 2)])
    }

    #[test]
    fn assets_and_sign_flipped_liabilities_as_of_inclusive_date() {
        let report = balance_sheet(&sample(), "2026-06-30", 3, &BTreeMap::new()).unwrap();
        assert_eq!(report.as_of.as_deref(), Some("2026-06-30"));
        assert_eq!(
            report
                .sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Assets", "Liabilities"]
        );
        let assets = &report.sections[0];
        assert_eq!(
            assets
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.depth))
                .collect::<Vec<_>>(),
            [
                ("assets", 1),
                ("assets:bank", 2),
                ("assets:bank:checking", 3),
                ("assets:bank:savings", 3),
            ]
        );
        assert_eq!(assets.rows[2].inclusive, usd_ma(50_000)); // checking: 1000 − 500, July excluded
        assert_eq!(assets.rows[3].inclusive, usd_ma(50_000));
        assert_eq!(assets.total, usd_ma(101_000));

        let liabilities = &report.sections[1];
        assert_eq!(
            liabilities
                .rows
                .iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>(),
            ["liabilities", "liabilities:cc", "liabilities:cc:visa"]
        );
        assert_eq!(liabilities.rows[0].inclusive, usd_ma(2500)); // displayed positive
        assert_eq!(liabilities.total, usd_ma(2500));

        assert_eq!(report.grand_total, usd_ma(98_500)); // 1010 − 25
    }

    #[test]
    fn distinguishes_own_from_inclusive() {
        let report = balance_sheet(&sample(), "2026-06-30", 2, &BTreeMap::new()).unwrap();
        let bank = report.sections[0]
            .rows
            .iter()
            .find(|r| r.account == "assets:bank")
            .unwrap();
        assert_eq!(bank.own, usd_ma(1000)); // only the direct $10 posting
        assert_eq!(bank.inclusive, usd_ma(101_000)); // checking + savings + own
        let root = report.sections[0]
            .rows
            .iter()
            .find(|r| r.account == "assets")
            .unwrap();
        assert_eq!(root.own, MixedAmount::new());
        assert_eq!(root.inclusive, usd_ma(101_000));
    }

    #[test]
    fn clamps_to_depth_one() {
        let report = balance_sheet(&sample(), "2026-06-30", 1, &BTreeMap::new()).unwrap();
        assert_eq!(report.sections[0].rows.len(), 1);
        assert_eq!(report.sections[0].rows[0].account, "assets");
        assert_eq!(report.sections[0].rows[0].own, MixedAmount::new());
        assert_eq!(report.sections[0].rows[0].inclusive, usd_ma(101_000));
        assert_eq!(report.sections[1].rows.len(), 1);
        assert_eq!(report.sections[1].rows[0].account, "liabilities");
        assert_eq!(report.sections[1].rows[0].inclusive, usd_ma(2500));
        assert_eq!(report.grand_total, usd_ma(98_500));
    }

    #[test]
    fn empty_sections_before_all_activity() {
        let report = balance_sheet(&sample(), "2025-12-31", 3, &BTreeMap::new()).unwrap();
        assert!(report.sections[0].rows.is_empty());
        assert_eq!(report.grand_total, MixedAmount::new());
    }

    // ---- grouped balance sheet ------------------------------------------

    fn c(symbol: &str) -> Commodity {
        Commodity(symbol.into())
    }

    /// An amount carrying a per-unit (`@`) cost, at any precision.
    fn at_unit(commodity: &str, mantissa: i128, places: u32, cost: Amount) -> Amount {
        let mut a = amount(commodity, mantissa, places);
        a.cost = Some(Box::new(Cost {
            kind: CostKind::Unit,
            amount: cost,
        }));
        a
    }

    /// An amount carrying a per-unit (`@`) cost in whole cents.
    fn at(commodity: &str, mantissa: i128, places: u32, cost_cents: i128) -> Amount {
        at_unit(commodity, mantissa, places, usd(cost_cents))
    }

    /// A tiny journal that balances AT COST: 10 STK bought for $2,000 out of
    /// $10,000 of opening cash, one $250 expense on a credit card, and $1,000 of
    /// revenue banked.
    fn grouped_sample() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-01-01",
                vec![
                    ("assets:bank:checking", vec![usd(1_000_000)]),
                    ("equity:opening", vec![usd(-1_000_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-01",
                vec![
                    ("assets:broker:stk", vec![at("STK", 10, 0, 20_000)]),
                    ("assets:bank:checking", vec![usd(-200_000)]),
                ],
            ),
            txn(
                3,
                "2026-03-01",
                vec![
                    ("expenses:food", vec![usd(25_000)]),
                    ("liabilities:cc:visa", vec![usd(-25_000)]),
                ],
            ),
            txn(
                4,
                "2026-04-01",
                vec![
                    ("assets:bank:checking", vec![usd(100_000)]),
                    ("income:consulting", vec![usd(-100_000)]),
                ],
            ),
        ]
    }

    /// `P 2026-06-30 STK $300.00` — a 50% gain on the $200 basis.
    fn stk_prices() -> Vec<PriceDirective> {
        vec![price("2026-06-30", "STK", amount("$", 30_000, 2))]
    }

    fn grouped(
        value: Valuation,
        depth: Option<usize>,
        groups: &BTreeMap<String, String>,
    ) -> BalanceSheetReport {
        balance_sheet_grouped(
            &grouped_sample(),
            &stk_prices(),
            &BsOpts {
                as_of: "2026-07-01",
                depth,
                value,
                value_in: None,
            },
            &BTreeMap::new(),
            groups,
            &BTreeMap::new(),
        )
        .expect("grouped balance sheet")
    }

    fn group<'a>(report: &'a BalanceSheetReport, section: usize, name: &str) -> &'a BsGroup {
        report.sections[section]
            .groups
            .iter()
            .find(|group| group.name == name)
            .unwrap_or_else(|| panic!("group {name} in section {section}"))
    }

    #[test]
    fn always_three_sections_in_order() {
        let report = grouped(Valuation::Market, Some(3), &BTreeMap::new());
        assert_eq!(
            report
                .sections
                .iter()
                .map(|s| (s.kind, s.title.as_str()))
                .collect::<Vec<_>>(),
            [
                (BsSectionKind::Assets, "Assets"),
                (BsSectionKind::Liabilities, "Liabilities"),
                (BsSectionKind::Equity, "Equity"),
            ]
        );
        assert_eq!(report.as_of, "2026-07-01");
        assert_eq!(report.base, Some(c("$")));
    }

    /// The HTTP layer's admission question, measured over THIS report's rows:
    /// commodities the sheet holds admit themselves, a typo prices nothing and
    /// is refused, and an empty sheet admits vacuously.
    #[test]
    fn prices_any_on_sheet_answers_the_valuation_admission_question() {
        let txns = grouped_sample();
        let prices = stk_prices();
        let admits = |target: &str| {
            prices_any_on_sheet(&txns, &prices, "2026-07-01", &BTreeMap::new(), &c(target))
                .expect("no overflow")
        };
        assert!(admits("$"), "the sheet holds dollars themselves");
        assert!(admits("STK"), "the sheet holds STK themselves");
        assert!(!admits("USDD"), "a typo prices nothing on the sheet");
        assert!(
            prices_any_on_sheet(&[], &prices, "2026-07-01", &BTreeMap::new(), &c("USDD"))
                .expect("no overflow"),
            "an empty sheet admits vacuously — there is nothing to misvalue"
        );
    }

    /// The set is the SHEET's, not the whole journal's: a commodity that
    /// appears only on an income-statement account cannot admit a `valueIn`
    /// the three boxes will never value — the wrong-set failure the holdings
    /// tabs' split admission exists to prevent, measured for this report.
    #[test]
    fn prices_any_on_sheet_ignores_off_sheet_commodities() {
        let txns = vec![txn(
            1,
            "2026-01-01",
            vec![
                ("expenses:fx-fees", vec![amount("EUR", 500, 2)]),
                ("assets:bank:checking", vec![usd(-600)]),
            ],
        )];
        let admits = |target: &str| {
            prices_any_on_sheet(&txns, &[], "2026-07-01", &BTreeMap::new(), &c(target))
                .expect("no overflow")
        };
        assert!(
            !admits("EUR"),
            "EUR lives only on an expense account, which the sheet never values"
        );
        assert!(admits("$"), "the bank balance itself admits the dollar");
    }

    /// Every clamp the report can be asked for, `None` (unclamped) included.
    const DEPTHS: [Option<usize>; 6] = [Some(0), Some(1), Some(2), Some(3), Some(9), None];

    /// The whole point: `A − L − E` is empty on every basis, because the
    /// synthetic equity lines book exactly what valuation moved.
    #[test]
    fn check_is_empty_on_every_basis() {
        for value in [Valuation::Market, Valuation::Cost, Valuation::None] {
            for depth in DEPTHS {
                let report = grouped(value, depth, &BTreeMap::new());
                assert_eq!(
                    report.check,
                    MixedAmount::new(),
                    "check at {value:?}, depth {depth:?}"
                );
                assert!(report.balanced, "balanced at {value:?}, depth {depth:?}");
            }
        }
    }

    /// $10,000 opening − $2,000 spent on stock + $1,000 revenue = $9,000 cash;
    /// 10 STK at $300 = $3,000 (basis $2,000); $250 owed. hledger's own answer
    /// for the same journal shape.
    #[test]
    fn values_assets_at_market_and_books_the_gain_to_equity() {
        let report = grouped(Valuation::Market, Some(3), &BTreeMap::new());
        assert_eq!(group(&report, 0, CASH_GROUP).total, usd_ma(900_000));
        assert_eq!(group(&report, 0, INVESTMENTS_GROUP).total, usd_ma(300_000));
        assert_eq!(report.sections[0].total, usd_ma(1_200_000));
        assert_eq!(report.sections[1].total, usd_ma(25_000)); // displayed positive
        assert_eq!(report.net_worth, usd_ma(1_175_000));

        // Equity: $10,000 contributed + ($1,000 − $250) retained + $1,000 gain.
        assert_eq!(group(&report, 2, "Opening").total, usd_ma(1_000_000));
        assert_eq!(
            group(&report, 2, RETAINED_EARNINGS_GROUP).total,
            usd_ma(75_000)
        );
        assert_eq!(
            group(&report, 2, VALUATION_ADJUSTMENT_GROUP).total,
            usd_ma(100_000)
        );
        assert_eq!(report.sections[2].total, usd_ma(1_175_000));
    }

    /// At cost there is nothing unbooked, so the line is absent entirely —
    /// and the stock is carried at its $2,000 basis.
    #[test]
    fn cost_basis_has_no_valuation_adjustment_line() {
        let report = grouped(Valuation::Cost, Some(3), &BTreeMap::new());
        assert_eq!(group(&report, 0, INVESTMENTS_GROUP).total, usd_ma(200_000));
        assert_eq!(report.sections[0].total, usd_ma(1_100_000));
        assert!(
            !report.sections[2]
                .groups
                .iter()
                .any(|group| group.name == VALUATION_ADJUSTMENT_GROUP),
            "a zero valuation-adjustment line must not be shown"
        );
        assert_eq!(
            report.base, None,
            "cost basis collapses to no one commodity"
        );
    }

    /// Unvalued, the stock is 10 STK; the identity still holds because the
    /// valuation-adjustment line books the cost residue instead.
    #[test]
    fn unvalued_keeps_share_counts() {
        let report = grouped(Valuation::None, Some(3), &BTreeMap::new());
        assert_eq!(
            group(&report, 0, INVESTMENTS_GROUP).total,
            mixed(&[("STK", 10, 0)])
        );
        assert_eq!(report.check, MixedAmount::new());
        assert!(report.meta.unpriced.is_empty(), "nothing is valued at all");
    }

    /// Group and section totals are summed over MEMBERS, so no total may move
    /// with `depth` — only the rows may (RPT-1/RPT-4).
    #[test]
    fn totals_are_depth_independent() {
        let baseline = grouped(Valuation::Market, None, &BTreeMap::new());
        for depth in DEPTHS {
            let report = grouped(Valuation::Market, depth, &BTreeMap::new());
            assert_eq!(
                report.net_worth, baseline.net_worth,
                "net worth at {depth:?}"
            );
            assert_eq!(report.check, baseline.check, "check at {depth:?}");
            for (section, want) in report.sections.iter().zip(&baseline.sections) {
                assert_eq!(
                    section.total, want.total,
                    "{} at depth {depth:?}",
                    section.title
                );
                assert_eq!(
                    section
                        .groups
                        .iter()
                        .map(|g| (g.name.clone(), g.total.clone()))
                        .collect::<Vec<_>>(),
                    want.groups
                        .iter()
                        .map(|g| (g.name.clone(), g.total.clone()))
                        .collect::<Vec<_>>(),
                    "{} groups at depth {depth:?}",
                    section.title
                );
            }
        }
        // ... and the rows really do move, so the test above is not vacuous.
        // `assets` is shared with Investments, so it is not the cash group's own
        // and never appears; `assets:bank` is.
        assert_eq!(
            group(&baseline, 0, CASH_GROUP)
                .rows
                .iter()
                .map(|row| row.account.as_str())
                .collect::<Vec<_>>(),
            ["assets:bank", "assets:bank:checking"]
        );
        assert_eq!(
            grouped(Valuation::Market, Some(2), &BTreeMap::new()).sections[0]
                .groups
                .iter()
                .map(|g| g.rows.len())
                .collect::<Vec<_>>(),
            [1, 1]
        );
        assert!(
            grouped(Valuation::Market, Some(0), &BTreeMap::new()).sections[0]
                .groups
                .iter()
                .all(|g| g.rows.is_empty()),
            "depth 0 is totals-only"
        );
    }

    /// A `bsgroup:` tag overrides every inferred signal, including the Cash type
    /// that would otherwise claim `assets:bank:checking`.
    #[test]
    fn a_bsgroup_tag_overrides_the_inferred_group() {
        let tagged: BTreeMap<String, String> = [
            ("assets:bank".to_string(), "Operating cash".to_string()),
            ("assets:broker".to_string(), "Portfolio".to_string()),
        ]
        .into_iter()
        .collect();
        let report = grouped(Valuation::Market, Some(3), &tagged);
        assert_eq!(
            report.sections[0]
                .groups
                .iter()
                .map(|g| (g.name.as_str(), g.source))
                .collect::<Vec<_>>(),
            // Both tagged, so both sort alphabetically after the built-ins.
            [
                ("Operating cash", GroupSource::Tag),
                ("Portfolio", GroupSource::Tag),
            ]
        );
        // Regrouping must not move a single number.
        let untagged = grouped(Valuation::Market, Some(3), &BTreeMap::new());
        assert_eq!(report.sections[0].total, untagged.sections[0].total);
        assert_eq!(report.check, MixedAmount::new());
    }

    /// A commodity with no `P` directive is left as a share count on the row —
    /// never dropped — and named in `meta.unpriced`.
    #[test]
    fn unpriced_commodities_stay_visible_and_are_reported() {
        let report = balance_sheet_grouped(
            &grouped_sample(),
            &[], // no prices at all: STK cannot be valued
            &BsOpts {
                as_of: "2026-07-01",
                depth: Some(3),
                value: Valuation::Market,
                value_in: Some(c("$")),
            },
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(report.meta.unpriced, vec![c("STK")]);
        assert_eq!(
            group(&report, 0, INVESTMENTS_GROUP).total,
            mixed(&[("STK", 10, 0)]),
            "the shares stay on the row rather than vanishing"
        );
        assert_eq!(report.check, MixedAmount::new());
    }

    /// An untypeable account belongs to NO section, so the three boxes really
    /// do not add up — and the check says so instead of hiding it.
    #[test]
    fn check_flags_an_account_that_lands_in_no_section() {
        let mut txns = grouped_sample();
        txns.push(txn(
            5,
            "2026-05-01",
            vec![
                ("mystery:pot", vec![usd(5000)]),
                ("assets:bank:checking", vec![usd(-5000)]),
            ],
        ));
        let report = balance_sheet_grouped(
            &txns,
            &stk_prices(),
            &BsOpts {
                as_of: "2026-07-01",
                depth: Some(3),
                value: Valuation::Cost,
                value_in: None,
            },
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(report.check, usd_ma(-5000));
        assert!(
            !report.balanced,
            "an unclassifiable account is 50 dollars, not dust — RPT-1 must still fire"
        );
    }

    // ---- the balanced verdict --------------------------------------------

    /// $10,000 of opening cash, then `1.234 STK @ $302.4567` — a cost of
    /// `$373.2315678` — settled with `cash_cents` of cash.
    ///
    /// The cash leg can only be written to the cent, so the five surplus digits
    /// the multiplication produced cannot be cancelled by anything a journal is
    /// able to say. They are real and exact: hledger's own `bal -B` carries the
    /// same `$0.0015678` (verified against 1.52, forced to eight decimals with
    /// `-c '$1000.00000000'`) and merely hides it behind two-decimal display,
    /// while `hledger check` passes because its balance test tolerates half a
    /// unit at the precision the entry was WRITTEN at.
    ///
    /// This shape is what made a valid journal report "assets − liabilities −
    /// equity should be zero, but it is $0.00227970".
    fn dusty(cash_cents: i128) -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-01-01",
                vec![
                    ("assets:bank:checking", vec![usd(1_000_000)]),
                    ("equity:opening", vec![usd(-1_000_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-01",
                vec![
                    (
                        "assets:broker:stk",
                        vec![at_unit("STK", 1234, 3, amount("$", 3_024_567, 4))],
                    ),
                    ("assets:bank:checking", vec![usd(cash_cents)]),
                ],
            ),
        ]
    }

    fn dust_report(txns: &[Transaction]) -> BalanceSheetReport {
        balance_sheet_grouped(
            txns,
            &[],
            &BsOpts {
                as_of: "2026-07-01",
                depth: None,
                value: Valuation::Cost,
                value_in: None,
            },
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("grouped balance sheet")
    }

    #[test]
    fn sub_cent_cost_dust_is_balanced_but_still_reported_exactly() {
        let report = dust_report(&dusty(-37_323));
        assert_eq!(
            report.check,
            mixed(&[("$", 15678, 7)]),
            "the exact residual is preserved for display"
        );
        assert!(!report.check.is_zero(), "and it really is non-zero");
        assert!(
            report.balanced,
            "$0.0015678 is under one cent, so it cannot be a posting"
        );
    }

    /// The boundary is the wider of one written unit and one hundredth. The
    /// dollar postings here are written to two places, so both readings give the
    /// same `$0.01`, and two ADJACENT cents of settlement straddle it.
    #[test]
    fn one_unit_of_the_commoditys_own_precision_is_the_boundary() {
        // Settled a cent high: still dust, and the largest residual that is.
        let just_under = dust_report(&dusty(-37_324));
        assert_eq!(just_under.check, mixed(&[("$", -84_322, 7)]));
        assert!(just_under.balanced, "$0.0084322 < $0.01");

        // The next cent the other way: $0.0115678, which a posting could carry.
        let just_over = dust_report(&dusty(-37_322));
        assert_eq!(just_over.check, mixed(&[("$", 115_678, 7)]));
        assert!(!just_over.balanced, "$0.0115678 >= $0.01");
    }

    /// **Regression.** Writing dollars more finely than the cent must not tighten
    /// the threshold. This is the shape that made the shipped warning fire on a
    /// journal hledger accepts: the extra entry balances exactly, so it moves the
    /// PRECISION without moving the residual by a digit, and the report flipped
    /// to "not balanced" anyway. `fixtures/reports/bs-cent-floor.journal` is the
    /// same story end to end.
    #[test]
    fn a_finer_written_precision_cannot_tighten_the_threshold_below_a_cent() {
        for places in [3, 4, 8] {
            let mut txns = dusty(-37_323);
            txns.push(txn(
                3,
                "2026-03-01",
                vec![
                    ("assets:bank:checking", vec![amount("$", 1234, places)]),
                    ("equity:opening", vec![amount("$", -1234, places)]),
                ],
            ));
            let report = dust_report(&txns);
            assert_eq!(
                report.check,
                mixed(&[("$", 15678, 7)]),
                "same residual at {places} places"
            );
            assert!(
                report.balanced,
                "$0.0015678 is under a cent whatever else the journal writes \
                 ({places} places)"
            );
        }
    }

    /// A precision table claiming `places` for `$` and knowing nothing else.
    fn dollars_written_at(places: u32) -> WrittenPrecision {
        WrittenPrecision {
            posted: [(c("$"), places)].into_iter().collect(),
            costed: BTreeMap::new(),
        }
    }

    /// The whole rule, tabulated: `tolerance = max(10^-p, 0.01)`, strictly.
    ///
    /// Both halves matter. A commodity written COARSELY keeps its wider
    /// threshold — whole shares are not denominated in cents, which is why the
    /// floor is a floor and not a replacement. A commodity written FINELY is
    /// lifted to a cent and no further.
    #[test]
    fn the_tolerance_is_the_wider_of_one_written_unit_and_one_hundredth() {
        // (places written, places the threshold actually lands at)
        for (written, threshold) in [(0, 0), (1, 1), (2, 2), (3, 2), (4, 2), (8, 2)] {
            let precision = dollars_written_at(written);
            // Residuals stated at a scale finer than either, so "one ulp under"
            // is expressible for every row.
            let unit = 10_i128.pow(9 - threshold);
            assert!(
                is_balanced(&mixed(&[("$", unit - 1, 9)]), &precision),
                "one ulp under the threshold, p={written}"
            );
            assert!(
                !is_balanced(&mixed(&[("$", unit, 9)]), &precision),
                "exactly the threshold is NOT dust, p={written}"
            );
        }
    }

    /// A cost annotation must NOT set the tolerance. `$302.4567` is four places,
    /// but it is a price, not a posting — measuring dollars against it would
    /// re-flag the very dust that multiplying by it created.
    #[test]
    fn a_cost_annotations_places_do_not_tighten_the_tolerance() {
        let precision = WrittenPrecision::scan(&dusty(-37_323), "2026-07-01");
        assert_eq!(precision.of(&c("$")), 2, "from the cash postings");
        assert_eq!(precision.of(&c("STK")), 3);
        assert_eq!(
            precision.costed.get(&c("$")).copied(),
            Some(4),
            "the price's own places are recorded, just not used here"
        );
        // A commodity the window never mentions falls back to whole units.
        assert_eq!(precision.of(&c("NOPE")), 0);
    }

    /// Postings after `as_of` are not summed into `check`, so they must not set
    /// its tolerance either. Asserted on the scan rather than on the verdict:
    /// past two places the floor makes the difference invisible to `balanced`,
    /// but the scan is what a coarser commodity's threshold still rides on.
    #[test]
    fn the_precision_scan_respects_the_as_of_window() {
        let mut txns = dusty(-37_323);
        txns.push(txn(
            3,
            "2026-12-01",
            vec![
                ("assets:bank:checking", vec![amount("$", 1234, 3)]),
                ("equity:opening", vec![amount("$", -1234, 3)]),
            ],
        ));
        assert_eq!(
            WrittenPrecision::scan(&txns, "2026-07-01").of(&c("$")),
            2,
            "December's third place is out of the window"
        );
    }

    /// **The hypothesis this refutes.** A reverse price edge makes `PriceGraph`
    /// divide, and `1/3` does not terminate, so the rate is rounded. That still
    /// cannot leave residue in `check`: the three sections partition exactly the
    /// map the valuation adjustment sums, `Dec` addition is exact, and
    /// multiplication by the one rounded rate distributes over it — so valuing
    /// per account and summing equals valuing the sum, term for term.
    ///
    /// Here EUR is priced in `$` and STK is priced in EUR at a third of a EUR,
    /// so valuing STK into `$` needs a two-hop chain, and the dollar leg has to
    /// be valued through the REVERSE of the EUR price.
    #[test]
    fn a_rounded_cross_rate_leaves_no_residue() {
        let txns = vec![
            txn(
                1,
                "2026-01-01",
                vec![
                    ("assets:bank:checking", vec![usd(1_000_000)]),
                    ("equity:opening", vec![usd(-1_000_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-01",
                vec![
                    ("assets:broker:stk", vec![at("STK", 7, 0, 30_000)]),
                    ("assets:bank:checking", vec![usd(-210_000)]),
                ],
            ),
            txn(
                3,
                "2026-03-01",
                vec![
                    ("assets:bank:eur", vec![at("EUR", 100_000, 2, 117)]),
                    ("assets:bank:checking", vec![usd(-117_000)]),
                ],
            ),
        ];
        let prices = vec![
            // 1 EUR = $1.17, whose reciprocal (0.8547008547…) does not terminate.
            price("2026-06-30", "EUR", amount("$", 117, 2)),
            // 1 STK = 1/3 EUR, so STK → $ is a two-hop chain over a rate that
            // cannot be written exactly at any precision.
            price("2026-06-30", "STK", amount("EUR", 3_333_333_333, 10)),
        ];
        for value in [Valuation::Market, Valuation::Cost, Valuation::None] {
            for value_in in [None, Some(c("$")), Some(c("EUR")), Some(c("STK"))] {
                let report = balance_sheet_grouped(
                    &txns,
                    &prices,
                    &BsOpts {
                        as_of: "2026-07-01",
                        depth: None,
                        value,
                        value_in: value_in.clone(),
                    },
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                )
                .unwrap();
                assert_eq!(
                    report.check,
                    MixedAmount::new(),
                    "check at {value:?} into {value_in:?} — valuation cannot leave dust"
                );
                assert!(report.balanced);
            }
        }
    }

    #[test]
    fn under_one_unit_is_strict_and_survives_extreme_scales() {
        // Two places: a cent is the unit.
        assert!(under_one_unit(Dec::new(999_999, 8), 2)); // $0.00999999
        assert!(!under_one_unit(Dec::new(1_000_000, 8), 2)); // exactly $0.01
        assert!(under_one_unit(Dec::new(0, 8), 2));
        // Whole units (a share count): anything fractional is dust, 1 is not.
        assert!(under_one_unit(Dec::new(9999, 4), 0));
        assert!(!under_one_unit(Dec::new(10_000, 4), 0));
        // Residual COARSER than the commodity's precision: only exact zero.
        assert!(under_one_unit(Dec::new(0, 0), 2));
        assert!(!under_one_unit(Dec::new(1, 0), 2));
        assert!(!under_one_unit(Dec::new(-1, 1), 2));
        // Past i128's reach, one unit exceeds every representable mantissa.
        assert!(under_one_unit(Dec::new(i128::MAX, 60), 2));
        assert!(under_one_unit(Dec::new(i128::MIN, 60), 2));
    }
}
