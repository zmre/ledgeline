import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {GainPeriod, HoldingsScope} from "$lib/holdings/types";
import {localToday} from "./filters.svelte";
import {defaultScope, holdingsData, holdingsScope, otherHoldingsData, trendQuery} from "./holdings.svelte";

describe("UNIT holdings scope store (module state, reset between tests)", () => {
    beforeEach(() => {
        holdingsScope.replace(defaultScope());
    });

    it("defaults to include-everything as of today, all-time gain (never a remembered date)", () => {
        expect(holdingsScope.value.asOf).toBe(localToday());
        expect(holdingsScope.value.mode).toBe("include");
        expect(holdingsScope.value.accounts.size).toBe(0);
        expect(holdingsScope.value.gainPeriod).toBe("all");
    });

    it("toggleAccount keeps the subtree-root invariant (same rules as the journal filters)", () => {
        holdingsScope.toggleAccount("assets:broker:taxable:vti");
        holdingsScope.toggleAccount("assets:broker:taxable:aapl");
        holdingsScope.toggleAccount("assets:broker");
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
        holdingsScope.toggleAccount("assets:broker:taxable:vti");
        expect(holdingsScope.value.accounts.size).toBe(0);
    });

    it("setMode switches include/exclude and keeps the selection", () => {
        holdingsScope.toggleAccount("assets:broker");
        holdingsScope.setMode("exclude");
        expect(holdingsScope.value.mode).toBe("exclude");
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
    });

    it("setAsOf changes only the date", () => {
        holdingsScope.toggleAccount("assets:broker");
        holdingsScope.setAsOf("2025-01-01");
        expect(holdingsScope.value.asOf).toBe("2025-01-01");
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
    });

    it("setGainPeriod changes only the gain window", () => {
        holdingsScope.toggleAccount("assets:broker");
        holdingsScope.setAsOf("2025-01-01");
        holdingsScope.setGainPeriod("ytd");
        expect(holdingsScope.value.gainPeriod).toBe("ytd");
        expect(holdingsScope.value.asOf).toBe("2025-01-01");
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
    });

    it("clear drops the selection but keeps mode, asOf and gain window", () => {
        holdingsScope.toggleAccount("assets:broker");
        holdingsScope.setMode("exclude");
        holdingsScope.setAsOf("2025-01-01");
        holdingsScope.setGainPeriod("12mo");
        holdingsScope.clear();
        expect(holdingsScope.value.accounts.size).toBe(0);
        expect(holdingsScope.value.mode).toBe("exclude");
        expect(holdingsScope.value.asOf).toBe("2025-01-01");
        expect(holdingsScope.value.gainPeriod).toBe("12mo");
    });

    it("replace swaps the whole scope and copies the account set", () => {
        const accounts = new Set(["assets:broker"]);
        holdingsScope.replace({accounts, mode: "exclude", asOf: "2024-12-31", gainPeriod: "ytd"});
        accounts.add("expenses"); // caller mutation must not leak in
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
        expect(holdingsScope.value.mode).toBe("exclude");
        expect(holdingsScope.value.asOf).toBe("2024-12-31");
        expect(holdingsScope.value.gainPeriod).toBe("ytd");
    });
});

describe("UNIT holdings trend window follows the gain period", () => {
    const at = (gainPeriod: GainPeriod): HoldingsScope => ({accounts: new Set(["assets:broker"]), mode: "exclude", asOf: "2026-09-28", gainPeriod});

    it("keeps the twelve-month chart exactly as it was", () => {
        expect(trendQuery(at("12mo"))).toEqual({asOf: "2026-09-28", accounts: "assets:broker", mode: "exclude", interval: "monthly", count: 12});
    });

    it("sends a since for the windows only the engine can count", () => {
        expect(trendQuery(at("1wk"))).toMatchObject({interval: "daily", since: "2026-09-21"});
        expect(trendQuery(at("all"))).toMatchObject({interval: "auto", since: "inception"});
        expect(trendQuery(at("all")).count).toBeUndefined();
    });

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("asks both tabs' series endpoints for the same window", async () => {
        const urls: string[] = [];
        vi.stubGlobal("fetch", (input: unknown) => {
            urls.push(String(input));
            return Promise.resolve(new Response("not wired", {status: 404}));
        });
        await holdingsData.load("http://engine.test", at("5yr"));
        await otherHoldingsData.load("http://engine.test", at("5yr"));
        const series = urls.filter((url) => url.includes("/series"));

        expect(series).toHaveLength(2);
        for (const url of series) {
            const params = new URL(url).searchParams;
            expect(params.get("interval")).toBe("monthly");
            expect(params.get("count")).toBe("60");
            expect(params.get("since")).toBeNull();
        }
        expect(urls.some((url) => url.includes("/api/holdings?") && url.includes("gainSince=2021-09-28"))).toBe(true);
    });
});
