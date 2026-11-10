// Running balances (plan 36): which commodity the columns are in, where "first
// failure" goes, and the window "narrow to gap" sets. Pure, so every edge of
// the date arithmetic is pinned here rather than through a mounted table.

import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle} from "#lib/domain/types.js";
import type {RunningAssertion, RunningAssertionPoint, RunningInfo} from "#lib/journalView/types.js";
import {assertionDetail, assertionLabel, firstFailure, gapRange, pinnedCommodity, widenRange} from "./runningBalance";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: STYLE});

const point = (tindex: number, date: string, ok: boolean): RunningAssertionPoint => ({tindex, date, ok});

function info(over: Partial<RunningInfo> = {}): RunningInfo {
    return {
        generation: "g1",
        account: "assets:checking",
        commodities: [
            {commodity: "$", postings: 40},
            {commodity: "EUR", postings: 3},
        ],
        assertions: {},
        ...over,
    };
}

describe("UNIT pinnedCommodity", () => {
    it("is null with no pin", () => {
        expect(pinnedCommodity(info(), null)).toBeNull();
    });

    it("uses the engine's default (most postings) when the pin chose none", () => {
        expect(pinnedCommodity(info(), {account: "assets:checking", commodity: null})).toBe("$");
    });

    it("keeps the pin's choice when the subtree holds it", () => {
        expect(pinnedCommodity(info(), {account: "assets:checking", commodity: "EUR"})).toBe("EUR");
    });

    it("falls back to the default for a stale choice, as the engine does", () => {
        expect(pinnedCommodity(info(), {account: "assets:checking", commodity: "GBP"})).toBe("$");
    });

    it("stands by the pin's choice until the engine has answered", () => {
        expect(pinnedCommodity(null, {account: "assets:checking", commodity: "EUR"})).toBe("EUR");
        expect(pinnedCommodity(null, {account: "assets:checking", commodity: null})).toBeNull();
    });
});

describe("UNIT firstFailure", () => {
    const points = {$: [point(3, "2026-01-31", true), point(9, "2026-02-28", false), point(12, "2026-03-31", false)]};

    it("is the earliest failure in evaluation order", () => {
        expect(firstFailure(info({assertions: points}), "$")).toEqual(point(9, "2026-02-28", false));
    });

    it("is null when every assertion holds, or the commodity has none", () => {
        expect(firstFailure(info({assertions: {$: [point(3, "2026-01-31", true)]}}), "$")).toBeNull();
        expect(firstFailure(info({assertions: points}), "EUR")).toBeNull();
        expect(firstFailure(null, "$")).toBeNull();
    });
});

describe("UNIT gapRange", () => {
    const points = [point(3, "2026-01-31", true), point(5, "2026-02-14", false), point(9, "2026-02-28", false), point(12, "2026-03-31", true)];

    it("runs from the day after the previous PASSING assertion to the failure", () => {
        // #9's predecessor #5 failed too, so the window reaches back past it.
        expect(gapRange(points, 9)).toEqual({from: "2026-02-01", to: "2026-02-28"});
        expect(gapRange(points, 5)).toEqual({from: "2026-02-01", to: "2026-02-14"});
    });

    it("is open at the start when nothing before it passed", () => {
        expect(gapRange([point(5, "2026-02-14", false)], 5)).toEqual({from: null, to: "2026-02-14"});
    });

    it("crosses a month end and a leap day", () => {
        expect(gapRange([point(1, "2024-02-29", true), point(2, "2024-03-10", false)], 2)).toEqual({from: "2024-03-01", to: "2024-03-10"});
        expect(gapRange([point(1, "2025-12-31", true), point(2, "2026-01-05", false)], 2)).toEqual({from: "2026-01-01", to: "2026-01-05"});
    });

    it("never inverts: a pass earlier the same day leaves that one day", () => {
        expect(gapRange([point(1, "2026-02-14", true), point(2, "2026-02-14", false)], 2)).toEqual({from: "2026-02-14", to: "2026-02-14"});
    });

    it("keeps a posting-dated row inside the window", () => {
        // The row is dated 03-28; its leg posts (and is asserted) on 04-02.
        expect(gapRange([point(1, "2026-03-31", true), point(2, "2026-04-02", false)], 2, "2026-03-28")).toEqual({from: "2026-03-28", to: "2026-04-02"});
        // A row dated AFTER its asserted leg stretches the end instead.
        expect(gapRange([point(2, "2026-04-02", false)], 2, "2026-04-05")).toEqual({from: null, to: "2026-04-05"});
    });

    it("is null for a failure that is not in the list", () => {
        expect(gapRange(points, 12)).toBeNull(); // #12 passed
        expect(gapRange(points, 99)).toBeNull();
    });
});

describe("UNIT widenRange", () => {
    it("moves only the end that excludes the date", () => {
        expect(widenRange("2026-03-01", "2026-03-31", "2026-02-10")).toEqual({from: "2026-02-10", to: "2026-03-31"});
        expect(widenRange("2026-03-01", "2026-03-31", "2026-04-10")).toEqual({from: "2026-03-01", to: "2026-04-10"});
        expect(widenRange("2026-03-01", "2026-03-31", "2026-03-15")).toEqual({from: "2026-03-01", to: "2026-03-31"});
    });

    it("leaves an open end open", () => {
        expect(widenRange(null, null, "2020-01-01")).toEqual({from: null, to: null});
    });
});

describe("UNIT assertion marks", () => {
    const ok: RunningAssertion = {account: "assets:checking", op: "=", asserted: usd(1240000), calculated: null, ok: true};
    const failed: RunningAssertion = {account: "assets:checking", op: "==", asserted: usd(1240000), calculated: usd(1215000), ok: false};

    it("names the asserted account only when it is not the pinned root", () => {
        expect(assertionLabel(ok, "assets:checking")).toBe("✓");
        expect(assertionLabel({...ok, account: "assets:checking:joint"}, "assets:checking")).toBe("✓ joint");
        expect(assertionLabel({...failed, account: "assets:checking:joint"}, "assets")).toBe("✗ checking:joint");
    });

    it("says what was expected, what was found, and by how much it is off", () => {
        expect(assertionDetail(ok)).toBe("assets:checking = $12,400.00 holds");
        expect(assertionDetail(failed)).toBe("assets:checking == expected $12,400.00, found $12,150.00, off by $-250.00");
    });

    it("gives no difference across commodities", () => {
        const other: RunningAssertion = {...failed, asserted: {...usd(0), commodity: "VTI"}, calculated: {...usd(300), commodity: "VTI"}};
        expect(assertionDetail(other)).toContain("off by");
        expect(assertionDetail({...failed, calculated: {...usd(300), commodity: "EUR"}})).not.toContain("off by");
    });
});
