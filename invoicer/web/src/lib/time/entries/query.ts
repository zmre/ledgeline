// Entries' filters, grouping and period as a URL query (the invoicer's list
// contract, PLAN 5b S1–S4): one parse and one serialize, so a link, a reload
// and Back/Forward all land on the same view. The server filters
// (`GET /api/time/entries`); `entriesApiQuery` turns the page's state into
// its parameters, resolving a period preset against today.
//
// URL parameters: `period` (a preset; absent is this month), `from`/`to`
// (a custom range, which wins over a preset), `person` (`-` is
// unattributed), `customer`, `billable` (`yes`/`no`), `billed`, `account`,
// `q`, `group` (`day`, the default; `week`; `account`) and `week` (the
// week grid's Monday; absent is this week).
import {addDays, isIsoDate, mondayOf, monthRange} from "../dates.js";

export const PERIODS = ["this-week", "last-week", "this-month", "last-month", "this-year", "last-year", "all"] as const;
export type Period = (typeof PERIODS)[number];

export const PERIOD_LABELS: Record<Period, string> = {
    "this-week": "This week",
    "last-week": "Last week",
    "this-month": "This month",
    "last-month": "Last month",
    "this-year": "This year",
    "last-year": "Last year",
    all: "All time",
};

export const BILLED_STATES = ["unbilled", "invoiced", "nobill", "not-billable", "billed"] as const;
export type BilledState = (typeof BILLED_STATES)[number];

export const BILLED_LABELS: Record<BilledState, string> = {
    unbilled: "Unbilled",
    invoiced: "On an invoice",
    nobill: "Not billed (written off)",
    "not-billable": "Not billable",
    billed: "Invoiced or written off",
};

export const GROUPS = ["day", "week", "account"] as const;
export type Grouping = (typeof GROUPS)[number];

/** The person filter's value for time with no person. */
export const UNATTRIBUTED = "-";

/** Entries' URL state. Empty strings are "no filter". */
export interface EntriesQuery {
    /** A preset; ignored while `from` or `to` is set. */
    period: Period;
    from: string;
    to: string;
    /** A short name; `UNATTRIBUTED` for none. */
    person: string;
    customer: string;
    billable: "" | "yes" | "no";
    billed: BilledState | "";
    account: string;
    q: string;
    group: Grouping;
    /** The week grid's Monday; "" is this week. */
    week: string;
}

type Params = Pick<URLSearchParams, "get">;

const DEFAULT_PERIOD: Period = "this-month";

const oneOf = <T extends string>(values: readonly T[], text: string, fallback: T | ""): T | "" =>
    (values as readonly string[]).includes(text) ? (text as T) : fallback;

const param = (params: Params, name: string): string => (params.get(name) ?? "").trim();

/** Entries' state from its URL; unreadable values are dropped. */
export function parseEntriesQuery(params: Params): EntriesQuery {
    const date = (name: string): string => {
        const value = param(params, name);
        return isIsoDate(value) ? value : "";
    };
    const billable = param(params, "billable");
    const week = date("week");
    return {
        period: oneOf(PERIODS, param(params, "period"), DEFAULT_PERIOD) as Period,
        from: date("from"),
        to: date("to"),
        person: param(params, "person"),
        customer: param(params, "customer"),
        billable: billable === "yes" || billable === "no" ? billable : "",
        billed: oneOf(BILLED_STATES, param(params, "billed"), ""),
        account: param(params, "account"),
        q: params.get("q") ?? "",
        group: oneOf(GROUPS, param(params, "group"), "day") as Grouping,
        week: week === "" ? "" : mondayOf(week),
    };
}

/** Entries' state as its URL query, without the `?`: only what isn't the default, in a fixed order. */
export function entriesQueryString(query: EntriesQuery): string {
    const out = new URLSearchParams();
    const custom = query.from !== "" || query.to !== "";
    if (!custom && query.period !== DEFAULT_PERIOD) out.set("period", query.period);
    if (query.from !== "") out.set("from", query.from);
    if (query.to !== "") out.set("to", query.to);
    if (query.person.trim() !== "") out.set("person", query.person.trim());
    if (query.customer.trim() !== "") out.set("customer", query.customer.trim());
    if (query.billable !== "") out.set("billable", query.billable);
    if (query.billed !== "") out.set("billed", query.billed);
    if (query.account.trim() !== "") out.set("account", query.account.trim());
    if (query.q.trim() !== "") out.set("q", query.q.trim());
    if (query.group !== "day") out.set("group", query.group);
    if (query.group === "week" && query.week !== "") out.set("week", query.week);
    return out.toString();
}

/** The dates a preset covers on `today` (null ends: unbounded). */
export function periodRange(period: Period, today: string): {from: string | null; to: string | null} {
    const year = today.slice(0, 4);
    switch (period) {
        case "this-week": {
            const monday = mondayOf(today);
            return {from: monday, to: addDays(monday, 6)};
        }
        case "last-week": {
            const monday = addDays(mondayOf(today), -7);
            return {from: monday, to: addDays(monday, 6)};
        }
        case "this-month":
            return monthRange(today);
        case "last-month":
            return monthRange(addDays(monthRange(today).from, -1));
        case "this-year":
            return {from: `${year}-01-01`, to: `${year}-12-31`};
        case "last-year": {
            const last = String(Number(year) - 1);
            return {from: `${last}-01-01`, to: `${last}-12-31`};
        }
        case "all":
            return {from: null, to: null};
    }
}

/** The week grid's Monday on `today`. */
export const shownWeek = (query: EntriesQuery, today: string): string => (query.week === "" ? mondayOf(today) : query.week);

/** The dates the view covers: the grid's week, a custom range, or the preset. */
export function shownRange(query: EntriesQuery, today: string): {from: string | null; to: string | null} {
    if (query.group === "week") {
        const monday = shownWeek(query, today);
        return {from: monday, to: addDays(monday, 6)};
    }
    if (query.from !== "" || query.to !== "") return {from: query.from === "" ? null : query.from, to: query.to === "" ? null : query.to};
    return periodRange(query.period, today);
}

/** `GET /api/time/entries`' query for `query` on `today`. */
export function entriesApiQuery(query: EntriesQuery, today: string): string {
    const out = new URLSearchParams();
    const range = shownRange(query, today);
    if (range.from !== null) out.set("from", range.from);
    if (range.to !== null) out.set("to", range.to);
    const person = query.person.trim();
    if (person !== "") out.set("person", person === UNATTRIBUTED ? "" : person);
    if (query.customer.trim() !== "") out.set("customer", query.customer.trim());
    if (query.billable !== "") out.set("billable", query.billable === "yes" ? "true" : "false");
    if (query.billed !== "") out.set("billed", query.billed);
    if (query.account.trim() !== "") out.set("account", query.account.trim());
    if (query.q.trim() !== "") out.set("q", query.q.trim());
    return out.toString();
}

/** Whether any filter narrows the list (the period and grouping are not filters). */
export function entriesFiltered(query: EntriesQuery): boolean {
    return [query.person.trim(), query.customer.trim(), query.billable, query.billed, query.account.trim(), query.q.trim()].some((v) => v !== "");
}

/** Every filter cleared; the period and grouping kept. */
export function clearEntriesFilters(query: EntriesQuery): EntriesQuery {
    return {...query, person: "", customer: "", billable: "", billed: "", account: "", q: ""};
}

/** The person a write should name: the filter's, unless it is "anyone" or "unattributed". */
export function filterPerson(query: EntriesQuery): string | undefined {
    const person = query.person.trim();
    return person === "" || person === UNATTRIBUTED ? undefined : person;
}
