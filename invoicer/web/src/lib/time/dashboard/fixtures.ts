// Test data for the time dashboard: what `GET /api/time/summary` answers
// for invoicer/fixtures/time, November 2026 and October 1–26 (the previous
// period to date), as the fact table (FACT_GROUPS) and by note. Used by the
// unit and component tests; never by the app.
import type {Qty, Summary, SummaryRow} from "../api/summary.js";
import type {DashMeta} from "./meta.js";

export function qty(minutes: number): Qty {
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return {seconds: minutes * 60, hours: String(minutes / 60), text: h === 0 ? `${m}m` : m === 0 ? `${h}h` : `${h}h ${String(m).padStart(2, "0")}m`};
}

type FactSpec = [account: string, customer: string, person: string, billable: "yes" | "no", billed: string, date: string, minutes: number, entries?: number];

const row = ([account, customer, person, billable, billed, date, minutes, entries = 1]: FactSpec): SummaryRow => ({
    key: [account, customer, person, billable, billed, date],
    quantity: qty(minutes),
    entries,
});

export function summary(rows: SummaryRow[]): Summary {
    const total = rows.reduce((t, r) => t + r.quantity.seconds / 60, 0);
    return {rows, other: null, total: qty(total), buckets: [], file: "time.journal"};
}

/** November 2026 as the fact table. */
export const NOVEMBER: Summary = summary(
    (
        [
            ["clients:acme:advisory", "Acme Corp", "pat", "yes", "unbilled", "2026-11-02", 2],
            ["clients:acme:advisory", "Acme Corp", "pat", "yes", "unbilled", "2026-11-03", 2],
            ["clients:acme:advisory", "Acme Corp", "pat", "yes", "unbilled", "2026-11-26", 60],
            ["clients:acme:advisory", "Acme Corp", "sam", "yes", "unbilled", "2026-11-04", 150],
            ["clients:acme:tax-return", "Acme Corp", "pat", "yes", "unbilled", "2026-11-02", 90],
            ["clients:acme:tax-return", "Acme Corp", "pat", "yes", "unbilled", "2026-11-25", 130],
            ["clients:acme:travel", "Acme Corp", "pat", "no", "not-billable", "2026-11-03", 60],
            ["clients:globex:audit", "Globex", "pat", "yes", "unbilled", "2026-11-03", 120],
            ["clients:globex:audit", "Globex", "riley", "yes", "unbilled", "2026-11-10", 180],
            ["clients:globex:audit", "Globex", "sam", "yes", "unbilled", "2026-11-04", 240],
            ["clients:globex:audit", "Globex", "sam", "yes", "unbilled", "2026-11-05", 270],
            ["internal:admin", "", "pat", "no", "not-billable", "2026-11-02", 15],
            ["internal:admin", "", "pat", "no", "not-billable", "2026-11-26", 15],
            ["internal:admin", "", "sam", "no", "not-billable", "2026-11-05", 30],
            ["internal:old", "", "pat", "no", "not-billable", "2026-11-25", 10],
        ] as FactSpec[]
    ).map(row)
);

/** October 1–26, 2026. */
export const OCTOBER: Summary = summary(
    (
        [
            ["clients:acme:advisory", "Acme Corp", "pat", "yes", "unbilled", "2026-10-06", 50],
            ["clients:acme:tax-return", "Acme Corp", "pat", "yes", "invoiced", "2026-10-06", 180],
            ["clients:acme:tax-return", "Acme Corp", "pat", "yes", "invoiced", "2026-10-20", 120],
            ["clients:globex:audit", "Globex", "pat", "yes", "unbilled", "2026-10-20", 90],
            ["clients:initech:discovery", "Initech Inc", "pat", "yes", "unbilled", "2026-10-20", 40],
            ["clients:orphan", "", "pat", "yes", "not-billable", "2026-10-20", 15],
            ["internal:admin", "", "pat", "no", "not-billable", "2026-10-06", 20],
            ["internal:training", "", "pat", "no", "not-billable", "2026-10-06", 25],
        ] as FactSpec[]
    ).map(row)
);

const note = (text: string, minutes: number, entries: number): SummaryRow => ({key: [text], quantity: qty(minutes), entries});

export const NOVEMBER_NOTES: Summary = summary([
    note("", 1145, 9),
    note("email", 4, 2),
    note("fieldwork", 510, 2),
    note("call with Fred about the K-1", 60, 1),
    note("document review", 180, 1),
    note("drive to site", 60, 1),
    note("gathered documents", 90, 1),
    note("reviewed schedules C and E", 130, 1),
    note("second opinion on the K-1", 150, 1),
]);

export const OCTOBER_NOTES: Summary = summary([note("", 220, 4), note("first pass", 180, 1), note("year-end planning", 50, 1), note("walkthrough", 90, 1)]);

export const META: DashMeta = {
    configured: true,
    me: "pat",
    weeklyTarget: 30,
    customers: ["Acme Corp", "Globex", "Initech Inc"],
    people: ["pat", "sam", "riley"],
    accounts: [
        "clients",
        "clients:acme",
        "clients:acme:advisory",
        "clients:acme:tax-return",
        "clients:acme:travel",
        "clients:globex",
        "clients:globex:audit",
        "clients:initech",
        "clients:initech:discovery",
        "clients:orphan",
        "internal",
        "internal:admin",
        "internal:old",
        "internal:training",
    ],
    maxDepth: 3,
    billableCustomers: true,
    anyTime: true,
    branchOrder: ["clients", "internal"],
    customerOrder: ["Globex", "Acme Corp", "Initech Inc"],
};
