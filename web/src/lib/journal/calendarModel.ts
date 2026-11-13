// The journal's calendar view (plan 40): the month math and the one-line
// rendering of a transaction in a day cell. Pure TS — no Svelte, no DOM — so
// it is a node test.
//
// The calendar is the SAME filtered journal the table shows, with the date
// range held to exactly one calendar month. The month lives in the filter's
// own `from`/`to` (so the insights panel, the footer and the URL all follow it
// with no second source of truth); this module only says which month a range
// means and how a row reads in a cell.

import type {Problem, Severity} from "#lib/checks/engine.js";
import {maxSeverity} from "#lib/checks/engine.js";
import {isAccountType, type AccountClasses} from "#lib/domain/accountClass.js";
import {add, formatAmount, formatDec, MAX_QUANTITY_DECIMALS, mul, neg, type Dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Transaction} from "#lib/domain/types.js";
import type {JournalRow} from "#lib/journalView/types.js";
import {txnFlowAmounts, writtenPostings} from "./rowModel";

/** A calendar month, `YYYY-MM`. */
export type MonthKey = string;

const MONTH_KEY = /^(\d{4})-(\d{2})$/;

function pad(n: number): string {
    return String(n).padStart(2, "0");
}

function daysIn(year: number, month: number): number {
    return new Date(Date.UTC(year, month, 0)).getUTCDate();
}

function parseMonth(key: MonthKey): [number, number] {
    const match = MONTH_KEY.exec(key);
    if (match === null) throw new Error(`not a month: ${key}`);
    return [Number(match[1]), Number(match[2])];
}

/** The month `date` falls in. */
export function monthOf(date: ISODate): MonthKey {
    return date.slice(0, 7);
}

/** The month's first and last day, inclusive — the filter range the calendar holds. */
export function monthRange(key: MonthKey): {from: ISODate; to: ISODate} {
    const [y, m] = parseMonth(key);
    return {from: `${key}-01`, to: `${key}-${pad(daysIn(y, m))}`};
}

/** The month `delta` months after `key` (negative: earlier). */
export function shiftMonth(key: MonthKey, delta: number): MonthKey {
    const [y, m] = parseMonth(key);
    const index = y * 12 + (m - 1) + delta;
    const year = Math.floor(index / 12);
    return `${year}-${pad(index - year * 12 + 1)}`;
}

/** "November 2026". */
export function monthLabel(key: MonthKey): string {
    const [y, m] = parseMonth(key);
    return `${MONTH_NAMES[m - 1]} ${y}`;
}

