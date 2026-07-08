// WP-04 CONTRACT STUB — pre-seeded by the orchestrator so WP-03/05 can build
// against the exact exported shape from plans/04-filter-bar.md. WP-04 replaces
// this file with the full implementation (subtree-aware toggleAccount, tests).
import type {ISODate} from "$lib/domain/types";

export interface JournalFilter {
    from: ISODate | null; // inclusive
    to: ISODate | null; // inclusive
    accounts: ReadonlySet<string>; // selected account names (empty = all)
    query: string; // free text, matched against txn.haystack lowercased
}

export type DatePreset = "thisMonth" | "lastMonth" | "last90" | "ytd" | "thisYear" | "lastYear" | "all";

function pad(n: number): string {
    return String(n).padStart(2, "0");
}

function localToday(): ISODate {
    const d = new Date();
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function lastDayOfMonth(y: number, m: number): number {
    // numeric-parts Date math only (never new Date('YYYY-MM-DD'))
    return new Date(Date.UTC(y, m, 0)).getUTCDate();
}

function shiftDays(date: ISODate, days: number): ISODate {
    const y = Number(date.slice(0, 4));
    const m = Number(date.slice(5, 7));
    const d = Number(date.slice(8, 10));
    const t = new Date(Date.UTC(y, m - 1, d + days));
    return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
}

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

function defaultFilter(): JournalFilter {
    const {from, to} = presetRange("thisMonth", localToday());
    return {from, to, accounts: new Set<string>(), query: ""};
}

let value = $state<JournalFilter>(defaultFilter());

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
    toggleAccount(name: string): void {
        const accounts = new Set(value.accounts);
        if (accounts.has(name)) {
            accounts.delete(name);
        } else {
            accounts.add(name);
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
};
