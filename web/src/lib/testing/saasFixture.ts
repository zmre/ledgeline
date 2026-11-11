// A decoded SaasReport for the startup-metrics tests, built from the figures
// fixtures/saas/startup.journal yields on its Year-over-year span (README:
// "Period metrics: the Year-over-year span"; checked against the live engine).
// `startupReport()` is that report; pass overrides to switch a tier off or
// drop a figure. Months are trimmed to the three the bridge test needs.

import {dec, type Dec} from "#lib/domain/money.js";
import type {BillingCustomers, Bridge, PeriodMetrics, SaasMonth, SaasReport} from "#lib/reports/saasTypes.js";

/** Dollars (and cents) as an exact Dec. */
export const usd = (dollars: number): Dec => dec(Math.round(dollars * 100), 2);

const ZERO = usd(0);

export function bridge(over: Partial<Bridge> = {}): Bridge {
    return {
        new: usd(1500),
        expansion: usd(1100),
        contraction: usd(150),
        churn: usd(800),
        reactivation: usd(600),
        other: ZERO,
        newLogos: 1,
        churnedLogos: 3,
        reactivatedLogos: 2,
        ...over,
    };
}

/** A half with nothing beyond tier A, for overriding into shape. */
export function emptyPeriod(over: Partial<PeriodMetrics> = {}): PeriodMetrics {
    return {
        month: null,
        months: 0,
        mrr: null,
        arr: null,
        usage: null,
        mrrGrowthPct: null,
        cmgrPct: null,
        netNewArr: null,
        bridge: null,
        customers: null,
        customerGrowthPct: null,
        logoChurnPct: null,
        arpa: null,
        acv: null,
        nrrPct: null,
        grrPct: null,
        quickRatio: null,
        top1Pct: null,
        top5Pct: null,
        top10Pct: null,
        revenue: ZERO,
        grossProfit: null,
        grossMarginPct: null,
        netBurn: ZERO,
        netBurnAvg3: null,
        netBurnAdjusted: null,
        netBurnAdjustedAvg3: null,
        cash: null,
        runwayBasis: null,
        runwayMonths: null,
        cashOutDate: null,
        burnMultiple: null,
        burnMultipleBasis: null,
        ruleOf40: null,
        sm: null,
        rnd: null,
        ga: null,
        smPct: null,
        rndPct: null,
        gaPct: null,
        cac: null,
        cacPaybackMonths: null,
        deferredRevenue: null,
        billings: null,
        dsoDays: null,
        dpoDays: null,
        billingCustomers: null,
        ...over,
    };
}

export const STARTUP_CURRENT: PeriodMetrics = emptyPeriod({
    month: "2026-06",
    months: 12,
    mrr: usd(5850),
    arr: dec(70200, 0),
    usage: usd(100),
    mrrGrowthPct: 62.5,
    cmgrPct: 4.128860000867873,
    netNewArr: dec(27000, 0),
    bridge: bridge(),
    customers: 8,
    customerGrowthPct: 0,
    logoChurnPct: 37.5,
    arpa: usd(731.25),
    acv: usd(18000),
    nrrPct: 120.83333333333333,
    grrPct: 90.27777777777779,
    quickRatio: 3.3684210526315788,
    top1Pct: 25.64102564102564,
    top5Pct: 87.17948717948718,
    top10Pct: 100,
    revenue: usd(66550),
    grossProfit: usd(54480),
    grossMarginPct: 81.8632607062359,
    netBurn: usd(43720),
    netBurnAvg3: usd(6543.33),
    netBurnAdjusted: usd(44070),
    netBurnAdjustedAvg3: usd(3693.33),
    cash: usd(432160),
    runwayBasis: "adjusted",
    runwayMonths: 117.01083032490975,
    cashOutDate: "2036-03-31",
    burnMultiple: 1.6322222222222222,
    burnMultipleBasis: "adjusted",
    ruleOf40: 195.0096408552365,
    sm: usd(33000),
    rnd: usd(48000),
    ga: usd(18000),
    smPct: 49.586776859504134,
    rndPct: 72.12622088655147,
    gaPct: 27.0473328324568,
    cac: usd(33000),
    cacPaybackMonths: 186.05133852931212,
    deferredRevenue: usd(10400),
    billings: usd(68950),
    dsoDays: 25.50338091660406,
    dpoDays: 3.8448726028630595,
});

export const STARTUP_PREVIOUS: PeriodMetrics = emptyPeriod({
    month: "2025-06",
    months: 12,
    mrr: usd(3600),
    arr: dec(43200, 0),
    usage: ZERO,
    netNewArr: dec(43200, 0),
    customers: 8,
    revenue: usd(18450),
    grossProfit: usd(14760),
    grossMarginPct: 80,
    netBurn: usd(24120),
    cash: usd(475880),
});

function month(name: string, mrr: number, over: Partial<SaasMonth> = {}): SaasMonth {
    return {
        month: name,
        partial: false,
        open: false,
        mrr: usd(mrr),
        usage: usd(100),
        unattributed: ZERO,
        customers: 8,
        bridge: bridge({new: ZERO, expansion: ZERO, contraction: ZERO, churn: ZERO, reactivation: ZERO, newLogos: 0, churnedLogos: 0, reactivatedLogos: 0}),
        revenue: usd(mrr + 100),
        cogs: usd(mrr * 0.2),
        netBurn: usd(9670),
        netBurnAdjusted: usd(3720),
        cashEnd: usd(432160),
        sm: usd(3000),
        rnd: usd(4000),
        ga: usd(1500),
        deferredEnd: usd(10400),
        arEnd: usd(4650),
        apEnd: usd(1170),
        ...over,
    };
}

