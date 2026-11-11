// Startup-metrics result shapes (plans/37): the domain mirror of the Rust
// `reports::saas` engine (crates/ledgeline-core/src/reports/saas.rs), served by
// `GET /api/saas` and decoded by nativeDecode.ts `decodeSaasReport`. Pure TS:
// relative imports only (reports/purity.test.ts).
//
// The wire omits every `Option` that is `None`; the decoder turns each absence
// into `null` here, so "the journal can't support this figure" is an explicit
// value a component tests for, never a missing key. Money is exact (Dec), in the
// base commodity. Percentages are numbers in percent units (62.5 means 62.5%);
// ratios are plain numbers.

import type {Dec} from "../domain/money";
import type {ISODate} from "../domain/types";
import type {InsightsPeriod} from "./insightsTypes";

/** Why the customer tier is hidden even though recurring revenue exists. */
export type CustomersHiddenReason = "no-customer-tags" | "low-coverage" | "billing-clearing";

export const CUSTOMERS_HIDDEN_REASONS: readonly CustomersHiddenReason[] = ["no-customer-tags", "low-coverage", "billing-clearing"];

/** Which tiers of the startup metrics the journal supports. */
export interface SaasAvailable {
    /** Some account resolves to `kpi: recurring` and carries revenue in the span. */
    recurring: boolean;
    /** ≥ 90% of recurring revenue is attributed to a customer (and not billing-clearing books). */
    customers: boolean;
    /** Any account declares a `kpi:` tag. */
    business: boolean;
    /** Some account resolves to `issection: cogs`. */
    cogs: boolean;
    /** Dimension keys offered as breakdowns, in request order. */
    dimensions: string[];
    customersHiddenReason: CustomersHiddenReason | null;
    /** 0–100; present whenever `recurring` is. */
    coveragePct: number | null;
    /** Some customers were attributed from payees rather than tags. */
    inferredCustomers: boolean;
    /**
     * Per-customer MRR is hidden but customers were billed (Amendment A1.2):
     * each half then carries {@link PeriodMetrics.billingCustomers}.
     */
    billingCustomers: boolean;
    /**
     * The data horizon (A2), `YYYY-MM`: the last month with any recurring
     * revenue recorded on or before the span's end. Later months are `open`
     * and no metric uses them. Present whenever `recurring` is.
     */
    closedThrough: string | null;
}

/** What the half's runway and cash-out date were computed from (Amendment A1.1). */
export type RunwayBasis = "adjusted" | "cash";

export const RUNWAY_BASES: readonly RunwayBasis[] = ["adjusted", "cash"];

/**
 * Customer counts from billings (Amendment A1.2), when per-customer MRR is
 * hidden: a customer is active in month m when billed in months m−11 … m.
 */
export interface BillingCustomers {
    /** Customers billed in the 12 months to the half's latest complete month. */
    customers: number;
    customerGrowthPct: number | null;
    /** First-ever billing in the half. */
    newLogos: number;
    /** Went 12 months without a billing during the half. */
    lapsedLogos: number;
    logoChurnPct: number | null;
    /** Trailing-12-month billings ÷ billed customers. */
    acv: Dec | null;
    /** The journal starts less than 12 months before the half's month: counts may be low. */
    windowShort: boolean;
    /** Some of the counted customers were named from payees. */
    inferred: boolean;
}

/** The MRR bridge for one month or summed over a half. `other` absorbs unattributed revenue and negative (credit) months. */
export interface Bridge {
    new: Dec;
    expansion: Dec;
    contraction: Dec;
    churn: Dec;
    reactivation: Dec;
    other: Dec;
    newLogos: number;
    churnedLogos: number;
    reactivatedLogos: number;
}

/** The five per-customer bridge categories (Other has no customers). */
export type BridgeCategory = "new" | "expansion" | "contraction" | "churn" | "reactivation";

export const BRIDGE_CATEGORIES: readonly BridgeCategory[] = ["new", "expansion", "contraction", "churn", "reactivation"];

/** One calendar month of the span. */
export interface SaasMonth {
    /** `YYYY-MM`. */
    month: string;
    /** Only partly inside the span: computed, never a period's "latest complete month". */
    partial: boolean;
    /** After the data horizon (A2): started, but its recurring revenue isn't booked yet. Always also `partial`. */
    open: boolean;
    mrr: Dec;
    usage: Dec;
    unattributed: Dec;
    /** Only when the customer tier is available. */
    customers: number | null;
    bridge: Bridge | null;
    revenue: Dec;
    /** Only when `available.cogs`. */
    cogs: Dec | null;
    netBurn: Dec;
    /** Net burn + Δdeferred revenue − Δreceivables; only when either role resolves. */
    netBurnAdjusted: Dec | null;
    cashEnd: Dec;
    sm: Dec;
    rnd: Dec;
    ga: Dec;
    /** Only when that role resolves somewhere. */
    deferredEnd: Dec | null;
    arEnd: Dec | null;
    apEnd: Dec | null;
}

