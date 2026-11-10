// INTEGRATION: the running-balance wire (plan 36) against a live engine
// (`just test-integration`, serving fixtures/sample.journal; every live suite
// pins today to 2026-07-08). Skipped unless LEDGELINE_API_URL is set.
//
// Pins the CONTRACT both routes make to the SPA, decoded through the SPA's own
// decoders, using facts of the fixture rather than captured figures:
//
//   - `assets:bank:savings` carries `= $13,500.00` on 2026-04-05, and
//     `assets:bank:checking` carries `= $5,000.00` on 2024-07-01 (the opening
//     balances), so both leaves have an assertion the column must agree with.
//   - The fixture has no `date:` posting tags, so in oldest-first order every
//     balance is the previous one plus the row's change.
//
// The balance line (amendment A1, `/api/journal/running/series`) is held to
// the same facts: savings' last transaction is that 2026-04-05 transfer, so
// its line reaches $13,500.00 in April 2026 and stays there to today; and at
// every period end the line equals the column of the newest row on or before
// it.
//
// The arithmetic itself (and parity with `hledger reg -H`) is pinned in Rust
// by the engine's goldens; this suite is about the shape and the invariants a
// rendered column relies on.

import {describe, expect, it} from "vitest";
import {decodeBalanceSeries, decodeRowsPage, decodeRunningInfo} from "#lib/api/journalViewDecode.js";
import {EngineRefusalError, LedgelineApi, type JournalQueryFilter, type RunningPinQuery} from "#lib/api/native.js";
import {add, cmp, dec, type Dec} from "#lib/domain/money.js";
import type {JournalRow, RunningCell} from "#lib/journalView/types.js";

