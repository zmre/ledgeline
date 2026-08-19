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

/** Balance sheet / income statement. `asOf` for point-in-time, `from`/`to` for ranges (all inclusive). */
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
 * an explicit `bsgroup:` tag, the account's effective type, the presence of a
 * non-base commodity, the account's second path segment, or a synthetic line
 * the engine computed (Retained earnings / Valuation adjustment).
 */
export type BsGroupSource = "tag" | "type" | "commodity" | "segment" | "computed";

/** Which valuation produced the figures. Mirrors the `value=` query parameter. */
export type BsValuation = "market" | "cost" | "none";

/** One named bucket of accounts within a section, with its own subtotal. */
export interface BsGroup {
    name: string;
    source: BsGroupSource;
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
