// The Time area's filters as a URL query, shared by its two tabs, Overview
// (/time) and Entries (/time/entries), so one filter bar means the same on
// both and a tab switch keeps it (the invoicer's list contract, PLAN 5b
// S1–S4: one parse and one serialize, so a link, a reload and Back/Forward
// land on the same view).
//
// URL parameters: `period` (a preset; absent is this month), `from`/`to`
// (a custom range, which wins over a preset), `person` (`-` is
// unattributed), `customer`, `billable` (`yes`/`no`), `billed` (the
// billing state), `account` (that account and below) and `q` (text in the
// note or account). Each tab adds its own view options after these
// (Overview's `depth`, `compare`, …; Entries' `group` and `week`), which a
// tab switch drops. Old links: `period=ytd` (Overview's Year to date) reads
// as This year, which counts the same time.
import {addDays, addMonths, daysBetween, isIsoDate, isMonthEnd, isMonthStart, mondayOf, monthEnd, monthStart, quarterStart} from "../dates.js";

export const PERIODS = ["this-week", "last-week", "this-month", "last-month", "this-quarter", "this-year", "last-year", "last-12-months", "all"] as const;
export type Period = (typeof PERIODS)[number];

export const PERIOD_LABELS: Record<Period, string> = {
    "this-week": "This week",
    "last-week": "Last week",
    "this-month": "This month",
    "last-month": "Last month",
    "this-quarter": "This quarter",
    "this-year": "This year",
    "last-year": "Last year",
    "last-12-months": "Last 12 months",
    all: "All time",
};

export const DEFAULT_PERIOD: Period = "this-month";

/** Presets of old links, read as today's. */
const LEGACY_PERIODS: Record<string, Period> = {ytd: "this-year"};

export const BILLED_STATES = ["unbilled", "invoiced", "nobill", "not-billable", "billed"] as const;
export type BilledState = (typeof BILLED_STATES)[number];

export const BILLED_LABELS: Record<BilledState, string> = {
    unbilled: "Unbilled",
    invoiced: "On an invoice",
    nobill: "Not billed (written off)",
    "not-billable": "Not billable",
    billed: "Invoiced or written off",
};

/** The person filter's value for time with no person. */
export const UNATTRIBUTED = "-";

/** The shared filters. Empty strings are "no filter". */
export interface TimeFilters {
    /** A preset; ignored while `from` or `to` is set (a custom range). */
    period: Period;
    from: string;
    to: string;
    /** A short name; `UNATTRIBUTED` for none. */
    person: string;
    /** The `customer:` tag's value. */
    customer: string;
    billable: "" | "yes" | "no";
    billed: BilledState | "";
    /** This account or below. */
    account: string;
    /** Text in the note or account. */
    q: string;
}

export const DEFAULT_FILTERS: TimeFilters = {
    period: DEFAULT_PERIOD,
    from: "",
    to: "",
    person: "",
    customer: "",
    billable: "",
    billed: "",
    account: "",
    q: "",
};

export type Params = Pick<URLSearchParams, "get">;

export const oneOf = <T extends string>(values: readonly T[], text: string): T | null => ((values as readonly string[]).includes(text) ? (text as T) : null);

export const param = (params: Params, name: string): string => (params.get(name) ?? "").trim();

/** An ISO date parameter, or "" when absent or unreadable. */
export function dateParam(params: Params, name: string): string {
    const value = param(params, name);
    return isIsoDate(value) ? value : "";
}

/** The filters from a URL; unreadable values are dropped. */
export function parseFilters(params: Params): TimeFilters {
    const period = param(params, "period");
    const billable = param(params, "billable");
    return {
        period: oneOf(PERIODS, period) ?? LEGACY_PERIODS[period] ?? DEFAULT_PERIOD,
        from: dateParam(params, "from"),
        to: dateParam(params, "to"),
        person: param(params, "person"),
        customer: param(params, "customer"),
        billable: billable === "yes" || billable === "no" ? billable : "",
        billed: oneOf(BILLED_STATES, param(params, "billed")) ?? "",
        account: param(params, "account"),
        q: param(params, "q"),
    };
}

/** The filters as URL parameters: only what isn't the default, in a fixed order. */
export function filterParams(filters: TimeFilters): URLSearchParams {
    const out = new URLSearchParams();
    const custom = filters.from !== "" || filters.to !== "";
    if (!custom && filters.period !== DEFAULT_PERIOD) out.set("period", filters.period);
    if (filters.from !== "") out.set("from", filters.from);
    if (filters.to !== "") out.set("to", filters.to);
    const text = (name: "person" | "customer" | "account" | "q"): void => {
        const value = filters[name].trim();
        if (value !== "") out.set(name, value);
    };
    text("person");
    text("customer");
    if (filters.billable !== "") out.set("billable", filters.billable);
    if (filters.billed !== "") out.set("billed", filters.billed);
    text("account");
    text("q");
    return out;
}

/** The filters as a URL query, without the `?`. */
export const filtersQueryString = (filters: TimeFilters): string => filterParams(filters).toString();

/** Only the shared filters of a tab's query (its view options dropped). */
export const filtersOf = (query: TimeFilters): TimeFilters => ({
    period: query.period,
    from: query.from,
    to: query.to,
    person: query.person,
    customer: query.customer,
    billable: query.billable,
    billed: query.billed,
    account: query.account,
    q: query.q,
});

/** `base` with `qs` (no `?`) as its query. */
export const withQuery = (base: string, qs: string): string => (qs === "" ? base : `${base}?${qs}`);

