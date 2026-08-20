// Report engine result shapes (WP-06). Pure TS: no Svelte/DOM imports —
// this module ports to Rust in a later phase.
//
// Sign conventions (matching hledger's bs/is presentation, verified against
// fixtures/golden/): sections whose accounts are negative internally
// (liabilities, revenues) are presented sign-flipped, so a healthy report
// shows positive numbers everywhere. Grand totals are nets:
//   balance sheet:     grandTotal = assets − liabilities(displayed)
//   income statement:  grandTotal = revenues(displayed) − expenses
// PeriodReport values (cash flow, net worth) keep natural signs.

import type {MixedAmount} from "../domain/money";
import type {ISODate} from "../domain/types";

export interface ReportRow {
    account: string;
    /** Number of `:`-separated segments in `account`. */
    depth: number;
    /** Direct total of postings to exactly this (clamped) account name. */
    own: MixedAmount;
    /** Rolled-up total including all sub-accounts. */
    inclusive: MixedAmount;
}

export interface Section {
    title: string;
    rows: ReportRow[];
    total: MixedAmount;
}

/**
 * The flat `hledger bs`/`is` lookalike. No longer rendered by either tab — both
 * moved to their own grouped shapes below — but still the type the hledger
 * parity goldens decode into, so it stays exactly as it is.
 *
 * `asOf` for point-in-time, `from`/`to` for ranges (all inclusive).
 */
export interface SectionedReport {
    asOf?: ISODate;
    from?: ISODate;
    to?: ISODate;
    sections: Section[];
    grandTotal: MixedAmount;
}

/** Extra result info (contract extension, see plans/06-reports-engine.md). */
export interface ReportMeta {
    /** Commodities skipped during valuation because no direct price to the target existed (sorted, deduped). */
    unpriced: string[];
}

// --- Grouped balance sheet (plans/12-balance-sheet-redesign.md) --------------
// The market-valued, three-box balance sheet. Structurally distinct from
// `SectionedReport` (which still backs the income statement and the hledger
// parity golden): every line here is ONE number in `base`, accounts are bucketed
// into named GROUPS, and the report carries its own integrity check.
//
// `kind` is added by the decoder, not by the engine — the wire has no such
// field. It exists because `SectionedReport` and `BalanceSheetReport` both have
// a `sections` array, so shape alone cannot tell them apart, and FE-1 was
// exactly that mistake made once already.

/** Which of the three boxes a section is. Fixed set, always all three, in this order. */
export type BsSectionKind = "assets" | "liabilities" | "equity";

/**
 * How a group got its name — the resolution step that matched, first-wins:
 * an explicit grouping tag (`bsgroup:` / `isgroup:`), the account's effective
 * type, the presence of a non-base commodity, a path segment, or a synthetic
 * line the engine computed (Retained earnings / Valuation adjustment).
 *
 * ONE vocabulary for both statements, because there is one resolver behind them
 * (`account_groups.rs`, widened to take a configurable tag name). Two copies
 * would let the balance sheet's badge and the income statement's disagree about
 * what "segment" means, which is the kind of drift the wire has no way to catch.
 */
export type GroupSource = "tag" | "type" | "commodity" | "segment" | "computed";

/** Which valuation produced the figures. Mirrors the `value=` query parameter. */
export type BsValuation = "market" | "cost" | "none";

/**
 * Where a group sits on the standard current / non-current axis.
 *
 * A CODE, not prose, and for the same reason `IsSectionKind` is one: it mirrors
 * the `bsterm:` tag, and a classification that decides which subtotal a balance
 * lands under must never be a match against English words
 * ([[account-type-not-name]]). The prose a reader sees — "Current", "Total
 * non-current assets" — is `BsSubsection`'s, supplied by the engine.
 */
export type BsTerm = "current" | "noncurrent";

/**
 * One current/non-current band inside a section: what to head it with, what to
 * call its subtotal, and the engine's subtotal.
 *
 * `heading` and `label` are ENGINE-SUPPLIED strings, deliberately, even though
 * both are derivable from `term` plus the section title. Deriving them would put
 * the same term→prose mapping in the view AND in the xlsx export, which is the
 * duplication (DRY-3) that has already bitten this repo — and the one place the
 * two copies would first disagree is the moment a section is renamed.
 *
 * `total` is likewise the engine's, summed over MEMBERS. Never re-add the group
 * lines to make one: they are rounded for display and their exact sum is what
 * this field already is.
 */
export interface BsSubsection {
    term: BsTerm;
    /** The subheading printed above the band's first group, e.g. "Non-current". */
    heading: string;
    /** The band's subtotal label, e.g. "Total non-current assets". */
    label: string;
    total: MixedAmount;
}

