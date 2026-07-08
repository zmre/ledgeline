// Journal filter store (WP-04): date range, account multi-select, free-text
// query. Svelte 5 runes state; THE contract WP-03/05 consume (plans/04).
//
// Account selection semantics: a selected name implies its whole subtree via
// `accountMatches` — the set stores ONLY subtree roots, never redundant
// children. See `toggleAccount` for the exact toggle rules.
/* eslint-disable svelte/prefer-svelte-reactivity -- the account Sets are
   immutable snapshots: every change replaces `value` wholesale, so plain Set
   is correct (and the contract exposes ReadonlySet); Date is read-once. */
import {accountMatches} from "$lib/domain/accounts";
import type {ISODate} from "$lib/domain/types";

export interface JournalFilter {
    from: ISODate | null; // inclusive
    to: ISODate | null; // inclusive
    accounts: ReadonlySet<string>; // selected account subtree roots (empty = all)
    query: string; // free text, matched against txn.haystack lowercased
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

/** Shift an ISO date by whole days using numeric-parts Date math (no string parsing of dates). */
function shiftDays(date: ISODate, days: number): ISODate {
    const y = Number(date.slice(0, 4));
    const m = Number(date.slice(5, 7));
    const d = Number(date.slice(8, 10));
    const t = new Date(Date.UTC(y, m - 1, d + days));
    return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
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
            return {from: shiftDays(today, -89), to: today};
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

/** The first-load default: current month, no accounts, empty query. */
export function defaultFilter(): JournalFilter {
    const {from, to} = presetRange("thisMonth", localToday());
    return {from, to, accounts: new Set<string>(), query: ""};
}

let value = $state<JournalFilter>(defaultFilter());

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
        value = {...value, from, to};
    },
    applyPreset(p: DatePreset): void {
        value = {...value, ...presetRange(p, localToday())};
    },
    /**
     * Toggle an account's selection, keeping the subtree-root invariant:
     * - name is selected → deselect it;
     * - name is covered by a selected ancestor → deselect that ancestor
     *   (drops the covering subtree; we cannot know its siblings here);
     * - otherwise → select it, pruning any now-redundant selected descendants.
     */
    toggleAccount(name: string): void {
        const accounts = new Set(value.accounts);
        if (accounts.has(name)) {
            accounts.delete(name);
        } else {
            const coveringAncestors = [...accounts].filter((sel) => accountMatches(sel, name));
            if (coveringAncestors.length > 0) {
                for (const sel of coveringAncestors) accounts.delete(sel);
            } else {
                for (const sel of [...accounts]) {
                    if (accountMatches(name, sel)) accounts.delete(sel);
                }
                accounts.add(name);
            }
        }
        value = {...value, accounts};
    },
    clearAccounts(): void {
        value = {...value, accounts: new Set<string>()};
    },
    setQuery(q: string): void {
        value = {...value, query: q};
    },
    reset(): void {
        value = defaultFilter();
    },
    /** Replace the whole filter at once (urlSync startup restore). */
    replace(f: JournalFilter): void {
        value = {from: f.from, to: f.to, accounts: new Set(f.accounts), query: f.query};
    },
};