const apiUrl = process.env.LEDGELINE_API_URL;
const TODAY = "2026-07-08";
const ALL: JournalQueryFilter = {from: null, to: null, accounts: new Set<string>(), query: ""};
const SAVINGS = "assets:bank:savings";
const CHECKING = "assets:bank:checking";

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live running balances", () => {
    const api = (): LedgelineApi => new LedgelineApi(apiUrl ?? "");

    /** Every row of `filter`, oldest first, with the pin's cells. */
    async function rows(filter: JournalQueryFilter, pin: RunningPinQuery | null): Promise<JournalRow[]> {
        const out: JournalRow[] = [];
        for (let offset = 0; ; offset += 1000) {
            const page = decodeRowsPage(await api().journalRows(filter, {offset, limit: 1000, order: "asc", today: TODAY}, pin));
            out.push(...page.rows);
            if (page.rows.length < 1000 || out.length >= page.total) return out;
        }
    }

    const cell = (row: JournalRow): RunningCell => {
        if (row.running == null) throw new Error(`row ${row.txn.index} has no running cell`);
        return row.running;
    };
    const sameQty = (a: Dec, b: Dec): boolean => cmp(a, b) === 0;

    it("/api/journal/running names the default commodity first and lists assertions in evaluation order", async () => {
        const info = decodeRunningInfo(await api().journalRunning(SAVINGS));
        expect(info.account).toBe(SAVINGS);
        expect(info.commodities[0]?.commodity).toBe("$");
        const counts = info.commodities.map((c) => c.postings);
        expect([...counts].sort((a, b) => b - a)).toEqual(counts);
        const points = info.assertions.$ ?? [];
        expect(points.some((point) => point.date === "2026-04-05")).toBe(true);
        expect(points.map((point) => point.date)).toEqual([...points.map((point) => point.date)].sort());
        expect(typeof info.generation).toBe("string");
    });

    it("rows carry no running cell unless a balance is asked for", async () => {
        const page = decodeRowsPage(await api().journalRows(ALL, {offset: 0, limit: 50, today: TODAY}));
        expect(page.rows.every((row) => row.running === null)).toBe(true);
    });

    it("every row gets a cell, in the default commodity when none (or a stale one) is named", async () => {
        for (const pin of [
            {account: SAVINGS, commodity: null},
            {account: SAVINGS, commodity: "NO-SUCH-COMMODITY"},
        ]) {
            const all = await rows(ALL, pin);
            expect(all.length).toBeGreaterThan(0);
            expect(all.every((row) => cell(row).balance.commodity === "$")).toBe(true);
        }
    });

    it("the column agrees with a passing assertion on the pinned leaf", async () => {
        for (const [account, date, asserted] of [
            [SAVINGS, "2026-04-05", dec(1350000, 2)],
            [CHECKING, "2024-07-01", dec(500000, 2)],
        ] as const) {
            const all = await rows(ALL, {account, commodity: null});
            const marked = all.filter((row) => row.txn.date === date && cell(row).assertions.length > 0);
            expect(marked).toHaveLength(1);
            const [mark] = cell(marked[0]).assertions;
            expect(mark.account).toBe(account);
            expect(mark.ok).toBe(true);
            expect(mark.calculated).toBeNull();
            expect(sameQty(mark.asserted.qty, asserted)).toBe(true);
            expect(sameQty(cell(marked[0]).balance.qty, asserted)).toBe(true);
        }
    });

    it("reads as a running total oldest-first: each balance is the previous plus the change", async () => {
        const all = await rows(ALL, {account: CHECKING, commodity: null});
        let previous: Dec = dec(0, 0);
        for (const row of all) {
            const c = cell(row);
            expect(c.postedOn).toBeNull(); // the fixture has no `date:` tags
            const expected = c.change === null ? previous : add(previous, c.change.qty);
            expect(sameQty(c.balance.qty, expected), `row ${row.txn.index} ${row.txn.date}`).toBe(true);
            // `touches` is about the subtree in ANY commodity, so it can be true with a null change, never the reverse.
            if (c.change !== null) expect(c.touches).toBe(true);
            previous = c.balance.qty;
        }
    });

    it("no filter changes the numbers: a filtered row has the same cell as unfiltered", async () => {
        const pin = {account: CHECKING, commodity: null};
        const byIndex = new Map((await rows(ALL, pin)).map((row) => [row.txn.index, cell(row)]));
        for (const filter of [
            {...ALL, from: "2026-01-01", to: "2026-03-31"},
            {...ALL, accounts: new Set(["income:salary"])},
            {...ALL, query: "rent"},
        ]) {
            const shown = await rows(filter, pin);
            expect(shown.length).toBeGreaterThan(0);
            for (const row of shown) expect(cell(row)).toEqual(byIndex.get(row.txn.index));
        }
    });

    it("rows outside the pinned subtree carry the balance, with no change", async () => {
        const shown = await rows({...ALL, accounts: new Set(["expenses:food:groceries"])}, {account: SAVINGS, commodity: null});
        expect(shown.length).toBeGreaterThan(0);
        for (const row of shown) {
            if (row.txn.postings.some((p) => p.account === SAVINGS || p.account.startsWith(`${SAVINGS}:`))) continue;
            expect(cell(row).change).toBeNull();
            expect(cell(row).touches).toBe(false);
        }
    });

    it("a subtree sums its children, one commodity at a time", async () => {
        const commodities = decodeRunningInfo(await api().journalRunning("assets:bank")).commodities.map((c) => c.commodity);
        expect(commodities).toContain("$");
        expect(commodities).toContain("EUR"); // assets:bank:wise:eur
        const last = async (account: string): Promise<Dec> => cell((await rows(ALL, {account, commodity: "$"})).at(-1)!).balance.qty;
        expect(sameQty(await last("assets:bank"), add(await last(CHECKING), await last(SAVINGS)))).toBe(true);
    });

    it("refuses an account that matches nothing on both routes", async () => {
        await expect(api().journalRunning("assets:no:such:account")).rejects.toBeInstanceOf(EngineRefusalError);
        await expect(api().journalRows(ALL, {offset: 0, limit: 10, today: TODAY}, {account: "assets:no:such:account", commodity: null})).rejects.toBeInstanceOf(
            EngineRefusalError
        );
    });

    describe("/api/journal/running/series", () => {
        const series = async (query: {account: string; commodity?: string | null; from?: string | null; to?: string | null}) =>
            decodeBalanceSeries(await api().journalRunningSeries({commodity: null, from: null, to: null, today: TODAY, ...query}));

        it("runs all time from the first posting to today, reaching the asserted figure and holding it", async () => {
            const line = await series({account: SAVINGS});
            expect(line.account).toBe(SAVINGS);
            expect(line.commodity).toBe("$");
            expect(line.to).toBe(TODAY);
            // Two years of history: months, the holdings chart's choice.
            expect(line.interval).toBe("monthly");
            expect(line.points.at(-1)?.end).toBe(TODAY);
            const april = line.points.find((point) => point.bucket === "2026-04");
            expect(april).toBeDefined();
            expect(sameQty(april!.balance, dec(1350000, 2))).toBe(true);
            // Nothing posts after it, so every later period carries it and has nothing to scroll to.
            for (const point of line.points.filter((p) => p.end > "2026-04-30")) {
                expect(sameQty(point.balance, dec(1350000, 2))).toBe(true);
                expect(point.tindex).toBeNull();
            }
            expect(line.points.every((point) => point.failures.length === 0)).toBe(true);
        });

        it("agrees with the column: each point is the balance of the newest row on or before its end", async () => {
            const all = await rows(ALL, {account: CHECKING, commodity: null});
            const line = await series({account: CHECKING, from: "2025-01-01", to: "2026-06-30"});
            expect({from: line.from, to: line.to}).toEqual({from: "2025-01-01", to: "2026-06-30"});
            expect(line.points.length).toBeGreaterThan(6);
            for (const point of line.points) {
                const newest = all.filter((row) => row.txn.date <= point.end).at(-1);
                const column = newest === undefined ? dec(0, 0) : cell(newest).balance.qty;
                expect(sameQty(point.balance, column), point.end).toBe(true);
            }
        });

        it("follows a narrow range at a finer grain, counting everything before it", async () => {
            const line = await series({account: SAVINGS, from: "2026-03-01", to: "2026-04-30"});
            expect(line.interval).toBe("weekly");
            // Nothing before the range is dropped: the first week already holds years of transfers.
            expect(cmp(line.points[0].balance, dec(1300000, 2))).toBe(0);
            expect(sameQty(line.points.at(-1)!.balance, dec(1350000, 2))).toBe(true);
        });

        it("resolves the commodity as the column does", async () => {
            expect((await series({account: SAVINGS, commodity: "NO-SUCH-COMMODITY"})).commodity).toBe("$");
            const eur = await series({account: "assets:bank", commodity: "EUR"});
            expect(eur.commodity).toBe("EUR");
        });

        it("refuses an account that matches nothing", async () => {
            await expect(series({account: "assets:no:such:account"})).rejects.toBeInstanceOf(EngineRefusalError);
        });
    });
});
