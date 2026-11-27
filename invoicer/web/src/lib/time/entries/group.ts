// Entries' three views over one list (T-entries): by day (the timesheet,
// with day subtotals), by account, and the weekly grid (accounts × seven
// days), plus what an edited grid cell writes and the day-over-24-hours
// check. Pure, so each is unit-tested without the page.
import type {EntryView} from "../api/entries.js";
import {formatDuration, minutesText} from "../duration.js";
import {weekDates} from "../dates.js";

/** A run of entries under one heading, with their total. */
export interface EntryGroup {
    /** The date (day view) or account (account view). */
    key: string;
    entries: EntryView[];
    seconds: number;
}

const sum = (entries: readonly EntryView[]): number => entries.reduce((total, e) => total + e.quantity.seconds, 0);

function groupBy(entries: readonly EntryView[], keyOf: (e: EntryView) => string): EntryGroup[] {
    const groups = new Map<string, EntryView[]>();
    for (const entry of entries) {
        const key = keyOf(entry);
        const list = groups.get(key);
        if (list === undefined) groups.set(key, [entry]);
        else list.push(entry);
    }
    return [...groups].map(([key, list]) => ({key, entries: list, seconds: sum(list)}));
}

/** By day, newest first; within a day, the files' order. */
export function groupByDay(entries: readonly EntryView[]): EntryGroup[] {
    return groupBy(entries, (e) => e.date).sort((a, b) => (a.key < b.key ? 1 : a.key > b.key ? -1 : 0));
}

/** By account, A–Z; within one, newest first. */
export function groupByAccount(entries: readonly EntryView[]): EntryGroup[] {
    return groupBy(entries, (e) => e.account)
        .sort((a, b) => a.key.localeCompare(b.key))
        .map((g) => ({...g, entries: [...g.entries].sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0))}));
}

/** The groups' entries in the order the page shows them: what `j`/`k` walk. */
export const flatten = (groups: readonly EntryGroup[]): EntryView[] => groups.flatMap((g) => g.entries);

/** One cell of the week grid: an account's time on one day. */
export interface WeekCell {
    date: string;
    entries: EntryView[];
    seconds: number;
}

/** One account's row of the week grid. */
export interface WeekRow {
    account: string;
    cells: WeekCell[];
    seconds: number;
}

/** The week grid (accounts × the seven days from `monday`). */
export interface WeekGrid {
    days: string[];
    rows: WeekRow[];
    /** Each day's total. */
    dayTotals: number[];
    seconds: number;
}

/** The grid for the week starting `monday`; `extra` accounts get a row even with no time (one just added). */
export function weekGrid(entries: readonly EntryView[], monday: string, extra: readonly string[] = []): WeekGrid {
    const days = weekDates(monday);
    const inWeek = entries.filter((e) => days.includes(e.date));
    const accounts = [...new Set([...inWeek.map((e) => e.account), ...extra])].sort((a, b) => a.localeCompare(b));
    const rows = accounts.map((account) => {
        const cells = days.map((date) => {
            const here = inWeek.filter((e) => e.account === account && e.date === date);
            return {date, entries: here, seconds: sum(here)};
        });
        return {account, cells, seconds: cells.reduce((t, c) => t + c.seconds, 0)};
    });
    const dayTotals = days.map((_, i) => rows.reduce((t, r) => t + r.cells[i].seconds, 0));
    return {days, rows, dayTotals, seconds: dayTotals.reduce((t, s) => t + s, 0)};
}

/** What an edited cell writes. */
export type CellPlan =
    | {kind: "none"}
    | {kind: "add"; date: string; account: string; duration: string}
    | {kind: "edit"; id: string; duration: string}
    | {kind: "delete"; id: string}
    | {kind: "refuse"; reason: string};

/** A line the grid may change: a timedot line, unlocked, and not yet billed or written off. */
export const isOpenLine = (e: EntryView): boolean => e.format === "timedot" && e.locked === null && e.invoice === null && e.bill === null && e.nobill === null;

/**
 * What setting `cell` (on `account`) to `wanted` seconds writes. More time
 * goes on the day's last open line, or a new line when there is none; less
 * comes off the last open line, removing it at zero. Time on billed, locked
 * or timeclock lines is never touched: a cut that would need it is refused,
 * and so is a cell holding more than one person's time.
 */
export function planCellEdit(cell: WeekCell, account: string, wanted: number): CellPlan {
    const delta = wanted - cell.seconds;
    if (Math.round(delta / 60) === 0) return {kind: "none"};
    const people = new Set(cell.entries.map((e) => e.person ?? ""));
    if (people.size > 1)
        return {kind: "refuse", reason: "This cell holds more than one person's time. Filter to one person, or change the lines in the day view."};
    const open = cell.entries.filter(isOpenLine);
    const line = open.at(-1);
    if (delta > 0) {
        if (line === undefined) return {kind: "add", date: cell.date, account, duration: minutesText(delta)};
        return {kind: "edit", id: line.id, duration: minutesText(line.quantity.seconds + delta)};
    }
    if (line === undefined || line.quantity.seconds + delta < 0) {
        return {
            kind: "refuse",
            reason: `Only ${formatDuration(line?.quantity.seconds ?? 0)} of this cell can be taken off here; the rest is billed, locked or on other lines. Change them in the day view.`,
        };
    }
    const left = line.quantity.seconds + delta;
    return Math.round(left / 60) === 0 ? {kind: "delete", id: line.id} : {kind: "edit", id: line.id, duration: minutesText(left)};
}

/** A day with more than 24 hours on one person (ABA 93-379): a warning, not a refusal. */
export interface DayWarning {
    date: string;
    person: string | null;
    seconds: number;
}

const DAY = 24 * 3600;

/** Every person-day over 24 hours in `entries`, by date. */
export function overDayWarnings(entries: readonly EntryView[]): DayWarning[] {
    const totals = new Map<string, DayWarning>();
    for (const e of entries) {
        const key = `${e.date}\u0000${e.person ?? ""}`;
        const seen = totals.get(key) ?? {date: e.date, person: e.person, seconds: 0};
        totals.set(key, {...seen, seconds: seen.seconds + e.quantity.seconds});
    }
    return [...totals.values()].filter((w) => w.seconds > DAY).sort((a, b) => a.date.localeCompare(b.date));
}

/** The people in `entries`, A–Z (the person filter's choices). */
export const peopleOf = (entries: readonly EntryView[]): string[] => [...new Set(entries.flatMap((e) => (e.person === null ? [] : [e.person])))].sort();

/** How a line stands for billing, in words, with the invoice number when it is on one. */
export function billingText(e: EntryView): string {
    const parts: string[] = [];
    if (e.invoice !== null) parts.push(`Invoice ${e.invoice}`);
    if (e.bill !== null) parts.push(`Bill ${e.bill}`);
    if (e.nobill !== null) parts.push(e.nobill === "" ? "Not billed" : `Not billed: ${e.nobill}`);
    if (parts.length > 0) return parts.join(" · ");
    return e.billed === "unbilled" ? "Unbilled" : "Not billable";
}
