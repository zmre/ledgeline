// The time dashboard's figures (plan 46 T-dash), computed from one "fact
// table" per period: `GET /api/time/summary` grouped by account, customer,
// person, billable, billed and day. Every part but the day is a function of
// the entry's account or its own tags, so the rows number about one per
// account, person and day; everything on the page (tiles, bars at any grain
// and split, the tree at any depth, the top lists) is a sum over them, so
// switching the split, the grain or the depth asks the server for nothing.
//
// Pure: no runes, no DOM. Durations are whole seconds throughout and shown
// the way the server shows them (`1h 05m`, `45m`), rounded to the minute.
import type {Summary, SummaryGroup, SummaryRow} from "../api/summary.js";
import {
    addDays,
    dayMonth,
    dayMonthYear,
    eachDay,
    isWorkingDay,
    mondayOf,
    monthEnd,
    monthName,
    monthStart,
    monthYear,
    rangeText,
    weekdayName,
    workingDays,
} from "../dates.js";
import type {Range} from "../filters/query.js";
import type {Drill, Grain, Split} from "./query.js";

/** The fact table's grouping, in this order. */
export const FACT_GROUPS: readonly SummaryGroup[] = ["account", "customer", "person", "billable", "billed", "day"];

/** One fact row: the time one person put on one account on one day, in one billing state. */
export interface Fact {
    account: string;
    /** `""`: no customer. */
    customer: string;
    /** `""`: unattributed. */
    person: string;
    billable: boolean;
    /** `unbilled`, `invoiced`, `nobill`, `not-billable`. */
    billed: string;
    date: string;
    seconds: number;
    entries: number;
}

export function factsOf(summary: Summary): Fact[] {
    return summary.rows.map((row) => {
        const [account = "", customer = "", person = "", billable = "", billed = "", date = ""] = row.key;
        return {account, customer, person, billable: billable === "yes", billed, date, seconds: row.quantity.seconds, entries: row.entries};
    });
}

// ── Formatting ──────────────────────────────────────────────────────────

/** `1h 05m`, `45m`, `2h`, `0m`: whole minutes, half up, as the server writes them. */
export function duration(seconds: number): string {
    const minutes = Math.round(Math.abs(seconds) / 60);
    const sign = seconds < 0 && minutes > 0 ? "−" : "";
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    if (h === 0) return `${sign}${m}m`;
    if (m === 0) return `${sign}${h}h`;
    return `${sign}${h}h ${String(m).padStart(2, "0")}m`;
}

/** Hours as a short number for an axis: `0`, `2.5`, `12`. */
export const hoursNumber = (seconds: number): string => String(Math.round((seconds / 3600) * 10) / 10);

/** `68%`; below 1% but not zero, `<1%`. */
export function percent(ratio: number): string {
    if (ratio > 0 && ratio < 0.005) return "<1%";
    return `${Math.round(ratio * 100)}%`;
}

export const count = (n: number, one: string, many = `${one}s`): string => `${n} ${n === 1 ? one : many}`;

// ── Change against the previous period ─────────────────────────────────

export type Direction = "up" | "down" | "flat";

/** A change, for people: `+1h 05m (+12%)`, `−3 pts`, and words for a screen reader. */
export interface Delta {
    direction: Direction;
    /** The visible text. */
    text: string;
    /** "1h 05m more than the previous period". */
    label: string;
}

/**
 * The change from `previous` to `current`. `kind` says how to show it: a
 * duration (with its percentage), percentage points of a share, or a count
 * of days. A change too small to show (under half a minute, half a point)
 * is flat.
 */
export function deltaOf(current: number, previous: number, kind: "duration" | "points" | "days"): Delta {
    const diff = current - previous;
    const shown = kind === "duration" ? Math.round(diff / 60) : kind === "points" ? Math.round(diff * 100) : Math.round(diff);
    const direction: Direction = shown > 0 ? "up" : shown < 0 ? "down" : "flat";
    if (direction === "flat") return {direction, text: "No change", label: "the same as the previous period"};
    const sign = direction === "up" ? "+" : "−";
    const more = direction === "up" ? "more" : "less";
    if (kind === "points") {
        const pts = Math.abs(shown);
        return {
            direction,
            text: `${sign}${pts} pts`,
            label: `${pts} percentage ${pts === 1 ? "point" : "points"} ${direction === "up" ? "higher" : "lower"} than the previous period`,
        };
    }
    if (kind === "days") {
        const n = Math.abs(shown);
        return {direction, text: `${sign}${n}`, label: `${count(n, "day")} ${direction === "up" ? "more" : "fewer"} than the previous period`};
    }
    const abs = duration(Math.abs(diff));
    const pct = previous > 0 ? ` (${sign}${percent(Math.abs(diff) / previous)})` : "";
    return {direction, text: `${sign}${abs}${pct}`, label: `${abs} ${more} than the previous period`};
}

