// Overview's URL state (plan 46 T-dash): the Time area's shared filters
// (`../filters/query.ts`: the period, person, customer, billable, billing
// state, account and search, the same on Entries) and Overview's own view
// options after them: `depth`, `compare=off`, `split`, `grain` and
// `target=off`. A tab switch keeps the filters and drops the options.
//
// Every figure drills into Entries with the same filters, narrowed by what
// the figure counts (a day, an account, unbilled time…).
import type {SummaryParams} from "../api/summary.js";
import {daysBetween} from "../dates.js";
import {
    DEFAULT_FILTERS,
    UNATTRIBUTED,
    filterParams,
    filtersOf,
    filtersQueryString,
    oneOf,
    param,
    parseFilters,
    type BilledState,
    type Params,
    type Range,
    type TimeFilters,
} from "../filters/query.js";

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

export const DEFAULT_DEPTH = 2;
export const MAX_DEPTH = 9;

/** Overview's view options. */
export interface DashView {
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

/** Overview's URL state: the shared filters and its view options. */
export type DashQuery = TimeFilters & DashView;

export const DEFAULT_QUERY: DashQuery = {
    ...DEFAULT_FILTERS,
    depth: DEFAULT_DEPTH,
    compare: true,
    split: "branch",
    grain: "",
    target: true,
};

/** Overview's state from its URL; unreadable values fall back to the defaults. */
export function parseDashQuery(params: Params): DashQuery {
    const depth = Number.parseInt(param(params, "depth"), 10);
    return {
        ...parseFilters(params),
        depth: Number.isInteger(depth) && depth >= 1 ? Math.min(depth, MAX_DEPTH) : DEFAULT_DEPTH,
        compare: param(params, "compare") !== "off",
        split: oneOf(SPLITS, param(params, "split")) ?? DEFAULT_QUERY.split,
        grain: oneOf(GRAINS, param(params, "grain")) ?? "",
        target: param(params, "target") !== "off",
    };
}

/** The state as its URL query, without the `?`: the filters, then the options that aren't the default. */
export function dashQueryString(query: DashQuery): string {
    const out = filterParams(query);
    if (query.depth !== DEFAULT_DEPTH) out.set("depth", String(query.depth));
    if (!query.compare) out.set("compare", "off");
    if (query.split !== DEFAULT_QUERY.split) out.set("split", query.split);
    if (query.grain !== "") out.set("grain", query.grain);
    if (!query.target) out.set("target", "off");
    return out.toString();
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
export function apiFilter(query: TimeFilters, r: Pick<Range, "from" | "to">): Omit<SummaryParams, "group"> {
    const person = query.person.trim();
    const customer = query.customer.trim();
    const account = query.account.trim();
    const q = query.q.trim();
    return {
        from: r.from,
        to: r.to,
        ...(person === "" ? {} : {person: person === UNATTRIBUTED ? "" : person}),
        ...(customer === "" ? {} : {customer}),
        ...(query.billable === "" ? {} : {billable: query.billable === "yes"}),
        ...(query.billed === "" ? {} : {billed: query.billed}),
        ...(account === "" ? {} : {account}),
        ...(q === "" ? {} : {q}),
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
    billed?: BilledState;
    /** Text in the note. */
    q?: string;
}

/**
 * Entries' URL query (no `?`) for the page's filters, narrowed by `drill`.
 * The period goes across as it is (a preset stays one, so Entries' `[`/`]`
 * step from it), unless the figure counts its own dates.
 */
export function entriesQuery(query: DashQuery, drill: Drill = {}): string {
    const filters = filtersOf(query);
    const dated = drill.from !== undefined || drill.to !== undefined;
    return filtersQueryString({
        ...filters,
        ...(dated ? {from: drill.from ?? "", to: drill.to ?? ""} : {}),
        ...(drill.account === undefined ? {} : {account: drill.account}),
        ...(drill.customer === undefined ? {} : {customer: drill.customer}),
        ...(drill.person === undefined ? {} : {person: drill.person === "" ? UNATTRIBUTED : drill.person}),
        ...(drill.billable === undefined ? {} : {billable: drill.billable}),
        ...(drill.billed === undefined ? {} : {billed: drill.billed}),
        ...(drill.q === undefined || drill.q === "" ? {} : {q: drill.q}),
    });
}
