// Time tracking's test rows, shaped as the server answers them for the
// time fixture (invoicer/fixtures/time). Each takes overrides.
import type {AccountRow, AccountsView, Inherited} from "./api/accounts.js";
import type {EntryList, EntryView, Qty} from "./api/entries.js";
import {formatDuration} from "./entries/duration.js";

/** A Qty of `minutes`. */
export const qty = (minutes: number): Qty => ({seconds: minutes * 60, hours: String(minutes / 60), text: formatDuration(minutes * 60)});

/** Pat's 1h on Acme advisory, 2026-11-26, unbilled. */
export function entry(over: Partial<EntryView> = {}): EntryView {
    return {
        id: "time/pat/2026-11.timedot@20",
        date: "2026-11-26",
        person: "pat",
        account: "clients:acme:advisory",
        quantity: qty(60),
        note: "call with Fred about the K-1",
        tags: [["person", "pat"]],
        invoice: null,
        bill: null,
        nobill: null,
        billed: "unbilled",
        billable: true,
        customer: "Acme Corp",
        file: "time/pat/2026-11.timedot",
        line: 20,
        format: "timedot",
        locked: null,
        ...over,
    };
}

/** Pat's 3h on invoice 101 (sent): locked. */
export const lockedEntry = (over: Partial<EntryView> = {}): EntryView =>
    entry({
        id: "time/pat/2026-10.timedot@5",
        date: "2026-10-06",
        account: "clients:acme:tax-return",
        quantity: qty(180),
        note: "first pass",
        invoice: "101",
        billed: "invoiced",
        file: "time/pat/2026-10.timedot",
        line: 5,
        locked: "it is on invoice 101, which is sent; void it or release its time to change it",
        ...over,
    });

/** A list of `entries`, totalled. */
export function list(entries: EntryView[], over: Partial<EntryList> = {}): EntryList {
    const seconds = entries.reduce((t, e) => t + e.quantity.seconds, 0);
    return {entries, total: qty(seconds / 60), findings: [], ...over};
}

const own = (value: string, from: string): Inherited => ({value, from, inherited: false});
const above = (value: string, from: string): Inherited => ({value, from, inherited: true});

/** An account row; `clients:acme` with its own customer by default. */
export function accountRow(over: Partial<AccountRow> = {}): AccountRow {
    return {
        account: "clients:acme",
        depth: 1,
        declared: true,
        used: true,
        tags: [],
        customer: own("Acme Corp", "clients:acme"),
        customer_found: true,
        billable: above("yes", "clients"),
        item: own("ARCH", "clients:acme"),
        archived: null,
        month: qty(0),
        unbilled: qty(0),
        total: qty(0),
        warnings: [],
        ...over,
    };
}

/** The fixture's tree, in part: clients, acme and two children, internal and an archived child. */
export function accountsView(over: Partial<AccountsView> = {}): AccountsView {
    return {
        accounts: [
            accountRow({account: "clients", depth: 0, customer: null, billable: own("yes", "clients"), item: null}),
            accountRow({month: qty(244), unbilled: qty(244)}),
            accountRow({
                account: "clients:acme:advisory",
                depth: 2,
                customer: above("Acme Corp", "clients:acme"),
                item: above("ARCH", "clients:acme"),
                month: qty(64),
            }),
            accountRow({
                account: "clients:acme:tax-return",
                depth: 2,
                customer: above("Acme Corp", "clients:acme"),
                item: own("TAX", "clients:acme:tax-return"),
                month: qty(220),
            }),
            accountRow({account: "internal", depth: 0, customer: null, billable: own("no", "internal"), item: null}),
            accountRow({
                account: "internal:old",
                depth: 1,
                customer: null,
                billable: above("no", "internal"),
                item: null,
                archived: own("yes", "internal:old"),
            }),
        ],
        undeclared: ["internal:training"],
        file: "time-accounts.journal",
        file_exists: true,
        ...over,
    };
}
