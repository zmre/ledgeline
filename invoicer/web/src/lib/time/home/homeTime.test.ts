// Home's time row figures: the two weeks from one summary, the unbilled
// sums per commodity, and when the row hides.
import {describe, expect, it, vi} from "vitest";
import {EditorApiError} from "#lib/editorApi.js";
import type {CustomerUnbilled, Unbilled} from "../api/billing.js";
import type {Summary, SummaryRow} from "../api/summary.js";
import {qty} from "../testing.js";
import {fetchHomeTime, unbilledOf, weekOf, weekRanges, type HomeTimeApi} from "./homeTime.js";

const row = (key: string[], minutes: number): SummaryRow => ({key, quantity: qty(minutes), entries: 1});

const summary = (rows: SummaryRow[]): Summary => ({
    rows,
    other: null,
    total: qty(rows.reduce((t, r) => t + r.quantity.seconds / 60, 0)),
    buckets: [],
    file: "time.journal",
});

const customer = (over: Partial<CustomerUnbilled> = {}): CustomerUnbilled => ({
    customer: "Acme Corp",
    title: "Acme Corp",
    path: "customers/Acme Corp.md",
    cadence: "monthly",
    quantity: qty(120),
    entries: 2,
    oldest: "2026-11-02",
    newest: "2026-11-26",
    estimate: "300.00",
    commodity: "$",
    periods: [],
    ...over,
});

const unbilled = (customers: CustomerUnbilled[], over: Partial<Unbilled> = {}): Unbilled => ({
    today: "2026-11-26",
    customers,
    needs_customer: [],
    orphans: [],
    suggested_total: qty(0),
    ...over,
});

// Thursday 2026-11-26: this week is Mon 23 – Sun 29; last week to Thursday is 16 – 19.
const WEEKS = summary([
    row(["2026-11-16", "yes"], 60),
    row(["2026-11-19", "no"], 60),
    row(["2026-11-20", "yes"], 600), // last Friday: after today's weekday, not counted
    row(["2026-11-23", "yes"], 90),
    row(["2026-11-26", "no"], 30),
]);

describe("UNIT weekRanges", () => {
    it("is this week Monday to Sunday and last week to the same weekday", () => {
        const {thisWeek, lastWeek} = weekRanges("2026-11-26");
        expect([thisWeek.from, thisWeek.to]).toEqual(["2026-11-23", "2026-11-29"]);
        expect([lastWeek.from, lastWeek.end]).toEqual(["2026-11-16", "2026-11-19"]);
    });
});

describe("UNIT weekOf", () => {
    it("sums the days in range, billable apart", () => {
        expect(weekOf(WEEKS, "2026-11-23", "2026-11-29")).toEqual({from: "2026-11-23", end: "2026-11-29", total: 7200, billable: 5400, share: 0.75});
        expect(weekOf(WEEKS, "2026-11-16", "2026-11-19")).toMatchObject({total: 7200, billable: 3600, share: 0.5});
    });

    it("has no share with no time", () => {
        expect(weekOf(summary([]), "2026-11-23", "2026-11-29").share).toBeNull();
    });
});

describe("UNIT unbilledOf", () => {
    it("sums hours and money per commodity, largest first, in whole cents", () => {
        const figures = unbilledOf(
            unbilled([
                customer({estimate: "0.10"}),
                customer({customer: "Globex", estimate: "0.20"}),
                customer({customer: "Initech", estimate: "900", commodity: "EUR"}),
            ])
        );
        expect(figures.money).toEqual([
            {commodity: "EUR", amount: "900.00"},
            {commodity: "$", amount: "0.30"},
        ]);
        expect(figures).toMatchObject({seconds: 3 * 7200, customers: 3, unpriced: false});
    });

    it("notes time with no rate, and billable time needing a customer", () => {
        const figures = unbilledOf(
            unbilled([customer(), customer({customer: "Initech", estimate: null, commodity: null})], {
                needs_customer: [{account: "clients:orphan", quantity: qty(45), oldest: "2026-11-03"}],
            })
        );
        expect(figures).toMatchObject({seconds: 14400, unpriced: true, needsCustomer: 2700, money: [{commodity: "$", amount: "300.00"}]});
    });
});

describe("UNIT fetchHomeTime", () => {
    const api = (over: Partial<HomeTimeApi> = {}): HomeTimeApi => ({
        summary: vi.fn((params) => Promise.resolve(params.group.length === 0 ? summary([row([], 600)]) : WEEKS)),
        unbilled: vi.fn(() => Promise.resolve(unbilled([customer()]))),
        ...over,
    });

    it("asks for all time, the two weeks by day and billable, and the unbilled list", async () => {
        const calls = api();
        const time = await fetchHomeTime("2026-11-26", calls);
        expect(calls.summary).toHaveBeenCalledWith({group: []});
        expect(calls.summary).toHaveBeenCalledWith({from: "2026-11-16", to: "2026-11-29", group: ["day", "billable"]});
        expect(time?.thisWeek.total).toBe(7200);
        expect(time?.lastWeek.total).toBe(7200);
        expect(time?.unbilled.money).toEqual([{commodity: "$", amount: "300.00"}]);
    });

    it("is null with no time at all", async () => {
        expect(await fetchHomeTime("2026-11-26", api({summary: vi.fn(() => Promise.resolve(summary([])))}))).toBeNull();
    });

    it("is null before setup, and throws any other refusal", async () => {
        const refuse = (code: string) => vi.fn(() => Promise.reject(new EditorApiError(409, code, "no")));
        expect(await fetchHomeTime("2026-11-26", api({summary: refuse("not-set-up")}))).toBeNull();
        await expect(fetchHomeTime("2026-11-26", api({unbilled: refuse("locked")}))).rejects.toThrow("no");
    });
});