// ── KPI tiles ───────────────────────────────────────────────────────────

export interface Kpis {
    total: number;
    billable: number;
    /** Billable over total; null with no time. */
    billableShare: number | null;
    /** Total over working days to the period's end; null with none. */
    perWorkingDay: number | null;
    /** Monday to Friday, `from`..=`end`. */
    workingDays: number;
    /** Working days with any time. */
    trackedWorkingDays: number;
    /** Saturdays and Sundays with any time. */
    weekendDays: number;
    unbilled: number;
    entries: number;
}

export function kpisOf(facts: readonly Fact[], r: Pick<Range, "from" | "end">): Kpis {
    let total = 0;
    let billable = 0;
    let unbilled = 0;
    let entries = 0;
    const days = new Set<string>();
    for (const f of facts) {
        total += f.seconds;
        entries += f.entries;
        if (f.billable) billable += f.seconds;
        if (f.billed === "unbilled") unbilled += f.seconds;
        if (f.seconds > 0 && f.date >= r.from && f.date <= r.end) days.add(f.date);
    }
    const working = workingDays(r.from, r.end);
    let trackedWorkingDays = 0;
    for (const d of days) if (isWorkingDay(d)) trackedWorkingDays += 1;
    return {
        total,
        billable,
        billableShare: total > 0 ? billable / total : null,
        perWorkingDay: working > 0 ? total / working : null,
        workingDays: working,
        trackedWorkingDays,
        weekendDays: days.size - trackedWorkingDays,
        unbilled,
        entries,
    };
}

// ── Sums ────────────────────────────────────────────────────────────────

export interface Sum {
    seconds: number;
    entries: number;
}

/** Facts summed by `keyOf`, in first-seen order. */
export function sumBy<T>(facts: readonly T[], keyOf: (fact: T) => string, value: (fact: T) => Sum): Map<string, Sum> {
    const out = new Map<string, Sum>();
    for (const f of facts) {
        const key = keyOf(f);
        const v = value(f);
        const slot = out.get(key);
        if (slot === undefined) out.set(key, {seconds: v.seconds, entries: v.entries});
        else {
            slot.seconds += v.seconds;
            slot.entries += v.entries;
        }
    }
    return out;
}

const factSum = (f: Fact): Sum => ({seconds: f.seconds, entries: f.entries});

/** An account cut to `depth` segments (`clients:acme:tax` at 2 is `clients:acme`). */
export const cut = (account: string, depth: number): string => account.split(":").slice(0, Math.max(1, depth)).join(":");

export function splitKey(fact: Fact, split: Split): string {
    switch (split) {
        case "branch":
            return cut(fact.account, 1);
        case "customer":
            return fact.customer;
        case "person":
            return fact.person;
        case "billable":
            return fact.billable ? "yes" : "no";
    }
}

/** A split's key for people. Italic: not a real name ("No customer"). */
export function splitLabel(split: Split, key: string): {label: string; italic: boolean} {
    if (split === "billable") return {label: key === "yes" ? "Billable" : "Non-billable", italic: false};
    if (key === "") return {label: split === "customer" ? "No customer" : split === "person" ? "Unattributed" : "(none)", italic: true};
    return {label: key, italic: false};
}

/** What clicking a split's key narrows Entries to; null where Entries can't say it (no customer). */
export function splitDrill(split: Split, key: string): Drill | null {
    switch (split) {
        case "branch":
            return {account: key};
        case "customer":
            return key === "" ? null : {customer: key};
        case "person":
            return {person: key};
        case "billable":
            return {billable: key === "yes" ? "yes" : "no"};
    }
}

// ── Colours ─────────────────────────────────────────────────────────────