/** One half's metrics. Every `null` is a figure the journal can't support for that half. */
export interface PeriodMetrics {
    /** The half's last complete month (`YYYY-MM`), or null when it has none. */
    month: string | null;
    /** k: the half's complete-month count. */
    months: number;
    mrr: Dec | null;
    arr: Dec | null;
    usage: Dec | null;
    mrrGrowthPct: number | null;
    cmgrPct: number | null;
    netNewArr: Dec | null;
    bridge: Bridge | null;
    customers: number | null;
    customerGrowthPct: number | null;
    logoChurnPct: number | null;
    arpa: Dec | null;
    acv: Dec | null;
    nrrPct: number | null;
    grrPct: number | null;
    quickRatio: number | null;
    top1Pct: number | null;
    top5Pct: number | null;
    top10Pct: number | null;
    /** Always present (zero when the half has no complete month). */
    revenue: Dec;
    grossProfit: Dec | null;
    grossMarginPct: number | null;
    /** Σ net burn over the half's complete months. Always present. */
    netBurn: Dec;
    netBurnAvg3: Dec | null;
    /** Σ adjusted net burn over the half's complete months (A3); only when adjusted burn exists. */
    netBurnAdjusted: Dec | null;
    /** 3-month average of adjusted burn (prepayments and receivables smoothed out). */
    netBurnAdjustedAvg3: Dec | null;
    cash: Dec | null;
    /** Runway and cash-out date use adjusted burn when it exists, else cash burn. Null without a complete month. */
    runwayBasis: RunwayBasis | null;
    runwayMonths: number | null;
    cashOutDate: ISODate | null;
    burnMultiple: number | null;
    /** Which burn the burn multiple divides (A3): adjusted whenever it exists. Present whenever `burnMultiple` is. */
    burnMultipleBasis: RunwayBasis | null;
    ruleOf40: number | null;
    sm: Dec | null;
    rnd: Dec | null;
    ga: Dec | null;
    smPct: number | null;
    rndPct: number | null;
    gaPct: number | null;
    cac: Dec | null;
    cacPaybackMonths: number | null;
    deferredRevenue: Dec | null;
    billings: Dec | null;
    dsoDays: number | null;
    dpoDays: number | null;
    /** Only when `available.billingCustomers`. */
    billingCustomers: BillingCustomers | null;
}

/** One value of a dimension key at the current half's month. */
export interface SegmentRow {
    /** The tag value, or `(unassigned)` (always last). */
    value: string;
    mrrCurr: Dec;
    mrrPrev: Dec;
    customersCurr: number | null;
    arpa: Dec | null;
    nrrPct: number | null;
    bridge: Bridge | null;
}

/** One breakdown key (`segment`, `product`, …) and its rows, by current MRR descending. */
export interface SegmentBreakdown {
    key: string;
    rows: SegmentRow[];
}

/** One customer's contribution to a bridge category over the current half. */
export interface BridgeCustomer {
    /** The identity key (`customer-id:` or the name). */
    customer: string;
    /** The display name. */
    name: string;
    segment: string | null;
    amount: Dec;
    /** Attributed from the payee rather than a tag. */
    inferred: boolean;
}

export type BridgeCustomers = Record<BridgeCategory, BridgeCustomer[]>;

/** One of the largest customers by MRR at the current half's month. */
export interface TopCustomer {
    customer: string;
    name: string;
    mrr: Dec;
    pct: number;
}

/** One customer's MRR across every month of the span (`detail=customers`). */
export interface CustomerMatrixRow {
    customer: string;
    name: string;
    segment: string | null;
    /** Aligned to {@link SaasReport.months}. */
    values: Dec[];
}

/** The startup-metrics report for an Insights span. */
export interface SaasReport {
    /** The same split as /api/insights. */
    period: InsightsPeriod;
    base: string;
    available: SaasAvailable;
    months: SaasMonth[];
    current: PeriodMetrics;
    previous: PeriodMetrics;
    /** Empty when no dimension is offered. */
    segments: SegmentBreakdown[];
    /** Null without the customer tier. */
    bridgeCustomers: BridgeCustomers | null;
    /** ≤ 10, at `current.month`; empty without the customer tier. */
    topCustomers: TopCustomer[];
    /** Only with `detail=customers` and the customer tier. */
    customerMatrix: CustomerMatrixRow[] | null;
    /** Recurring/usage revenue in commodities with no price, left out of MRR. */
    unvalued: ReadonlyMap<string, Dec>;
}
