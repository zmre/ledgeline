// The home page's time row (Home, item 5): hours this week against last
// week to the same weekday, the billable share of each, and what is unbilled
// across customers (hours and, at each account's item rate, money).
//
// Composed from the existing endpoints, three requests in parallel:
// - `GET /api/time/summary` with no grouping: one row when there is any time
//   at all (none: the row stays hidden);
// - `GET /api/time/summary?from=…&to=…&group=day,billable` over last week's
//   Monday to this week's Sunday: both weeks' figures from one answer;
// - `GET /api/time/unbilled`: each customer's unbilled time and estimate.
// Before setup the summary refuses with `not-set-up`, which is "hidden" too.
//
// Pure apart from `fetchHomeTime`: no runes, no DOM.
import {EditorApiError} from "#lib/editorApi.js";
import {fetchUnbilled, type Unbilled} from "../api/billing.js";
import {fetchSummary, type Summary} from "../api/summary.js";
import {presetRange, previousRange, type Range} from "../dashboard/query.js";

/** One week's figures, in seconds. */
export interface WeekFigures {
    from: string;
    /** The last day counted (last week: the same weekday as today). */
    end: string;
    total: number;
    billable: number;
    /** Billable over total; null with no time. */
    share: number | null;
}

/** An amount in one commodity, plain decimal text (`1583.33`). */
export interface Money {
    commodity: string;
    amount: string;
}

export interface UnbilledFigures {
    seconds: number;
    /** Customers with unbilled time. */
    customers: number;
    /** The estimates summed per commodity, largest first. */
    money: Money[];
    /** Some customer's time has no rate, so `money` leaves it out. */
    unpriced: boolean;
    /** Billable time on accounts no customer is linked to. */
    needsCustomer: number;
}

export interface HomeTime {
    thisWeek: WeekFigures;
    lastWeek: WeekFigures;
    unbilled: UnbilledFigures;
}

/** Where the row's tiles go, resolved. */
export interface HomeTimeLinks {
    /** The Overview on This week. */
    hours: string;
    /** The Overview on This week, split by billable. */
    billable: string;
    /** Entries' unbilled time. */
    unbilled: string;
    /** To bill. */
    bill: string;
    /** The Overview. */
    overview: string;
}

/** This week (Monday to Sunday) and last week to the same weekday, on `today`. */
export function weekRanges(today: string): {thisWeek: Range; lastWeek: Range} {
    const thisWeek = presetRange("this-week", today);
    return {thisWeek, lastWeek: previousRange(thisWeek)};
}

/** A week's figures from a `group=day,billable` summary's rows. */
export function weekOf(summary: Pick<Summary, "rows">, from: string, end: string): WeekFigures {
    let total = 0;
    let billable = 0;
    for (const row of summary.rows) {
        const [date = "", isBillable = ""] = row.key;
        if (date < from || date > end) continue;
        total += row.quantity.seconds;
        if (isBillable === "yes") billable += row.quantity.seconds;
    }
    return {from, end, total, billable, share: total > 0 ? billable / total : null};
}

/** Whole cents of a plain decimal; null when it isn't one. */
function cents(amount: string): number | null {
    const value = Number(amount);
    return Number.isFinite(value) ? Math.round(value * 100) : null;
}

/** What is unbilled across customers. */
export function unbilledOf(unbilled: Pick<Unbilled, "customers" | "needs_customer">): UnbilledFigures {
    const sums = new Map<string, number>();
    let seconds = 0;
    let unpriced = false;
    for (const c of unbilled.customers) {
        seconds += c.quantity.seconds;
        const value = c.estimate === null ? null : cents(c.estimate);
        if (value === null) {
            if (c.quantity.seconds > 0) unpriced = true;
            continue;
        }
        const commodity = c.commodity ?? "";
        sums.set(commodity, (sums.get(commodity) ?? 0) + value);
    }
    const money = [...sums.entries()]
        .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
        .map(([commodity, value]) => ({commodity, amount: (value / 100).toFixed(2)}));
    return {
        seconds,
        customers: unbilled.customers.filter((c) => c.quantity.seconds > 0).length,
        money,
        unpriced,
        needsCustomer: unbilled.needs_customer.reduce((t, n) => t + n.quantity.seconds, 0),
    };
}

/** The requests `fetchHomeTime` makes, as a seam for tests. */
export interface HomeTimeApi {
    summary: typeof fetchSummary;
    unbilled: typeof fetchUnbilled;
}

const httpApi: HomeTimeApi = {summary: fetchSummary, unbilled: fetchUnbilled};

/**
 * The row's figures on `today`, or null when the row has nothing to say:
 * time tracking isn't set up, or there is no time at all. Any other refusal
 * is thrown.
 */
export async function fetchHomeTime(today: string, api: HomeTimeApi = httpApi): Promise<HomeTime | null> {
    const {thisWeek, lastWeek} = weekRanges(today);
    try {
        const [all, weeks, unbilled] = await Promise.all([
            api.summary({group: []}),
            api.summary({from: lastWeek.from, to: thisWeek.to, group: ["day", "billable"]}),
            api.unbilled(),
        ]);
        if (all.rows.length === 0) return null;
        return {
            thisWeek: weekOf(weeks, thisWeek.from, thisWeek.to),
            lastWeek: weekOf(weeks, lastWeek.from, lastWeek.end),
            unbilled: unbilledOf(unbilled),
        };
    } catch (e: unknown) {
        if (e instanceof EditorApiError && e.code === "not-set-up") return null;
        throw e;
    }
}