/** One named bucket of accounts within a section, with its own subtotal. */
export interface BsGroup {
    name: string;
    source: GroupSource;
    /**
     * Which band this group belongs to, or null when nothing in the journal
     * claims one — the adaptive default, and the case that must render exactly
     * as it did before the axis existed.
     *
     * Guaranteed by the engine when the group's section has `subsections`: every
     * group carries a non-null term, and groups sharing a term are CONTIGUOUS,
     * current first. That ordering is what lets one pass over the groups decide
     * where a heading opens and a subtotal closes.
     */
    term: BsTerm | null;
    /**
     * The depth-clamped member rows, ancestors included and lexically sorted
     * (as every other report's rows are), so `compressSectionRows` applies.
     * EMPTY for a computed group — "Retained earnings" summarizes accounts that
     * are not on the balance sheet at all, so it has a total and no rows.
     */
    rows: ReportRow[];
    /** Summed over MEMBERS, not over displayed rows, so it is depth-independent (RPT-1/RPT-4). */
    total: MixedAmount;
}

/** One of the three boxes. `total` sums its groups' members. */
export interface BsSection {
    kind: BsSectionKind;
    title: string;
    groups: BsGroup[];
    /**
     * The current/non-current bands to print inside this box, in visual order.
     *
     * EMPTY is the adaptive default and the common case: a journal that carries
     * no `bsterm:` tag gets no subheadings, no band subtotals, and a box
     * identical to the one it got before the axis existed. Always empty on the
     * equity section — equity is not split by term.
     *
     * A term listed here always has at least one group, so a heading can never
     * be printed over nothing.
     */
    subsections: BsSubsection[];
    total: MixedAmount;
}

/** The grouped, valued balance sheet: three sections, a net worth, and an integrity check. */
export interface BalanceSheetReport {
    /** Decoder-applied discriminator (see the note above); never on the wire. */
    kind: "balanceSheet";
    asOf: ISODate;
    /** The commodity every line is valued in, or null when the journal has no base commodity. */
    base: string | null;
    value: BsValuation;
    sections: BsSection[];
    /** assets − liabilities. */
    netWorth: MixedAmount;
    /**
     * assets − liabilities − equity, EXACTLY: the engine computes it from `Dec`
     * values and never rounds it. It must be shown, never swallowed.
     *
     * It is NOT the verdict — read `balanced` for that. A journal holding
     * fractional lots leaves real residue here with nothing wrong: a priced
     * posting is worth `quantity × price` at cost, and that product carries more
     * decimal places than the cash leg paying for it can be written to.
     */
    check: MixedAmount;
    /**
     * Whether `check` is arithmetic dust rather than an imbalance — true when
     * every commodity's residual is strictly under `max(10^-p, 0.01)`, where `p`
     * is the widest precision the journal writes for that commodity. The
     * one-hundredth floor is deliberate product policy: the balance sheet
     * ignores imbalances below one cent.
     *
     * The engine decides this once and every consumer must render its ✓/✗ from
     * it. Re-deriving it locally (`maIsZero(check)`) is what made a valid
     * journal report "should be zero, but it is $0.00227970".
     */
    balanced: boolean;
    /** Present only when something noteworthy happened (commodities with no price to `base`). */
    meta?: ReportMeta;
}

// --- Grouped income statement (plans/13-income-statement-redesign.md) --------
// The market-valued, adaptive-GAAP income statement. Same redesign as the
// balance sheet one section up, and deliberately the same shapes where they
// mean the same thing (`GroupSource`, `ReportMeta`, the decoder-applied `kind`).
//
// Three things make it structurally different from `BsSection`:
//
//   1. Every figure is an `Amounts`, not a `MixedAmount` — the report carries a
//      prior period beside the current one.
//   2. A section can trail SUBTOTALS (Gross profit, EBITDA, …), which print
//      between the boxes rather than inside them.
//   3. Rows carry one rolled-up figure, not the `own`/`inclusive` pair, so
//      chain compression tests amounts rather than `own` (`compressIsRows`).
//
// `kind` is added by the decoder and is not on the wire. THREE report types now
// carry a `sections` array (this one, `SectionedReport`, `BalanceSheetReport`),
// so shape alone has never been further from telling them apart — FE-1's
// failure mode, two shapes on.

/** An inclusive date window. Used for the prior comparison period's own dates. */
export interface DateRange {
    from: ISODate;
    to: ISODate;
}

/**
 * One figure, with the comparison period's beside it.
 *
 * `prior` is OPTIONAL, and absent — not null, and never a zero — when the report
 * is not comparing. A zero would be a claim about a period that was never
 * computed; `report.prior` (the window) says definitively which case a report is
 * in, and the decoder cross-checks every `Amounts` against it.
 *
 * The prior/current join happens in RUST, over the union of section/group/
 * account keys, so a line present in only one period arrives with an explicit
 * empty amount on the other side rather than being dropped. Doing that join
 * here would be exactly the sort of key-matching that silently loses rows.
 */
export interface Amounts {
    current: MixedAmount;
    /** The prior window's figure. Absent unless `IncomeStatementReport.prior` is set. */
    prior?: MixedAmount;
}

/**
 * Which box a section is. Closed, coded vocabulary — it mirrors the `issection:`
 * tag, which is a CODE rather than prose for the reason recorded in
 * [[account-type-not-name]]: a classification that decides membership must never
 * match English words, because the failure mode is a section that reads zero.
 *
 * Not every kind appears in every report: a section with no members is omitted
 * entirely, so an untagged personal journal yields exactly `revenue` + `opex`.
 */