/** The startup report; `over` replaces top-level fields wholesale. */
export function startupReport(over: Partial<SaasReport> = {}): SaasReport {
    return {
        period: {
            start: "2024-07-01",
            mid: "2025-06-30",
            end: "2026-06-30",
            prevStart: "2024-07-01",
            prevEnd: "2025-06-30",
            currStart: "2025-07-01",
            currEnd: "2026-06-30",
        },
        base: "$",
        available: {
            recurring: true,
            customers: true,
            business: true,
            cogs: true,
            dimensions: ["segment", "product"],
            customersHiddenReason: null,
            coveragePct: 99.74619289340102,
            inferredCustomers: false,
            billingCustomers: false,
            closedThrough: "2026-06",
        },
        months: [month("2026-04", 5950), month("2026-05", 5850), month("2026-06", 5850)],
        current: STARTUP_CURRENT,
        previous: STARTUP_PREVIOUS,
        segments: [
            {
                key: "segment",
                rows: [
                    {value: "mid", mrrCurr: usd(2900), mrrPrev: usd(1100), customersCurr: 3, arpa: usd(966.67), nrrPct: 127.27272727272727, bridge: null},
                    {value: "enterprise", mrrCurr: usd(2200), mrrPrev: usd(1000), customersCurr: 2, arpa: usd(1100), nrrPct: 130, bridge: null},
                    {value: "smb", mrrCurr: usd(750), mrrPrev: usd(1500), customersCurr: 3, arpa: usd(250), nrrPct: 50, bridge: null},
                ],
            },
            {
                key: "product",
                rows: [
                    {value: "core", mrrCurr: usd(3650), mrrPrev: usd(2600), customersCurr: 6, arpa: usd(608.33), nrrPct: 82.6923076923077, bridge: null},
                    {value: "platform", mrrCurr: usd(2200), mrrPrev: usd(1000), customersCurr: 2, arpa: usd(1100), nrrPct: 130, bridge: null},
                ],
            },
        ],
        bridgeCustomers: {
            new: [{customer: "Granite Health", name: "Granite Health", segment: "mid", amount: usd(1500), inferred: false}],
            expansion: [
                {customer: "Orbit Freight", name: "Orbit Freight", segment: "enterprise", amount: usd(500), inferred: false},
                {customer: "Bluebird Analytics", name: "Bluebird Analytics", segment: "mid", amount: usd(300), inferred: false},
                {customer: "Northwind Logistics", name: "Northwind Logistics", segment: "enterprise", amount: usd(300), inferred: false},
            ],
            contraction: [{customer: "Harbor Yoga", name: "Harbor Yoga", segment: "smb", amount: usd(150), inferred: false}],
            churn: [
                {customer: "Fjord Studio", name: "Fjord Studio", segment: "smb", amount: usd(350), inferred: false},
                {customer: "Lumen Labs", name: "Lumen Labs", segment: "smb", amount: usd(250), inferred: false},
                {customer: "Pinecrest Dental", name: "Pinecrest Dental", segment: "smb", amount: usd(200), inferred: false},
            ],
            reactivation: [
                {customer: "Fjord Studio", name: "Fjord Studio", segment: "smb", amount: usd(350), inferred: false},
                {customer: "Lumen Labs", name: "Lumen Labs", segment: "smb", amount: usd(250), inferred: false},
            ],
        },
        topCustomers: [{customer: "Granite Health", name: "Granite Health", mrr: usd(1500), pct: 25.64102564102564}],
        customerMatrix: null,
        unvalued: new Map(),
        ...over,
    };
}

/** A household journal's report (sample.journal's shape): nothing available, tier-A figures only. */
export function householdReport(): SaasReport {
    return startupReport({
        available: {
            recurring: false,
            customers: false,
            business: false,
            cogs: false,
            dimensions: [],
            customersHiddenReason: null,
            coveragePct: null,
            inferredCustomers: false,
            billingCustomers: false,
            closedThrough: null,
        },
        current: emptyPeriod({month: "2026-06", months: 12, revenue: usd(68007.5), netBurn: usd(-20000), cash: usd(50277.56), runwayBasis: "cash"}),
        previous: emptyPeriod({month: "2025-06", months: 12}),
        segments: [],
        bridgeCustomers: null,
        topCustomers: [],
    });
}

/**
 * Books that tag customers on invoices only (fixtures/saas/clearing.journal's
 * shape, Amendment A1.2): per-customer MRR is hidden as billing-clearing, and
 * the customer count comes from billings instead.
 */
export function billingReport(over: Partial<BillingCustomers> = {}): SaasReport {
    const base = startupReport();
    return startupReport({
        available: {...base.available, customers: false, customersHiddenReason: "billing-clearing", coveragePct: 100, billingCustomers: true},
        current: emptyPeriod({
            ...STARTUP_CURRENT,
            customers: null,
            bridge: null,
            customerGrowthPct: null,
            logoChurnPct: null,
            arpa: null,
            acv: null,
            nrrPct: null,
            grrPct: null,
            quickRatio: null,
            top1Pct: null,
            top5Pct: null,
            top10Pct: null,
            cac: null,
            cacPaybackMonths: null,
            billingCustomers: {
                customers: 12,
                customerGrowthPct: 20,
                newLogos: 3,
                lapsedLogos: 1,
                logoChurnPct: 10,
                acv: usd(12600),
                windowShort: false,
                inferred: false,
                ...over,
            },
        }),
        segments: [],
        bridgeCustomers: null,
        topCustomers: [],
    });
}
