// The time dashboard's filters as a URL query (plan 46 T-dash; the
// invoicer's list contract, PLAN 5b S1–S4): one parse and one serialize, so
// a link, a reload and Back/Forward all land on the same page.
//
// The filter names are Entries' (`from`, `to`, `person` with `-` for
// unattributed, `customer`, `billable` as `yes`/`no`, `account`), so a
// drill-down carries every filter across. The dashboard adds `period` (its
// own presets), `depth`, `compare=off`, `split`, `grain` and `target=off`.
// Every link into Entries names its dates explicitly (`from` and `to`), so
// it never depends on the two pages sharing a preset.
import type {SummaryParams} from "../api/summary.js";
import {addDays, addMonths, daysBetween, isIsoDate, isMonthEnd, isMonthStart, mondayOf, monthEnd, monthStart, quarterStart} from "../dates.js";

export const PRESETS = ["this-week", "last-week", "this-month", "last-month", "this-quarter", "ytd", "last-12-months"] as const;
export type Preset = (typeof PRESETS)[number];

export const PRESET_LABELS: Record<Preset, string> = {
    "this-week": "This week",
    "last-week": "Last week",
    "this-month": "This month",
    "last-month": "Last month",
    "this-quarter": "This quarter",
    ytd: "Year to date",
    "last-12-months": "Last 12 months",
};

/** What the hours-over-time bars are split by. */
export const SPLITS = ["branch", "customer", "person", "billable"] as const;
export type Split = (typeof SPLITS)[number];

export const SPLIT_LABELS: Record<Split, string> = {
    branch: "Top-level account",
    customer: "Customer",
    person: "Person",
    billable: "Billable",
};

/** One bar per day, week or month. */
export const GRAINS = ["day", "week", "month"] as const;
export type Grain = (typeof GRAINS)[number];

/** The person filter's value for time with no person (Entries' too). */
export const UNATTRIBUTED = "-";

export const DEFAULT_DEPTH = 2;
export const MAX_DEPTH = 9;

/** The dashboard's URL state. Empty strings are "no filter". */
export interface DashQuery {
    /** A preset; ignored while `from` or `to` is set (a custom range). */
    period: Preset;
    from: string;
    to: string;
    /** A short name; `UNATTRIBUTED` for none. */
    person: string;
    /** The `customer:` tag's value. */
    customer: string;
    billable: "" | "yes" | "no";
    /** This account or below. */
    account: string;
    /** Account depth for the tree and the top accounts, 1 … MAX_DEPTH. */
    depth: number;
    /** Compare with the previous period of the same length. On by default. */
    compare: boolean;
    split: Split;
    /** "" is chosen from the period's length. */
    grain: Grain | "";
    /** Draw the weekly target (when one is set). On by default. */
    target: boolean;
}

export const DEFAULT_QUERY: DashQuery = {
    period: "this-month",
    from: "",
    to: "",
    person: "",
    customer: "",
    billable: "",
    account: "",
    depth: DEFAULT_DEPTH,
    compare: true,
    split: "branch",
    grain: "",
    target: true,
};

type Params = Pick<URLSearchParams, "get">;

const oneOf = <T extends string>(values: readonly T[], text: string): T | null => ((values as readonly string[]).includes(text) ? (text as T) : null);

const param = (params: Params, name: string): string => (params.get(name) ?? "").trim();

/** The dashboard's state from its URL; unreadable values fall back to the defaults. */
export function parseDashQuery(params: Params): DashQuery {
    const date = (name: string): string => {
        const value = param(params, name);
        return isIsoDate(value) ? value : "";
    };
    const billable = param(params, "billable");
    const depth = Number.parseInt(param(params, "depth"), 10);
    return {
        period: oneOf(PRESETS, param(params, "period")) ?? DEFAULT_QUERY.period,
        from: date("from"),
        to: date("to"),
        person: param(params, "person"),
        customer: param(params, "customer"),
        billable: billable === "yes" || billable === "no" ? billable : "",
        account: param(params, "account"),
        depth: Number.isInteger(depth) && depth >= 1 ? Math.min(depth, MAX_DEPTH) : DEFAULT_DEPTH,
        compare: param(params, "compare") !== "off",
        split: oneOf(SPLITS, param(params, "split")) ?? DEFAULT_QUERY.split,
        grain: oneOf(GRAINS, param(params, "grain")) ?? "",
        target: param(params, "target") !== "off",
    };
}

/** The state as its URL query, without the `?`: only what isn't the default, in a fixed order. */
export function dashQueryString(query: DashQuery): string {
    const out = new URLSearchParams();
    const custom = query.from !== "" || query.to !== "";
    if (!custom && query.period !== DEFAULT_QUERY.period) out.set("period", query.period);
    if (query.from !== "") out.set("from", query.from);
    if (query.to !== "") out.set("to", query.to);
    if (query.person.trim() !== "") out.set("person", query.person.trim());
    if (query.customer.trim() !== "") out.set("customer", query.customer.trim());
    if (query.billable !== "") out.set("billable", query.billable);
    if (query.account.trim() !== "") out.set("account", query.account.trim());
    if (query.depth !== DEFAULT_DEPTH) out.set("depth", String(query.depth));
    if (!query.compare) out.set("compare", "off");
    if (query.split !== DEFAULT_QUERY.split) out.set("split", query.split);
    if (query.grain !== "") out.set("grain", query.grain);
    if (!query.target) out.set("target", "off");
    return out.toString();
}

