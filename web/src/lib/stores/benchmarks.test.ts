import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {HoldingsScope} from "$lib/holdings/types";
import {benchmarkLines} from "./benchmarks.svelte";
import {settings} from "./settings.svelte";

const URL_BASE = "http://engine.test";
const scope = (gainPeriod: HoldingsScope["gainPeriod"]): HoldingsScope => ({accounts: new Set(), mode: "include", asOf: "2026-09-28", gainPeriod});

const body = (symbol: string) => ({
    base: "$",
    benchmarks: [{symbol, label: symbol, points: [{date: "2026-09-28", value: 1}], pricedThrough: "2026-09-25", stale: false, unvaluedFlows: 0, error: null}],
});

/** A fetch whose responses the test releases by hand, so ordering is under its control. */
function deferredFetch(): {requests: string[]; release: (i: number) => void} {
    const requests: string[] = [];
    const resolvers: (() => void)[] = [];
    vi.stubGlobal("fetch", (input: unknown) => {
        const url = new URL(String(input));
        requests.push(url.search);
        return new Promise<Response>((resolve) => {
            resolvers.push(() => resolve(new Response(JSON.stringify(body(url.searchParams.get("symbols") ?? "")), {status: 200})));
        });
    });
    return {requests, release: (i) => resolvers[i]?.()};
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("UNIT benchmark overlay store", () => {
    beforeEach(() => benchmarkLines.reset());
    afterEach(() => vi.unstubAllGlobals());

    it("requests each ticked symbol once per window", async () => {
        const fetch = deferredFetch();
        benchmarkLines.sync(URL_BASE, scope("12mo"), ["SPY"]);
        benchmarkLines.sync(URL_BASE, scope("12mo"), ["SPY", "QQQ"]);
        benchmarkLines.sync(URL_BASE, scope("12mo"), ["SPY", "QQQ"]);

        expect(fetch.requests).toHaveLength(2);
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("loading");
        fetch.release(0);
        await settle();
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("ready");
    });

    it("forgets every line when the window changes, and drops a response from the old one", async () => {
        const fetch = deferredFetch();
        benchmarkLines.sync(URL_BASE, scope("12mo"), ["SPY"]);
        benchmarkLines.sync(URL_BASE, scope("1wk"), ["SPY"]);

        expect(fetch.requests).toHaveLength(2);
        expect(new URLSearchParams(fetch.requests[1]).get("since")).toBe("2026-09-21");
        fetch.release(0); // the twelve-month answer arrives late
        await settle();
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("loading");
        fetch.release(1);
        await settle();
        expect(benchmarkLines.entries.get("SPY")?.status).toBe("ready");
    });

    it("records a failed request as that benchmark's error", async () => {
        vi.stubGlobal("fetch", () => Promise.resolve(new Response("boom", {status: 500})));
        benchmarkLines.sync(URL_BASE, scope("12mo"), ["SPY"]);
        await settle();

        expect(benchmarkLines.entries.get("SPY")?.status).toBe("error");
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