const MONTH_NAMES = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** Column headings, Sunday first (the grid's week start). */
export const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;

/** Whether `from`…`to` is exactly one whole calendar month. */
export function isWholeMonth(from: ISODate | null, to: ISODate | null): boolean {
    if (from === null || to === null) return false;
    const range = monthRange(monthOf(from));
    return range.from === from && range.to === to;
}

/**
 * The month a date range means to the calendar.
 *
 * The range's END decides, because nearly every preset ends where the user is
 * looking: "last 90 days" and "year to date" end today, "last year" in its
 * December. An end in the future (this year's 31 December) is pulled back to
 * today when the range has already started, so "this year" opens on the
 * current month rather than eleven months ahead. An open end falls back to the
 * start, and an open range ("all dates") to today.
 */
export function calendarMonth(from: ISODate | null, to: ISODate | null, today: ISODate): MonthKey {
    if (to !== null) return monthOf(to > today && (from === null || from <= today) ? today : to);
    return monthOf(from ?? today);
}

/**
 * The month as Sunday-first weeks of ISO dates, padded out to whole weeks with
 * the neighbouring months' days (the grid greys those).
 */
export function monthGrid(key: MonthKey): ISODate[][] {
    const [y, m] = parseMonth(key);
    const lead = new Date(Date.UTC(y, m - 1, 1)).getUTCDay();
    const count = daysIn(y, m);
    const cells = Math.ceil((lead + count) / 7) * 7;
    const days: ISODate[] = [];
    for (let i = 0; i < cells; i += 1) {
        const at = new Date(Date.UTC(y, m - 1, 1 + i - lead));
        days.push(`${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}`);
    }
    return Array.from({length: cells / 7}, (_, week) => days.slice(week * 7, week * 7 + 7));
}

/**
 * How a day's `count` items fit `capacity` lines: all of them when they fit,
 * else one line fewer than fits plus an "N more" line. Always at least one
 * line, so a cramped cell still says something is there.
 */
export function fitDay(count: number, capacity: number): {shown: number; more: number} {
    const lines = Math.max(1, capacity);
    if (count <= lines) return {shown: count, more: 0};
    const shown = lines - 1;
    return {shown, more: count - shown};
}

/** A stock buy or sell, for the item's pill. */
export interface CalendarTrade {
    side: "buy" | "sell";
    /** The commodity traded (`AAPL`). */
    symbol: string;
    /** Signed share count as the journal wrote it: `+10`, `-120`. */
    shares: string;
}

/** One transaction as a day cell draws it: text left, rounded amount right. */
export interface CalendarItem {
    /** The tindex the editor opens on. */
    index: number;
    date: ISODate;
    /** The description, or for a trade the symbol and shares. */
    label: string;
    trade: CalendarTrade | null;
    /** Rounded to whole units of the commodity; null when the transaction moves nothing. */
    amount: string | null;
    /** Money in (income, a sale), money out (an expense, a purchase), or neither (a transfer). */
    tone: "positive" | "negative" | "neutral";
    /** The worst finding on the row, for the usual flag dot. */
    severity: Severity | null;
    /** The hover text: the description, the amount and every finding. */
    title: string;
}

/** The first written posting that trades a commodity for another at a cost (`10 AAPL @ $150`). */
function tradeOf(txn: Transaction): {amount: Amount; cash: Dec; cashCommodity: string} | null {
    for (const posting of writtenPostings(txn)) {
        for (const amount of posting.amounts) {
            const cost = amount.cost;
            if (cost === undefined || cost.commodity === amount.commodity || amount.qty.m === 0n) continue;
            const units = amount.qty.m < 0n ? neg(amount.qty) : amount.qty;
            return {amount, cash: cost.per ? mul(units, cost.qty) : cost.qty, cashCommodity: cost.commodity};
        }
    }
    return null;
}

/**
 * The transaction's profit-and-loss effect in `commodity`: income minus
 * expenses over its written postings (positive: money earned). Null when it
 * touches no income or expense account in that commodity — a transfer.
 * Accounts are classified by their declared type, never their name.
 */
function earned(txn: Transaction, commodity: string, classes: AccountClasses): Dec | null {
    let total: Dec | null = null;
    for (const posting of writtenPostings(txn)) {
        if (!isAccountType(posting.account, classes, "revenue") && !isAccountType(posting.account, classes, "expense")) continue;
        for (const amount of posting.amounts) {
            if (amount.commodity !== commodity) continue;
            total = total === null ? neg(amount.qty) : add(total, neg(amount.qty));
        }
    }
    return total;
}

function toneOf(qty: Dec): CalendarItem["tone"] {
    return qty.m > 0n ? "positive" : qty.m < 0n ? "negative" : "neutral";
}

function rounded(commodity: string, qty: Dec, style: AmountStyle): string {
    return formatAmount({commodity, qty, style}, 0);
}

/** The amount a non-trade shows in: the preferred commodity when the row has it, else the first it moves. */
function shownAmount(txn: Transaction, preferred: string): Amount | null {
    const flows = txnFlowAmounts(txn);
    return flows.find((amount) => amount.commodity === preferred) ?? flows[0] ?? null;
}

/**
 * Render one journal row for a day cell.
 *
 * A trade reads `[SELL] AAPL -120   $18,000`: the pill, the symbol and signed
 * share count, and the cash it moved (in for a sale, out for a purchase).
 * Anything else reads `Description   $-42`: income positive, an expense
 * negative, by the income-minus-expenses it books — so a paycheck with tax
 * withheld shows its net. A transfer (no income or expense leg) shows the
 * amount it moved, unsigned.
 */
export function calendarItem(row: JournalRow, opts: {classes: AccountClasses; styleOf: (commodity: string) => AmountStyle; preferred: string}): CalendarItem {
    const {txn} = row;
    const description = txn.description.trim() === "" ? "(no description)" : txn.description;
    const severity = maxSeverity(row.flags);
    const findings = row.flags.map((problem: Problem) => problem.message);
    const base = {index: txn.index, date: txn.date, severity};
    const titled = (amount: string | null): string => [`${description}${amount === null ? "" : ` · ${amount}`}`, ...findings].join("\n");

    const trade = tradeOf(txn);
    if (trade !== null) {
        const side = trade.amount.qty.m > 0n ? "buy" : "sell";
        const sign = side === "buy" ? "+" : "";
        const shares = sign + formatDec(trade.amount.qty, trade.amount.style, MAX_QUANTITY_DECIMALS);
        const cash = side === "buy" ? neg(trade.cash) : trade.cash;
        const amount = rounded(trade.cashCommodity, cash, opts.styleOf(trade.cashCommodity));
        return {
            ...base,
            label: `${trade.amount.commodity} ${shares}`,
            trade: {side, symbol: trade.amount.commodity, shares},
            amount,
            tone: toneOf(cash),
            title: titled(amount),
        };
    }

    const flow = shownAmount(txn, opts.preferred);
    if (flow === null) return {...base, label: description, trade: null, amount: null, tone: "neutral", title: titled(null)};
    const pnl = earned(txn, flow.commodity, opts.classes);
    const qty = pnl === null || pnl.m === 0n ? flow.qty : pnl;
    const amount = rounded(flow.commodity, qty, flow.style);
    return {...base, label: description, trade: null, amount, tone: pnl === null ? "neutral" : toneOf(pnl), title: titled(amount)};
}

/**
 * The view's rows by day, each day in journal order (oldest first). The view
 * serves rows newest first, so each day's list is the reverse of how they
 * arrived.
 */
export function groupByDay<T extends {date: ISODate}>(items: readonly T[]): Map<ISODate, T[]> {
    const days = new Map<ISODate, T[]>();
    for (let i = items.length - 1; i >= 0; i -= 1) {
        const item = items[i];
        const list = days.get(item.date);
        if (list === undefined) days.set(item.date, [item]);
        else list.push(item);
    }
    return days;
}
