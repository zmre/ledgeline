import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {HoldingsScope} from "$lib/holdings/types";
import {benchmarkLines} from "./benchmarks.svelte";
import {settings} from "./settings.svelte";

const URL_BASE = "http://engine.test";
const scope = (gainPeriod: HoldingsScope["gainPeriod"]): HoldingsScope => ({accounts: new Set(), mode: "include", asOf: "2026-09-28", gainPeriod});

const line = (symbol: string, error: string | null = null) => ({
    symbol,
    label: symbol,
    points: [{date: "2026-09-28", value: error === null ? 1 : null}],
    pricedThrough: error === null ? "2026-09-25" : null,
    stale: false,
    unvaluedFlows: 0,
    error,
});

/** The server's answer: one line per requested symbol, in request order. */
const body = (symbols: string, failing: readonly string[] = []) => ({
    base: "$",
    benchmarks: symbols
        .split(",")
        .filter((s) => s !== "")
        .map((s) => line(s, failing.includes(s) ? `No ${s} price history covers this window.` : null)),
});

/** A fetch whose responses the test releases by hand, so ordering is under its control. */
function deferredFetch(failing: readonly string[] = []): {requests: string[]; symbols: (i: number) => string | null; release: (i: number) => void} {
    const requests: string[] = [];
    const resolvers: (() => void)[] = [];
    vi.stubGlobal("fetch", (input: unknown) => {
        const url = new URL(String(input));
        requests.push(url.search);
        return new Promise<Response>((resolve) => {
            resolvers.push(() => resolve(new Response(JSON.stringify(body(url.searchParams.get("symbols") ?? "", failing)), {status: 200})));
        });
    });
    return {requests, symbols: (i) => new URLSearchParams(requests[i]).get("symbols"), release: (i) => resolvers[i]?.()};
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** A loaded base series: only its identity matters to the store. */
const trend = (): object => ({points: []});

describe("UNIT benchmark overlay store", () => {
    beforeEach(() => benchmarkLines.reset());
    afterEach(() => vi.unstubAllGlobals());

    it("asks for every ticked symbol in ONE request, then only for newly ticked ones", async () => {
        const fetch = deferredFetch();
        const base = trend();
        benchmarkLines.sync(URL_BASE, scope("12mo"), base, ["SPY", "QQQ"]);
        benchmarkLines.sync(URL_BASE, scope("12mo"), base, ["SPY", "QQQ"]);

        expect(fetch.requests).toHaveLength(1);
        expect(fetch.symbols(0)).toBe("SPY,QQQ");
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("loading");
        expect(benchmarkLines.entries.get("QQQ")?.status).toBe("loading");
        fetch.release(0);
        await settle();
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("ready");
        expect(benchmarkLines.entries.get("QQQ")?.status).toBe("ready");

        benchmarkLines.sync(URL_BASE, scope("12mo"), base, ["SPY", "QQQ", "GLD"]);
        expect(fetch.requests).toHaveLength(2);
        expect(fetch.symbols(1)).toBe("GLD");
    });

    it("keeps a per-symbol error on that symbol alone", async () => {
        const fetch = deferredFetch(["GLD"]);
        benchmarkLines.sync(URL_BASE, scope("12mo"), trend(), ["SPY", "GLD"]);
        fetch.release(0);
        await settle();

        expect(benchmarkLines.entries.get("SPY")?.status).toBe("ready");
        const gld = benchmarkLines.entries.get("GLD");
        expect(gld?.status === "error" && gld.message).toContain("No GLD price history");
    });

    it("forgets every line when the window changes, and drops a response from the old one", async () => {
        const fetch = deferredFetch();
        benchmarkLines.sync(URL_BASE, scope("12mo"), trend(), ["SPY"]);
        benchmarkLines.sync(URL_BASE, scope("1wk"), trend(), ["SPY"]);

        expect(fetch.requests).toHaveLength(2);
        expect(new URLSearchParams(fetch.requests[1]).get("since")).toBe("2026-09-21");
        fetch.release(0); // the twelve-month answer arrives late
        await settle();
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("loading");
        fetch.release(1);
        await settle();
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("ready");
    });

    // Refresh, Update prices and a journal edit all reload the base series for
    // the SAME window: a line seeded from the old values must not survive it.
    it("refetches every line when the base series reloads for the same window", async () => {
        const fetch = deferredFetch();
        benchmarkLines.sync(URL_BASE, scope("12mo"), trend(), ["SPY", "QQQ"]);
        fetch.release(0);
        await settle();

        benchmarkLines.sync(URL_BASE, scope("12mo"), trend(), ["SPY", "QQQ"]);
        expect(fetch.requests).toHaveLength(2);
        expect(fetch.symbols(1)).toBe("SPY,QQQ");
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("loading");
    });

    it("records a failed request as the error of every benchmark in it", async () => {
        vi.stubGlobal("fetch", () => Promise.resolve(new Response("boom", {status: 500})));
        benchmarkLines.sync(URL_BASE, scope("12mo"), trend(), ["SPY", "QQQ"]);
        await settle();

        expect(benchmarkLines.entries.get("SPY")?.status).toBe("error");
        expect(benchmarkLines.entries.get("QQQ")?.status).toBe("error");
    });

    it("retries one symbol on its own", async () => {
        const fetch = deferredFetch(["GLD"]);
        const base = trend();
        benchmarkLines.sync(URL_BASE, scope("12mo"), base, ["SPY", "GLD"]);
        fetch.release(0);
        await settle();

        benchmarkLines.retry(URL_BASE, scope("12mo"), base, "GLD");
        expect(fetch.requests).toHaveLength(2);
        expect(fetch.symbols(1)).toBe("GLD");
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("ready");
    });
});

describe("UNIT settings benchmark selection", () => {
    afterEach(() => {
        for (const symbol of [...settings.benchmarks]) settings.toggleBenchmark(symbol, false);
    });

    it("keeps ticked benchmarks in catalog order, once each, and ignores unknown symbols", () => {
        settings.toggleBenchmark("GLD", true);
        settings.toggleBenchmark("SPY", true);
        settings.toggleBenchmark("SPY", true);
        settings.toggleBenchmark("AAPL", true);

        expect(settings.benchmarks).toEqual(["SPY", "GLD"]);
        settings.toggleBenchmark("SPY", false);
        expect(settings.benchmarks).toEqual(["GLD"]);
    });
});