/**
 * Categorical slots used: 7 named series plus Other, the dataviz ceiling of
 * eight. Every theme defines at least ten `--chart-N`, validated in order
 * (packages/ui/src/theme.css), so slots 1–7 always exist.
 */
export const SERIES_SLOTS = 7;

export const slotColor = (slot: number | null): string => (slot === null ? "var(--chart-other)" : `var(--chart-${slot + 1})`);

/**
 * Each key's colour slot, from an order that does not depend on the filters
 * or the period: colour follows the entity, never its rank in the view, so
 * narrowing the page never repaints a series. `order` is that stable order
 * (most hours across all time first); keys it doesn't know come after it,
 * most hours here first. "No customer"/"Unattributed" (`""`) never take a
 * slot: they draw in the Other grey, labelled.
 */
export function slotsFor(order: readonly string[], seen: ReadonlyMap<string, Sum>): Map<string, number> {
    const known = order.filter((k) => k !== "");
    const extra = [...seen.entries()]
        .filter(([k]) => k !== "" && !known.includes(k))
        .sort((a, b) => b[1].seconds - a[1].seconds || a[0].localeCompare(b[0]))
        .map(([k]) => k);
    const slots = new Map<string, number>();
    [...known, ...extra].slice(0, SERIES_SLOTS).forEach((k, i) => slots.set(k, i));
    return slots;
}

// ── Hours over time ─────────────────────────────────────────────────────

export interface Bucket {
    /** Its first day. */
    from: string;
    /** Its last day, inside the period. */
    to: string;
    /** The axis label: `26`, `Nov 23`, `Nov`. */
    tick: string;
    /** For a tooltip: "Thu, Nov 26, 2026", "Week of Nov 23", "Nov 2026". */
    title: string;
    /** Whether it is a Saturday or Sunday (day grain only). */
    weekend: boolean;
}

/** The period's buckets at `grain`, the first and last clipped to it. */
export function bucketsOf(r: Pick<Range, "from" | "to">, grain: Grain): Bucket[] {
    if (grain === "day") {
        return eachDay(r.from, r.to).map((d) => ({
            from: d,
            to: d,
            tick: d.endsWith("-01") ? monthName(d) : String(Number(d.slice(8))),
            title: `${weekdayName(d)}, ${dayMonthYear(d)}`,
            weekend: !isWorkingDay(d),
        }));
    }
    const out: Bucket[] = [];
    let start = grain === "week" ? mondayOf(r.from) : monthStart(r.from);
    while (start <= r.to) {
        const last = grain === "week" ? addDays(start, 6) : monthEnd(start);
        const from = start < r.from ? r.from : start;
        const to = last > r.to ? r.to : last;
        out.push(
            grain === "week"
                ? {from, to, tick: dayMonth(from), title: `Week of ${dayMonth(start)} (${rangeText(from, to)})`, weekend: false}
                : {from, to, tick: start.endsWith("-01-01") || out.length === 0 ? monthYear(start) : monthName(start), title: monthYear(start), weekend: false}
        );
        start = addDays(last, 1);
    }
    return out;
}

export interface ChartSeries {
    key: string;
    label: string;
    italic: boolean;
    color: string;
    /** Seconds per bucket. */
    values: number[];
    total: number;
    /** Narrows Entries to this series; null where Entries can't. */
    drill: Drill | null;
}

export interface Chart {
    buckets: Bucket[];
    /** Largest first, Other last. */
    series: ChartSeries[];
    /** Seconds per bucket, every series. */
    totals: number[];
}

export const OTHER_KEY = "\u0000other";

/**
 * Stacked bars: `facts` per bucket of `r` at `grain`, split by `split`. At
 * most SERIES_SLOTS named series, those with a colour slot, largest first;
 * the rest (and any key without a slot) fold into Other.
 */
