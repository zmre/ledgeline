// The startup-metrics section's decisions (plans/37), as pure functions over the
// decoded SaasReport: which Insights boxes hide, what the Runway box says, the
// MRR bridge's rows, the "More metrics" list, and the hidden-customers note.
// No Svelte/DOM, so it is a node test (startup.test.ts); the components only
// render what these return.

import {add, isZero, sub, type Dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {InsightsReport} from "#lib/reports/insightsTypes.js";
import type {Bridge, BridgeCategory, PeriodMetrics, SaasAvailable, SaasReport} from "#lib/reports/saasTypes.js";
import type {DataView} from "#lib/stores/loadState.js";
import {fmt, fmtDays, fmtMonths, fmtPct, fmtRatio} from "./format";
import type {MetricId} from "./metricHelp";

/** The published docs page for startup metrics; anchors are its headings. */
export const STARTUP_DOCS = "https://zmre.github.io/ledgeline/reports/startup-metrics/";

/** Customer metrics need this share of recurring revenue attributed (plans/37 decision 8). */
export const CUSTOMER_COVERAGE_NEEDED = 90;

/** A comparison span, as the insights and saas resources key it. */
export interface Span {
    start: string;
    end: string;
}

/**
 * Whether a held report answers the span on screen. A saas report for the
 * PREVIOUS span must never drive this one's boxes: its business flag might
 * survive, but its Runway, cash and startup figures are another period's.
 */
export function sameSpan(a: Span | null, b: Span | null): boolean {
    return a !== null && b !== null && a.start === b.start && a.end === b.end;
}

/**
 * Whether Insights offers the startup-metrics export: a report is held, it
 * answers the span on screen (one from the previous span must not decide), and
 * the journal has recurring revenue.
 */
export function exportsStartup(view: DataView, query: Span | null, report: SaasReport | null, span: Span): boolean {
    return view === "data" && sameSpan(query, span) && report !== null && report.available.recurring;
}

/**
 * The month the startup metrics stop at, when that is before the span's last
 * month (A2): later months have no recurring revenue booked yet, and the
 * engine leaves them out rather than read them as churn. Null otherwise.
 */
export function closedThroughMonth(report: SaasReport): string | null {
    const through = report.available.closedThrough;
    return through !== null && through < report.period.end.slice(0, 7) ? through : null;
}

/** The burn multiple box's small line: the burn it divided (A3). */
export function burnMultipleLine(current: PeriodMetrics, base: string, styles: ReadonlyMap<string, AmountStyle>): string {
    if (current.burnMultipleBasis === "adjusted" && current.netBurnAdjusted !== null) {
        return `net burn ${fmt(base, current.netBurnAdjusted, styles)} (adjusted for prepayments)`;
    }
    return `net burn ${fmt(base, current.netBurn, styles)}`;
}

/** Which of the existing Insights boxes show (plans/37 decision 4). */
export interface BoxVisibility {
    netWorth: boolean;
    /** In Net Worth's slot; only ever true when `netWorth` is false. */
    runway: boolean;
    investment: boolean;
    movers: boolean;
}

/**
 * The hide rules. `saas` is null until /api/saas answers (or when it fails):
 * the dashboard then treats the journal as a household, so a slow or broken
 * startup endpoint never costs a household its Net Worth box.
 */
export function boxVisibility(report: InsightsReport, saas: SaasReport | null): BoxVisibility {
    const worth = report.netWorth.current.get(report.base);
    const negative = worth !== undefined && worth.m < 0n;
    const netWorth = !negative && !(saas?.available.business ?? false);
    return {
        netWorth,
        runway: !netWorth && saas !== null && saas.current.cash !== null,
        investment: report.hasHoldings,
        movers: report.movers.length > 0,
    };
}

/** Whether the startup section renders at all. */
export function showsStartup(available: SaasAvailable): boolean {
    return available.recurring || available.business;
}

/** The Runway box's two lines. */
export interface RunwayText {
    big: string;
    small: string;
}

/**
 * Runway from the current half: months of cash at the 3-month average burn,
 * or "Not burning" with the cash itself when the average burn is zero or
 * less. Null only when the half has no complete month (no cash figure).
 *
 * The burn is the one the engine's runway used (Amendment A1.1): burn
 * adjusted for prepayments and receivables when the journal has either role
 * (`runwayBasis: "adjusted"`), shown beside the raw cash burn; cash burn
 * otherwise. "Not burning" is judged on that same burn.
 */
export function runwayText(current: PeriodMetrics, base: string, styles: ReadonlyMap<string, AmountStyle>): RunwayText | null {
    if (current.cash === null) return null;
    const adjusted = current.runwayBasis === "adjusted";
    const used = adjusted ? current.netBurnAdjustedAvg3 : current.netBurnAvg3;
    if (current.runwayMonths === null || used === null) {
        return {big: "Not burning", small: `cash ${fmt(base, current.cash, styles)}`};
    }
    const burn =
        adjusted && current.netBurnAvg3 !== null
            ? `burn ${fmt(base, used, styles)}/mo adjusted for prepayments · ${fmt(base, current.netBurnAvg3, styles)}/mo cash (3-mo avg)`
            : `burn ${fmt(base, used, styles)}/mo (3-mo avg)`;
    // The engine leaves the date out beyond 100 years.
    const out = current.cashOutDate === null ? "" : `cash-out ~${current.cashOutDate.slice(0, 7)} · `;
    return {big: fmtMonths(current.runwayMonths), small: `${out}${burn}`};
}

/** Human labels for the bridge categories, in bridge order. */
export const BRIDGE_LABELS: Readonly<Record<BridgeCategory, string>> = {
    new: "New",
    expansion: "Expansion",
    contraction: "Contraction",
    churn: "Churn",
    reactivation: "Reactivation",
};

/** One line of the MRR bridge between its start and end. */
export interface BridgeLine {
    category: BridgeCategory | "other";
    label: string;
    /** Signed as it moves MRR: contraction and churn are negative. */
    amount: Dec;
    /** Customer count, for the categories that count logos. */
    count: number | null;
}

/** The bridge for a half: start MRR, each non-zero line, end MRR. */
export interface BridgeModel {
    start: Dec;
    end: Dec;
    lines: BridgeLine[];
}

/**
 * The bridge as a list. Start MRR is derived from the identity the engine
 * guarantees (end − start = new + expansion − contraction − churn +
 * reactivation + other), so it exists even when the month before the half is
 * outside the span. Zero lines are left out; Other shows only when non-zero.
 */
export function bridgeModel(bridge: Bridge, end: Dec): BridgeModel {
    const negate = (d: Dec): Dec => sub({m: 0n, p: d.p}, d);
    const lines: BridgeLine[] = [
        {category: "new", label: BRIDGE_LABELS.new, amount: bridge.new, count: bridge.newLogos},
        {category: "expansion", label: BRIDGE_LABELS.expansion, amount: bridge.expansion, count: null},
        {category: "contraction", label: BRIDGE_LABELS.contraction, amount: negate(bridge.contraction), count: null},
        {category: "churn", label: BRIDGE_LABELS.churn, amount: negate(bridge.churn), count: bridge.churnedLogos},
        {category: "reactivation", label: BRIDGE_LABELS.reactivation, amount: bridge.reactivation, count: bridge.reactivatedLogos},
        {category: "other", label: "Other", amount: bridge.other, count: null},
    ];
    const net = lines.reduce<Dec>((sum, line) => add(sum, line.amount), {m: 0n, p: 0});
    return {start: sub(end, net), end, lines: lines.filter((line) => !isZero(line.amount))};
}

/** One row of the "More metrics" list. */
export interface MoreMetric {
    /** Stable id: the testid suffix, and the key of its help. */
    key: MetricId;
    label: string;
    value: string;
}

/**
 * The secondary metrics, each only when the engine sent a value. Labels match
 * the table under "More metrics" in docs/reports/startup-metrics.md.
 */
export function moreMetrics(current: PeriodMetrics, base: string, styles: ReadonlyMap<string, AmountStyle>): MoreMetric[] {
    const money = (d: Dec | null): string | null => (d === null ? null : fmt(base, d, styles));
    const pct = (n: number | null): string | null => (n === null ? null : fmtPct(n));
    const share = (n: number | null, label: string): string | null => (n === null ? null : `${fmtPct(n)} ${label}`);
    const tops = [
        current.top1Pct === null ? null : `top 1: ${fmtPct(current.top1Pct)}`,
        current.top5Pct === null ? null : `top 5: ${fmtPct(current.top5Pct)}`,
        current.top10Pct === null ? null : `top 10: ${fmtPct(current.top10Pct)}`,
    ].filter((part): part is string => part !== null);
    const rows: [MetricId, string, string | null][] = [
        ["arpa", "ARPA", money(current.arpa)],
        ["acv", "ACV", money(current.acv)],
        ["logo-churn", "Logo churn", pct(current.logoChurnPct)],
        ["quick-ratio", "Quick ratio", current.quickRatio === null ? null : fmtRatio(current.quickRatio)],
        ["top-customers", "Top customer share", tops.length === 0 ? null : tops.join(" · ")],
        ["cac", "CAC", money(current.cac)],
        ["cac-payback", "CAC payback", current.cacPaybackMonths === null ? null : fmtMonths(current.cacPaybackMonths)],
        ["sm-pct", "S&M %", share(current.smPct, "of revenue")],
        ["rnd-pct", "R&D %", share(current.rndPct, "of revenue")],
        ["ga-pct", "G&A %", share(current.gaPct, "of revenue")],
        ["rule-of-40", "Rule of 40", current.ruleOf40 === null ? null : current.ruleOf40.toFixed(1)],
        ["dso", "DSO", current.dsoDays === null ? null : fmtDays(current.dsoDays)],
        ["dpo", "DPO", current.dpoDays === null ? null : fmtDays(current.dpoDays)],
        ["deferred-revenue", "Deferred revenue", money(current.deferredRevenue)],
        ["billings", "Billings", money(current.billings)],
        // Amendment A1.2: only when per-customer MRR is hidden and customers were billed.
        ["acv-billings", "ACV (from billings)", money(current.billingCustomers?.acv ?? null)],
        ["logo-churn-billings", "Logo churn (from billings)", pct(current.billingCustomers?.logoChurnPct ?? null)],
    ];
    return rows.filter((row): row is [MetricId, string, string] => row[2] !== null).map(([key, label, value]) => ({key, label, value}));
}

/** The hidden-customers note: the sentence, and where in the docs it is explained. */
export interface HiddenNote {
    /** Text before the inline `customer:` code span (empty when the sentence has none). */
    before: string;
    /** Whether the sentence names the `customer:` tag (rendered as code). */
    tag: boolean;
    after: string;
    /** Link text, and the docs anchor it opens. */
    link: {text: string; href: string};
}

/**
 * Why customer metrics are hidden, in one line, or null when they aren't.
 * Coverage is FLOORED, so 89.6% reads "89%", never a "90% … (90% needed)"
 * that contradicts itself.
 */
export function hiddenNote(available: SaasAvailable): HiddenNote | null {
    const note = hiddenNoteText(available);
    if (note === null || !available.billingCustomers) return note;
    // Amendment A1.2: the Customers box is back, counted from billings, so the
    // note says exactly which figures are still hidden and where the count is from.
    return {
        ...note,
        before: note.before.replace(/^Customer metrics hidden:/, "Per-customer MRR figures hidden:"),
        after: `${note.after} Customer counts are from billings.`,
    };
}

function hiddenNoteText(available: SaasAvailable): HiddenNote | null {
    const tagsLink = {text: "Customer tags", href: `${STARTUP_DOCS}#customer-tags`};
    switch (available.customersHiddenReason) {
        case null:
            return null;
        case "no-customer-tags":
            return {before: "Customer metrics hidden: no recurring revenue has a ", tag: true, after: " tag.", link: tagsLink};
        case "low-coverage": {
            const pct = Math.floor(available.coveragePct ?? 0);
            return {
                before: `Customer metrics hidden: ${pct}% of recurring revenue has a `,
                tag: true,
                after: ` tag (${CUSTOMER_COVERAGE_NEEDED}% needed).`,
                link: tagsLink,
            };
        }
        case "billing-clearing":
            return {
                before: "Customer metrics hidden: your ",
                tag: true,
                after: " tags are on invoices, but the monthly revenue-recognition lines that make up MRR have none, so per-customer figures would be billings, not MRR. Tag the recognition lines to turn these on.",
                link: {text: "Deferred revenue books", href: `${STARTUP_DOCS}#deferred-revenue-books`},
            };
    }
}