export type IsSectionKind = "revenue" | "cogs" | "opex" | "depreciation" | "interest" | "tax" | "other";

/**
 * A rung of the GAAP subtotal ladder. Each is emitted only when the sections it
 * needs exist, so a journal that never asked for one never sees it.
 *
 * EBITDA sits above D&A and Operating income below it, which makes every rung a
 * running total of everything printed above it — no line is ever the sum of
 * things both above and below it.
 */
export type IsSubtotalKind = "grossProfit" | "ebitda" | "operatingIncome" | "pretaxIncome";

/** One account inside a group. `amounts` is the subaccount-INCLUSIVE roll-up. */
export interface IsRow {
    account: string;
    /** Number of `:`-separated segments in `account`. */
    depth: number;
    amounts: Amounts;
}

/** One named line within a section, with its own subtotal. */
export interface IsGroup {
    name: string;
    source: GroupSource;
    /** Member rows, ancestors included and lexically sorted, so chain compression applies. */
    rows: IsRow[];
    /** Summed over MEMBERS, never over displayed rows, so collapsing a group cannot change it. */
    total: Amounts;
}

/**
 * A ruled ladder line. It hangs off the section it FOLLOWS (`IsSection.trailing`)
 * rather than floating in a list of its own, so a subtotal can never be orphaned
 * from the box it summarizes — or survive that box being omitted.
 */
export interface IsSubtotal {
    kind: IsSubtotalKind;
    label: string;
    total: Amounts;
}

/**
 * One box. Figures arrive DISPLAY-SIGNED: the engine has already flipped the
 * sections that are negative internally, so revenue and every cost section read
 * positive here and nothing on this side of the wire negates anything.
 *
 * `other` is the exception, and deliberately: a grant and a lawsuit settlement
 * can share it, so it is presented as a net contribution to income and is
 * allowed to print negative.
 */
export interface IsSection {
    kind: IsSectionKind;
    title: string;
    groups: IsGroup[];
    total: Amounts;
    /** Ladder lines printed below this box. Usually empty; never null. */
    trailing: IsSubtotal[];
}

/** The grouped, valued income statement: ladder-ordered boxes and a bottom line. */
export interface IncomeStatementReport {
    /** Decoder-applied discriminator (see the note above); never on the wire. */
    kind: "incomeStatement";
    from: ISODate;
    to: ISODate;
    /**
     * The window the `prior` figures cover — the immediately preceding window of
     * EQUAL LENGTH — or null when not comparing.
     *
     * Each period is valued at ITS OWN period end, matching `hledger is -V` run
     * over that range. That makes the change column noisier than constant-currency
     * would, and is the right trade: the prior column agrees with the report you
     * actually ran last year.
     */
    prior: DateRange | null;
    /** The commodity every line is valued in, or null when the journal has no base commodity. */
    base: string | null;
    value: BsValuation;
    /** Non-empty sections only, in ladder order. */
    sections: IsSection[];
    /** The bottom line, display-signed (positive = a profit). */
    netIncome: Amounts;
    /**
     * Whether any member resolved to a section other than revenue/opex — i.e.
     * whether the journal asked for the GAAP ladder at all.
     *
     * It is the engine's, not re-derived from `sections.length`: it also decides
     * whether `opex` is titled "Expenses" or "Operating expenses", and a client
     * guessing that would relabel a box the engine had already named.
     */
    multiStep: boolean;
    /** Present only when something noteworthy happened (commodities with no price to `base`). */
    meta?: ReportMeta;
}

/** Cash flow / net worth: one column per bucket, oldest → newest. */
export interface PeriodReport {
    buckets: string[];
    rows: {account: string; depth: number; values: MixedAmount[]}[];
    totals: MixedAmount[];
    /** Present only when something noteworthy happened (e.g. unpriced commodities in netWorth). */
    meta?: ReportMeta;
}

// Budget report (actuals vs. `~` periodic-rule goals). Structurally a period
// report whose cells are two-valued: `actual` and, when the account is part of
// the selected goal tree, `goal` (null for `<unbudgeted>` and non-budgeted
// accounts — kept distinct from an all-zero `{}` goal). The `kind` tag
// discriminates it from PeriodReport (both carry buckets/rows/totals).

/** One account × bucket cell: the actual balance and, when budgeted, its goal. */
export interface BudgetCell {
    actual: MixedAmount;
    /** Subaccount-inclusive goal, or null when the account has no goal (e.g. `<unbudgeted>`). */
    goal: MixedAmount | null;
}

/** One budget row: an account and its per-bucket cells (parallel to `buckets`). */
export interface BudgetRow {
    account: string;
    /** Number of `:`-separated segments in `account`. */
    depth: number;
    cells: BudgetCell[];
}

/** Budget report: bucket keys (oldest → newest), rows, and a grand-total cell per bucket. */
export interface BudgetReport {
    kind: "budget";
    buckets: string[];
    rows: BudgetRow[];
    totals: BudgetCell[];
}