export function chartOf(facts: readonly Fact[], r: Pick<Range, "from" | "to">, grain: Grain, split: Split, order: readonly string[]): Chart {
    const buckets = bucketsOf(r, grain);
    const index = new Map<string, number>();
    buckets.forEach((b, i) => {
        for (const d of eachDay(b.from, b.to)) index.set(d, i);
    });
    const byKey = sumBy(facts, (f) => splitKey(f, split), factSum);
    const slots =
        split === "billable"
            ? new Map([
                  ["yes", 0],
                  ["no", 1],
              ])
            : slotsFor(order, byKey);
    const named = [...byKey.entries()]
        .filter(([k, v]) => v.seconds !== 0 && slots.has(k))
        .sort((a, b) => b[1].seconds - a[1].seconds || a[0].localeCompare(b[0]))
        .map(([k]) => k);
    const rows = new Map<string, number[]>(named.map((k) => [k, buckets.map(() => 0)]));
    const other = buckets.map(() => 0);
    const totals = buckets.map(() => 0);
    for (const f of facts) {
        const at = index.get(f.date);
        if (at === undefined) continue;
        const key = splitKey(f, split);
        (rows.get(key) ?? other)[at] += f.seconds;
        totals[at] += f.seconds;
    }
    const series: ChartSeries[] = named.map((key) => {
        const values = rows.get(key) ?? [];
        return {
            key,
            ...splitLabel(split, key),
            color: slotColor(slots.get(key) ?? null),
            values,
            total: values.reduce((a, b) => a + b, 0),
            drill: splitDrill(split, key),
        };
    });
    const otherTotal = other.reduce((a, b) => a + b, 0);
    if (otherTotal !== 0) {
        const lone = [...byKey.entries()].filter(([k, v]) => v.seconds !== 0 && !slots.has(k));
        // One key alone in Other ("No customer") keeps its own name.
        const only = lone.length === 1 ? splitLabel(split, lone[0][0]) : {label: "Other", italic: false};
        series.push({
            key: OTHER_KEY,
            ...only,
            color: slotColor(null),
            values: other,
            total: otherTotal,
            drill: lone.length === 1 ? splitDrill(split, lone[0][0]) : null,
        });
    }
    return {buckets, series, totals};
}

/** Round axis ticks from 0 past `max` seconds, in whole or half hours: about four steps. */
export function hourTicks(max: number): number[] {
    const hours = max / 3600;
    if (hours <= 0) return [0, 3600];
    const steps = [0.5, 1, 2, 4, 5, 8, 10, 20, 25, 40, 50, 100, 200, 250, 500, 1000];
    const step = steps.find((s) => hours / s <= 4) ?? Math.ceil(hours / 4 / 1000) * 1000;
    const top = Math.ceil(hours / step) * step;
    return Array.from({length: Math.round(top / step) + 1}, (_, i) => i * step * 3600);
}

/** Every k-th tick label, so labels don't collide: about `fit` of them. */
export const tickEvery = (buckets: number, fit: number): number => Math.max(1, Math.ceil(buckets / Math.max(1, fit)));

// ── The account tree ────────────────────────────────────────────────────

export interface TreeNode {
    account: string;
    /** Its last segment. */
    name: string;
    /** 0 for a top-level branch. */
    depth: number;
    /** It and everything below it. */
    seconds: number;
    billable: number;
    /** The previous period's, it and below. */
    previous: number;
    children: TreeNode[];
}

/**
 * `bal --tree` with roll-ups: every account and its ancestors, each with its
 * own time plus its children's, billable hours and the previous period's
 * hours. An account with time only in the previous period is kept (at 0h)
 * so its change shows. Siblings sort by hours, most first, then by name.
 */
export function treeOf(current: readonly Fact[], previous: readonly Fact[] = []): TreeNode[] {
    const nodes = new Map<string, TreeNode>();
    const roots: TreeNode[] = [];
    const node = (account: string): TreeNode => {
        const found = nodes.get(account);
        if (found !== undefined) return found;
        const parts = account.split(":");
        const made: TreeNode = {account, name: parts[parts.length - 1], depth: parts.length - 1, seconds: 0, billable: 0, previous: 0, children: []};
        nodes.set(account, made);
        if (parts.length === 1) roots.push(made);
        else node(parts.slice(0, -1).join(":")).children.push(made);
        return made;
    };
    const add = (f: Fact, into: (n: TreeNode) => void): void => {
        const parts = f.account.split(":");
        for (let i = 1; i <= parts.length; i++) into(node(parts.slice(0, i).join(":")));
    };
    for (const f of current)
        add(f, (n) => {
            n.seconds += f.seconds;
            if (f.billable) n.billable += f.seconds;
        });
    for (const f of previous) add(f, (n) => (n.previous += f.seconds));
    const sort = (list: TreeNode[]): void => {
        list.sort((a, b) => b.seconds - a.seconds || b.previous - a.previous || a.name.localeCompare(b.name));
        for (const n of list) sort(n.children);
    };
    sort(roots);
    return roots;
}

