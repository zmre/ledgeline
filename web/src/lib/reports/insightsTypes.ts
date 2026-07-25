// Insights dashboard result shapes — the domain mirror of the Rust
// `reports::insights` engine (crates/ledgeline-core/src/reports/insights.rs),
// decoded by nativeDecode.ts. Pure TS: no Svelte/DOM imports.
//
// A dashboard compares a CURRENT period against the PREVIOUS one. The engine is
// handed an inclusive `[start, end]` span and splits it at its midpoint, so the
// previous period is `[start, mid]` and the current period is `[currStart, end]`.
// Money is exact (Dec/MixedAmount); percents are display-boundary numbers.

import type {Dec, MixedAmount} from "../domain/money";
import type {ISODate} from "../domain/types";

/** The resolved comparison window: the whole span plus its midpoint split. */
export interface InsightsPeriod {
    start: ISODate;
    mid: ISODate;
    end: ISODate;
    prevStart: ISODate;
    prevEnd: ISODate;
    currStart: ISODate;
    currEnd: ISODate;
}

/** A metric's current + previous value, the exact change, and a base-commodity percent. */
export interface MetricDelta {
    current: MixedAmount;
    previous: MixedAmount;
    /** Exact `current − previous`. */
    delta: MixedAmount;
    /** Base-commodity percent change; null when the previous base value is absent or zero. */
    pct: number | null;
}

/** Average monthly cost of living: per-period totals + month counts (averaged for display). */
export interface CostOfLiving {
    currentTotal: MixedAmount;
    previousTotal: MixedAmount;
    monthsCurrent: number;
    monthsPrevious: number;
}

/** One period's portfolio performance in the base commodity. */
export interface PerfPoint {
    /** `marketValue(end) − marketValue(start)`; null for an unpriced/empty portfolio. */
    gain: Dec | null;
    /** `gain / marketValue(start) × 100`; null when the start value is zero/absent. */
    gainPct: number | null;
}

/** Investment performance for both periods (Box 5). */
export interface InvestmentPerf {
    current: PerfPoint;
    previous: PerfPoint;
}

/**
 * How a leaf account changed. Categories with NO previous-period activity are
 * not reported at all (nothing to compare against), so there is no "new" kind.
 */
export type ChangeKind = "changed" | "ended";

/** One leaf-account change between the two periods (Boxes 7 & 9), base commodity, display-signed. */
export interface ChangeRow {
    account: string;
    current: Dec;
    previous: Dec;
    /** Exact `current − previous`. */
    delta: Dec;
    /** Percent change; null for a brand-new category. */
    pct: number | null;
    kind: ChangeKind;
}

/** One stock's percent move over the current period (Box 8). */
export interface MoverRow {
    symbol: string;
    name: string;
    /** Windowed dollar gain over the current period (base commodity). */
    gain: Dec | null;
    /** Windowed percent move over the current period. */
    gainPct: number | null;
    /**
     * The position had no market price at the START of the window, so its
     * baseline fell back to the purchase cost — the "move" then approximates the
     * all-time gain since purchase rather than a true period return.
     */
    startEstimated: boolean;
}

/** How often a detected subscription recurs. */
export type Cadence = "monthly" | "annual";

/** One recurring charge inferred from the journal's expense history. */
export interface Subscription {
    /** Payee as written in the journal (description before `|`). */
    payee: string;
    cadence: Cadence;
    /** The representative charge (median of the matched cluster). */
    typicalAmount: Dec;
    /** Cost per year: `typical × 12` monthly, `typical` annual. */
    annualizedCost: Dec;
    occurrences: number;
    firstSeen: ISODate;
    lastSeen: ISODate;
    /** Next charge projected from `lastSeen`. */
    nextExpected: ISODate;
    /** Expense accounts the charges posted to. */
    accounts: string[];
    /** Hand-added via a `subscription:true` tag rather than detected. */
    manual: boolean;
}

/**
 * Detected subscriptions, split by cadence and sorted by annual cost desc.
 * Deliberately independent of the dashboard's comparison period — it always
 * scans a trailing window ending at `asOf`.
 */
export interface SubscriptionsReport {
    asOf: ISODate;
    lookbackStart: ISODate;
    monthly: Subscription[];
    annual: Subscription[];
}

/** One of the largest transactions in the current period (Box 10). */
export interface TopTxn {
    index: number;
    date: ISODate;
    description: string;
    /** Base-commodity magnitude of money moved. */
    amount: Dec;
}

/** The Insights dashboard: period-over-period core metrics (Boxes 1–6) + list boxes (7–10). */
export interface InsightsReport {
    period: InsightsPeriod;
    /** Base commodity symbol used for percent changes and headline figures. */
    base: string;
    /**
     * Date of the journal's earliest transaction. When it falls inside the
     * previous period, that period covers less time than the current one and
     * every delta overstates growth — the dashboard says so rather than
     * presenting a half-covered baseline as a real comparison.
     */
    journalStart: ISODate | null;
    revenue: MetricDelta;
    expenses: MetricDelta;
    netWorth: MetricDelta;
    costOfLiving: CostOfLiving;
    investment: InvestmentPerf;
    cashBalance: MetricDelta;
    expenseChanges: ChangeRow[];
    revenueChanges: ChangeRow[];
    movers: MoverRow[];
    topTxns: TopTxn[];
}