/** Whether any filter narrows the figures (the period, depth and chart options are not filters). */
export const isFiltered = (query: DashQuery): boolean =>
    [query.person.trim(), query.customer.trim(), query.billable, query.account.trim()].some((v) => v !== "");

/** Every filter cleared; the period and the view options kept. */
export const clearFilters = (query: DashQuery): DashQuery => ({...query, person: "", customer: "", billable: "", account: ""});

/** How a period steps to the one before or after it: by whole months (the day kept), or by days. */
export type Step = {unit: "month"; n: number} | {unit: "day"; n: number};

/** The dates a figure covers. */
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

/** A preset's dates on `today`. */
export function presetRange(preset: Preset, today: string): Range {
    const week: Step = {unit: "day", n: 7};
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
            const first = quarterStart(today);
            return range(first, addDays(addMonths(first, 3), -1), {unit: "month", n: 3}, today);
        }
        case "ytd":
            return range(`${today.slice(0, 4)}-01-01`, today, {unit: "month", n: 12}, today);
        case "last-12-months":
            return range(addMonths(monthStart(today), -11), monthEnd(today), {unit: "month", n: 12}, today);
    }
}

/**
 * The dates `query` covers on `today`. A custom range with one end missing
 * is open on that side as far as the other preset would go: no `from` is the
 * 30 days up to `to`, no `to` runs to today.
 */
export function resolveRange(query: DashQuery, today: string): Range {
    if (query.from === "" && query.to === "") return presetRange(query.period, today);
    const to = query.to !== "" ? query.to : today < query.from ? query.from : today;
    const from = query.from !== "" ? query.from : addDays(to, -29);
    const [a, b] = from <= to ? [from, to] : [to, from];
    return range(a, b, stepOf(a, b), today);
}

/** The period just before `current`, the same length, to the same point in it. */
export function previousRange(current: Range): Range {
    const from = shift(current.from, current.step, -1);
    const end = shift(current.end, current.step, -1);
    return {from, to: end, end, step: current.step};
}

/**
 * The period `direction` steps before (-1) or after (+1) the one shown, as a
 * new query: a preset when the result is one (stepping back from This month
 * is Last month), else a custom range.
 */
export function stepPeriod(query: DashQuery, today: string, direction: -1 | 1): DashQuery {
    const current = resolveRange(query, today);
    const from = shift(current.from, current.step, direction);
    // A month's last day steps to a month's last day (addMonths keeps it one).
    const to = shift(current.to, current.step, direction);
    const preset = PRESETS.find((p) => {
        const r = presetRange(p, today);
        return r.from === from && r.to === to;
    });
    return preset === undefined ? {...query, from, to} : {...query, period: preset, from: "", to: ""};
}

/** The grain chosen from a period's length: days up to a month, weeks up to about six months, then months. */
export function autoGrain(r: Pick<Range, "from" | "to">): Grain {
    const days = daysBetween(r.from, r.to) + 1;
    if (days <= 31) return "day";
    if (days <= 190) return "week";
    return "month";
}

export const grainOf = (query: DashQuery, r: Range): Grain => (query.grain === "" ? autoGrain(r) : query.grain);

/** The filters of `query` over `r`, as `GET /api/time/summary` takes them. */
export function apiFilter(query: DashQuery, r: Pick<Range, "from" | "to">): Omit<SummaryParams, "group"> {
    const person = query.person.trim();
    const customer = query.customer.trim();
    const account = query.account.trim();
    return {
        from: r.from,
        to: r.to,
        ...(person === "" ? {} : {person: person === UNATTRIBUTED ? "" : person}),
        ...(customer === "" ? {} : {customer}),
        ...(query.billable === "" ? {} : {billable: query.billable === "yes"}),
        ...(account === "" ? {} : {account}),
    };
}

/** What a drill-down narrows, beyond the page's filters. */
export interface Drill {
    from?: string;
    to?: string;
    account?: string;
    customer?: string;
    /** A short name, or `""` for unattributed time. */
    person?: string;
    billable?: "yes" | "no";
    /** Entries' billing state: `unbilled`, `invoiced`, … */
    billed?: string;
    /** Text in the note. */
    q?: string;
}

/** Entries' URL query (no `?`) for the page's filters over `r`, narrowed by `drill`. */
export function entriesQuery(query: DashQuery, r: Pick<Range, "from" | "to">, drill: Drill = {}): string {
    const out = new URLSearchParams();
    out.set("from", drill.from ?? r.from);
    out.set("to", drill.to ?? r.to);
    const person = drill.person === undefined ? query.person.trim() : drill.person === "" ? UNATTRIBUTED : drill.person;
    if (person !== "") out.set("person", person);
    const customer = drill.customer ?? query.customer.trim();
    if (customer !== "") out.set("customer", customer);
    const billable = drill.billable ?? query.billable;
    if (billable !== "") out.set("billable", billable);
    if (drill.billed !== undefined) out.set("billed", drill.billed);
    const account = drill.account ?? query.account.trim();
    if (account !== "") out.set("account", account);
    if (drill.q !== undefined && drill.q !== "") out.set("q", drill.q);
    return out.toString();
}
