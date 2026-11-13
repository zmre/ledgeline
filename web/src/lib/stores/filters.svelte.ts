// Journal filter store (WP-04): date range, account multi-select, free-text
// query. Svelte 5 runes state; THE contract WP-03/05 consume (plans/04).
//
// Account selection semantics: a selected name implies its whole subtree via
// `accountMatches` — the set stores ONLY subtree roots, never redundant
// children. See `toggleAccount` for the exact toggle rules.
/* eslint-disable svelte/prefer-svelte-reactivity -- the account Sets are
   immutable snapshots: every change replaces `value` wholesale, so plain Set
   is correct (and the contract exposes ReadonlySet); Date is read-once. */
import type {ISODate} from "#lib/domain/types.js";
import {toggleSubtreeRoot} from "#lib/filters/treeSelect.js";
import {calendarMonth, isWholeMonth, monthRange} from "#lib/journal/calendarModel.js";
import {addDays} from "#lib/reports/periods.js";

export interface JournalFilter {
    from: ISODate | null; // inclusive
    to: ISODate | null; // inclusive
    accounts: ReadonlySet<string>; // selected account subtree roots (empty = all)
    query: string; // free text, matched against txn.haystack lowercased
    /**
     * Which preset produced from/to, or null for a hand-picked range. Presets
     * stay LIVE: the URL stores the preset name instead of frozen dates, so a
     * restored "ytd" recomputes against the current day rather than pinning
     * the range to whenever it was clicked.
     */
    preset?: DatePreset | null;
    /**
     * The account whose running balance the table shows (plan 36), or null.
     * Separate from `accounts` on purpose: pinning sets the account filter too,
     * but after that the two move independently — changing the filter keeps the
     * pin, and only `reset()` (or the pin's own ✕) clears it. Optional so every
     * filter literal that predates it stays a valid filter.
     */
    pin?: RunningPin | null;
    /**
     * The journal shows a month calendar instead of the table (plan 40). While
     * it is on, `from`/`to` are always exactly one calendar month — every
     * change to the range is snapped to one (`holdMonth`) — and there is never
     * a pin: entering the calendar clears the running balance, and pinning
     * leaves the calendar. Not part of the view's key: switching view
     * refetches nothing the range did not already change. Optional, like
     * `pin`, so older filter literals stay valid (absent: the table).
     */
    calendar?: boolean;
}

/** A pinned running balance: the subtree root, and the commodity (null: the engine's default, the one with most postings). */
export interface RunningPin {
    account: string;
    commodity: string | null;
}

export type DatePreset = "thisMonth" | "lastMonth" | "last90" | "ytd" | "thisYear" | "lastYear" | "all";

function pad(n: number): string {
    return String(n).padStart(2, "0");
}