export interface TreeRow {
    node: TreeNode;
    expandable: boolean;
    expanded: boolean;
}

/**
 * The rows to show: a node's children show when it is expanded. Up to the
 * page's depth everything is expanded; a row's own toggle flips that, so a
 * branch can be closed above the depth, or opened below it.
 */
export function visibleRows(roots: readonly TreeNode[], depth: number, toggled: ReadonlySet<string>): TreeRow[] {
    const out: TreeRow[] = [];
    const walk = (list: readonly TreeNode[]): void => {
        for (const node of list) {
            const expandable = node.children.length > 0;
            const expanded = expandable && node.depth + 1 < depth !== toggled.has(node.account);
            out.push({node, expandable, expanded});
            if (expanded) walk(node.children);
        }
    };
    walk(roots);
    return out;
}

// ── Top N ───────────────────────────────────────────────────────────────

export interface TopItem {
    key: string;
    label: string;
    italic: boolean;
    seconds: number;
    entries: number;
    /** Of the list's total. */
    share: number;
    /** The previous period's; null with comparison off. */
    previous: number | null;
    /** Narrows Entries to it; null where Entries can't. */
    drill: Drill | null;
}

export interface TopList {
    items: TopItem[];
    /** The rest, summed; null when nothing is left over. */
    other: {keys: number; seconds: number; entries: number; previous: number | null} | null;
    total: number;
}

export interface TopOptions {
    n?: number;
    /** Rank by hours (the default) or by how many entries. */
    by?: "seconds" | "entries";
    label?: (key: string) => {label: string; italic: boolean};
    drill?: (key: string) => Drill | null;
}

export const TOP_N = 10;

/**
 * The largest `n` of `current` (by hours, or by entries), with the rest as
 * Other and each item's previous-period figure (`previous`, null with
 * comparison off). Ties break by key, so the order is stable.
 */
export function topOf(current: ReadonlyMap<string, Sum>, previous: ReadonlyMap<string, Sum> | null, options: TopOptions = {}): TopList {
    const n = options.n ?? TOP_N;
    const by = options.by ?? "seconds";
    const rows = [...current.entries()]
        .filter(([, v]) => v.seconds !== 0 || v.entries !== 0)
        .sort((a, b) => b[1][by] - a[1][by] || b[1].seconds - a[1].seconds || a[0].localeCompare(b[0]));
    const total = rows.reduce((t, [, v]) => t + v.seconds, 0);
    const prev = (key: string): number | null => (previous === null ? null : (previous.get(key)?.seconds ?? 0));
    const items = rows.slice(0, n).map(([key, v]) => ({
        key,
        ...(options.label?.(key) ?? {label: key, italic: false}),
        seconds: v.seconds,
        entries: v.entries,
        share: total > 0 ? v.seconds / total : 0,
        previous: prev(key),
        drill: options.drill === undefined ? {} : options.drill(key),
    }));
    const rest = rows.slice(n);
    const other =
        rest.length === 0
            ? null
            : {
                  keys: rest.length,
                  seconds: rest.reduce((t, [, v]) => t + v.seconds, 0),
                  entries: rest.reduce((t, [, v]) => t + v.entries, 0),
                  previous: previous === null ? null : rest.reduce((t, [k]) => t + (previous.get(k)?.seconds ?? 0), 0),
              };
    return {items, other, total};
}

/** Accounts cut to `depth`, summed. */
export const accountsAt = (facts: readonly Fact[], depth: number): Map<string, Sum> => sumBy(facts, (f) => cut(f.account, depth), factSum);
export const byCustomer = (facts: readonly Fact[]): Map<string, Sum> => sumBy(facts, (f) => f.customer, factSum);
export const byPerson = (facts: readonly Fact[]): Map<string, Sum> => sumBy(facts, (f) => f.person, factSum);

/** A `group=note` summary's rows, the empty note dropped: the most frequent notes are notes. */
export function notesOf(rows: readonly SummaryRow[]): Map<string, Sum> {
    return sumBy(
        rows.filter((r) => (r.key[0] ?? "") !== ""),
        (r) => r.key[0] ?? "",
        (r) => ({seconds: r.quantity.seconds, entries: r.entries})
    );
}