/** Whether any filter narrows the time (the period is not a filter). */
export const isFiltered = (filters: TimeFilters): boolean =>
    [filters.person, filters.customer, filters.billable, filters.billed, filters.account, filters.q].some((v) => v.trim() !== "");

/** Every filter cleared; the period and the view options kept. */
export const clearFilters = <Q extends TimeFilters>(query: Q): Q => ({...query, person: "", customer: "", billable: "", billed: "", account: "", q: ""});

/** The person a write should name: the filter's, unless it is "anyone" or "unattributed". */
export function filterPerson(filters: TimeFilters): string | undefined {
    const person = filters.person.trim();
    return person === "" || person === UNATTRIBUTED ? undefined : person;
}

/** Whether the period is a custom range. */
export const isCustom = (filters: Pick<TimeFilters, "from" | "to">): boolean => filters.from !== "" || filters.to !== "";

// ── Periods as dates ────────────────────────────────────────────────────

/** How a period steps to the one before or after it: by whole months (the day kept), or by days. */
export type Step = {unit: "month"; n: number} | {unit: "day"; n: number};

/** The dates a period covers. */
export interface Range {
    from: string;
    /** The last day, inclusive (it may be after today: this month's end). */
    to: string;
    /**
     * The last day that has happened: `to`, or today when the period runs on
     * past it. Working days, averages and the previous period count to here,
     * so this month to date compares with last month to the same day.
     */
    end: string;
    step: Step;
}

export const shift = (date: string, step: Step, times: number): string =>
    step.unit === "month" ? addMonths(date, step.n * times) : addDays(date, step.n * times);

const range = (from: string, to: string, step: Step, today: string): Range => ({
    from,
    to,
    end: to <= today || from > today ? to : today,
    step,
});

/** A custom range's step: whole months when it is, else its length in days. */
function stepOf(from: string, to: string): Step {
    if (isMonthStart(from) && isMonthEnd(to)) {
        const months = (Number(to.slice(0, 4)) - Number(from.slice(0, 4))) * 12 + Number(to.slice(5, 7)) - Number(from.slice(5, 7)) + 1;
        return {unit: "month", n: months};
    }
    return {unit: "day", n: daysBetween(from, to) + 1};
}

/**
 * A preset's dates on `today`. All time runs from `first` (the first day
 * with time; today when there is none) to today.
 */
export function presetRange(preset: Period, today: string, first: string | null = null): Range {
    const week: Step = {unit: "day", n: 7};
    const year = today.slice(0, 4);
    switch (preset) {
        case "this-week": {
            const monday = mondayOf(today);
            return range(monday, addDays(monday, 6), week, today);
        }
        case "last-week": {
            const monday = addDays(mondayOf(today), -7);
            return range(monday, addDays(monday, 6), week, today);
        }
        case "this-month":
            return range(monthStart(today), monthEnd(today), {unit: "month", n: 1}, today);
        case "last-month": {
            const last = addMonths(monthStart(today), -1);
            return range(last, monthEnd(last), {unit: "month", n: 1}, today);
        }
        case "this-quarter": {
            const start = quarterStart(today);
            return range(start, addDays(addMonths(start, 3), -1), {unit: "month", n: 3}, today);
        }
        case "this-year":
            return range(`${year}-01-01`, `${year}-12-31`, {unit: "month", n: 12}, today);
        case "last-year": {
            const last = String(Number(year) - 1);
            return range(`${last}-01-01`, `${last}-12-31`, {unit: "month", n: 12}, today);
        }
        case "last-12-months":
            return range(addMonths(monthStart(today), -11), monthEnd(today), {unit: "month", n: 12}, today);
        case "all": {
            const from = first !== null && first < today ? first : today;
            return range(from, today, {unit: "day", n: daysBetween(from, today) + 1}, today);
        }
    }
}

/**
 * The dates `filters` cover on `today`, as a closed range (Overview's
 * figures need both ends). A custom range with one end missing is open on
 * that side as far as a preset would go: no `from` is the 30 days up to
 * `to`, no `to` runs to today.
 */
export function resolveRange(filters: Pick<TimeFilters, "period" | "from" | "to">, today: string, first: string | null = null): Range {
    if (!isCustom(filters)) return presetRange(filters.period, today, first);
    const to = filters.to !== "" ? filters.to : today < filters.from ? filters.from : today;
    const from = filters.from !== "" ? filters.from : addDays(to, -29);
    const [a, b] = from <= to ? [from, to] : [to, from];
    return range(a, b, stepOf(a, b), today);
}

/** Whether the period is All time, which has no period before or after it. */
export const isAllTime = (filters: Pick<TimeFilters, "period" | "from" | "to">): boolean => !isCustom(filters) && filters.period === "all";

/** The period just before `current`, the same length, to the same point in it. */
export function previousRange(current: Range): Range {
    const from = shift(current.from, current.step, -1);
    const end = shift(current.end, current.step, -1);
    return {from, to: end, end, step: current.step};
}

/**
 * The period `direction` steps before (-1) or after (+1) the one shown, as a
 * new query: a preset when the result is one (stepping back from This month
 * is Last month), else a custom range. All time doesn't step.
 */
export function stepPeriod<Q extends TimeFilters>(query: Q, today: string, direction: -1 | 1): Q {
    if (isAllTime(query)) return query;
    const current = resolveRange(query, today);
    const from = shift(current.from, current.step, direction);
    // A month's last day steps to a month's last day (addMonths keeps it one).
    const to = shift(current.to, current.step, direction);
    const preset = PERIODS.find((p) => {
        if (p === "all") return false;
        const r = presetRange(p, today);
        return r.from === from && r.to === to;
    });
    return preset === undefined ? {...query, from, to} : {...query, period: preset, from: "", to: ""};
}
