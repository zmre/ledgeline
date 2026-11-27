// Entries' URL state (plan 46 T-entries): the Time area's shared filters
// (`../filters/query.ts`, the same on Overview) and Entries' own view
// options after them: `group` (`day`, the default; `week`; `account`) and
// `week` (the week grid's Monday; absent is this week). The server filters
// (`GET /api/time/entries`); `entriesApiQuery` turns the page's state into
// its parameters, resolving a period preset against today.
import {addDays, mondayOf} from "../dates.js";
import {
    DEFAULT_FILTERS,
    dateParam,
    filterParams,
    isCustom,
    oneOf,
    param,
    parseFilters,
    presetRange,
    UNATTRIBUTED,
    type Params,
    type Period,
    type TimeFilters,
} from "../filters/query.js";

export const GROUPS = ["day", "week", "account"] as const;
export type Grouping = (typeof GROUPS)[number];

/** Entries' view options. */
export interface EntriesView {
    group: Grouping;
    /** The week grid's Monday; "" is this week. */
    week: string;
}

/** Entries' URL state: the shared filters and its view options. */
export type EntriesQuery = TimeFilters & EntriesView;

export const DEFAULT_ENTRIES_QUERY: EntriesQuery = {...DEFAULT_FILTERS, group: "day", week: ""};

/** Entries' state from its URL; unreadable values are dropped. */
export function parseEntriesQuery(params: Params): EntriesQuery {
    const week = dateParam(params, "week");
    return {
        ...parseFilters(params),
        group: oneOf(GROUPS, param(params, "group")) ?? "day",
        week: week === "" ? "" : mondayOf(week),
    };
}

/** Entries' state as its URL query, without the `?`: the filters, then the options that aren't the default. */
export function entriesQueryString(query: EntriesQuery): string {
    const out = filterParams(query);
    if (query.group !== "day") out.set("group", query.group);
    if (query.group === "week" && query.week !== "") out.set("week", query.week);
    return out.toString();
}

/** The dates a preset covers on `today` (null ends: unbounded; All time is both). */
export function periodRange(period: Period, today: string): {from: string | null; to: string | null} {
    if (period === "all") return {from: null, to: null};
    const {from, to} = presetRange(period, today);
    return {from, to};
}

/** The week grid's Monday on `today`. */
export const shownWeek = (query: EntriesQuery, today: string): string => (query.week === "" ? mondayOf(today) : query.week);

/** The dates the view covers: the grid's week, a custom range (open where an end is missing), or the preset. */
export function shownRange(query: EntriesQuery, today: string): {from: string | null; to: string | null} {
    if (query.group === "week") {
        const monday = shownWeek(query, today);
        return {from: monday, to: addDays(monday, 6)};
    }
    if (isCustom(query)) return {from: query.from === "" ? null : query.from, to: query.to === "" ? null : query.to};
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
