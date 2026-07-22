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