/** Today as an ISO date from LOCAL Date parts (never `new Date('YYYY-MM-DD')` — that parses UTC). */
export function localToday(): ISODate {
    const d = new Date();
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Days in month `m` (1-12) of year `y`, via numeric-parts Date math only. */
function lastDayOfMonth(y: number, m: number): number {
    return new Date(Date.UTC(y, m, 0)).getUTCDate();
}

/** Pure preset → inclusive range math with an injected `today` (unit-tested; `last90` includes today, i.e. today-89 … today). */
export function presetRange(p: DatePreset, today: ISODate): {from: ISODate | null; to: ISODate | null} {
    const y = Number(today.slice(0, 4));
    const m = Number(today.slice(5, 7));
    switch (p) {
        case "thisMonth":
            return {from: `${y}-${pad(m)}-01`, to: `${y}-${pad(m)}-${pad(lastDayOfMonth(y, m))}`};
        case "lastMonth": {
            const ly = m === 1 ? y - 1 : y;
            const lm = m === 1 ? 12 : m - 1;
            return {from: `${ly}-${pad(lm)}-01`, to: `${ly}-${pad(lm)}-${pad(lastDayOfMonth(ly, lm))}`};
        }
        case "last90":
            return {from: addDays(today, -89), to: today};
        case "ytd":
            return {from: `${y}-01-01`, to: today};
        case "thisYear":
            return {from: `${y}-01-01`, to: `${y}-12-31`};
        case "lastYear":
            return {from: `${y - 1}-01-01`, to: `${y - 1}-12-31`};
        case "all":
            return {from: null, to: null};
    }
}

/** The first-load default: the last 90 days, no accounts, empty query. */
export function defaultFilter(): JournalFilter {
    const {from, to} = presetRange("last90", localToday());
    return {from, to, accounts: new Set<string>(), query: "", preset: "last90", pin: null};
}

/**
 * The calendar's invariants applied to `f`: with the calendar on, the range
 * becomes the one month it means (`calendarMonth`) and the pin goes. A range
 * that already IS a whole month is left alone, preset and all, so a live
 * "this month" stays live. With the calendar off, `f` is returned untouched.
 */
export function holdMonth(f: JournalFilter, today: ISODate): JournalFilter {
    if (f.calendar !== true) return f;
    const pinless = (f.pin ?? null) === null ? f : {...f, pin: null};
    if (isWholeMonth(f.from, f.to)) return pinless;
    return {...pinless, ...monthRange(calendarMonth(f.from, f.to, today)), preset: null};
}

let value = $state<JournalFilter>(defaultFilter());

/** Every write goes through here, so no path can leave the calendar on a range that is not a month. */
function commit(next: JournalFilter): void {
    value = holdMonth(next, localToday());
}

/**
 * Observe filter changes outside component context (used by urlSync, which is
 * plain TS and cannot declare rune effects itself). The callback fires once
 * immediately, then after every filter change. Returns an unsubscribe.
 */
export function subscribeFilters(cb: (f: JournalFilter) => void): () => void {
    return $effect.root(() => {
        $effect(() => {
            cb(value);
        });
    });
}

export const filters = {
    get value(): JournalFilter {
        return value;
    },
    setRange(from: ISODate | null, to: ISODate | null): void {
        commit({...value, from, to, preset: null});
    },
    applyPreset(p: DatePreset): void {
        commit({...value, ...presetRange(p, localToday()), preset: p});
    },
    /** Toggle an account's selection, keeping the subtree-root invariant (see treeSelect.toggleSubtreeRoot). */
    toggleAccount(name: string): void {
        commit({...value, accounts: toggleSubtreeRoot(value.accounts, name)});
    },
    /**
     * Replace the whole account selection at once.
     *
     * For a caller that has applied several toggles to a local copy and wants to
     * commit them as ONE filter change — the filter bar does this, so a burst of
     * checkbox clicks costs one re-derivation of the journal view instead of one
     * each. The set is copied, so the caller's working copy stays theirs.
     * Callers are responsible for the subtree-root invariant; build the set with
     * `toggleSubtreeRoot`, exactly as `toggleAccount` does.
     */
    setAccounts(accounts: ReadonlySet<string>): void {
        commit({...value, accounts: new Set(accounts)});
    },
    clearAccounts(): void {
        commit({...value, accounts: new Set<string>()});
    },
    setQuery(q: string): void {
        commit({...value, query: q});
    },
    /** Pin `account`'s running balance, leaving the account filter as it is. `commodity` null: the engine's default. */
    pin(account: string, commodity: string | null = null): void {
        commit({...value, pin: {account, commodity}, calendar: false});
    },
    /**
     * Filter to `account` AND pin it, as one filter change — the Balances view's
     * click. The date range is kept: the balance is right whatever the range.
     */
    focusAccount(account: string, commodity: string | null = null): void {
        commit({...value, accounts: new Set([account]), pin: {account, commodity}, calendar: false});
    },
    /** Switch the pinned commodity (null: back to the engine's default). No-op without a pin. */
    setPinCommodity(commodity: string | null): void {
        const pin = value.pin ?? null;
        if (pin === null || pin.commodity === commodity) return;
        commit({...value, pin: {account: pin.account, commodity}});
    },
    /** Remove the running-balance columns. The account filter stays as it is. */
    unpin(): void {
        if ((value.pin ?? null) === null) return;
        commit({...value, pin: null});
    },
    /**
     * Back to the default filter. In the calendar, the view and the month it
     * shows stay: Reset clears what narrows the month, it does not throw the
     * user out of the view or back to a 90-day range the calendar cannot show.
     */
    reset(): void {
        const fresh = defaultFilter();
        value = value.calendar === true ? {...fresh, from: value.from, to: value.to, preset: value.preset ?? null, calendar: true} : fresh;
    },
    /** Replace the whole filter at once (urlSync startup restore). */
    replace(f: JournalFilter): void {
        const pin = f.pin ?? null;
        commit({
            from: f.from,
            to: f.to,
            accounts: new Set(f.accounts),
            query: f.query,
            preset: f.preset ?? null,
            pin: pin === null ? null : {account: pin.account, commodity: pin.commodity},
            ...(f.calendar === true ? {calendar: true} : {}),
        });
    },
    /**
     * Show the calendar (plan 40) or the table. Turning it on clears the
     * running balance and holds the range to the month it means; turning it
     * off keeps the month the calendar was showing.
     */
    setCalendar(on: boolean): void {
        if ((value.calendar === true) === on) return;
        commit({...value, calendar: on});
    },
    /** The calendar's month navigation: show `month` (`YYYY-MM`). */
    showMonth(month: string): void {
        commit({...value, ...monthRange(month), preset: null});
    },
};
