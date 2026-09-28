// The sources fetch is gated on somebody looking at it — the flows store's rule
// (see flows.svelte.test.ts), plus the chart's mode: "By account" draws the
// table's own report and must not cost a request.

import {flushSync} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {cashFlowSources, loadSourcesWhenWatched, sourcesMatch} from "./cashFlowSources.svelte";
import {settings} from "./settings.svelte";

const QUERY = {end: "2026-07-08", interval: "monthly" as const, count: 12, depth: 3};
const EMPTY = {buckets: [], rows: [], totals: []};

let requested: string[] = [];
let stop: () => void = () => {};

function watch(tab: () => string): void {
    stop = $effect.root(() => {
        loadSourcesWhenWatched(() => ({tab: tab(), query: QUERY}));
    });
    flushSync();
}

beforeEach(async () => {
    requested = [];
    vi.stubGlobal("fetch", (input: string) => {
        const url = String(input);
        requested.push(url);
        const body = url.endsWith("/version") ? "1.52" : EMPTY;
        return Promise.resolve(new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}}));
    });
    await settings.setServerUrl("http://engine");
    requested = [];
});

afterEach(() => {
    stop();
    vi.unstubAllGlobals();
    settings.cashFlowChartOpen = true;
    settings.cashFlowChartMode = "source";
});

describe("UNIT cash-flow sources are fetched only while the chart shows them", () => {
    it("fetches the table's own window on the Cash Flow tab, open, by source", () => {
        watch(() => "cf");

        expect(requested).toHaveLength(1);
        expect(requested[0]).toContain("/api/reports/cashflow/sources?end=2026-07-08&interval=monthly&count=12&depth=3");
    });

    it("issues nothing while the panel is shut", () => {
        settings.cashFlowChartOpen = false;
        watch(() => "cf");

        expect(requested).toEqual([]);
    });

    it("issues nothing in By account mode, and fetches on switching back", () => {
        settings.cashFlowChartMode = "account";
        watch(() => "cf");
        expect(requested).toEqual([]);

        settings.cashFlowChartMode = "source";
        flushSync();

        expect(requested).toHaveLength(1);
    });

    it("asks nothing again for a window it already requested: re-expanding, or toggling the mode back", async () => {
        watch(() => "cf");
        await vi.waitFor(() => expect(cashFlowSources.status).toBe("ready"));

        settings.cashFlowChartOpen = false;
        flushSync();
        settings.cashFlowChartOpen = true;
        flushSync();
        settings.cashFlowChartMode = "account";
        flushSync();
        settings.cashFlowChartMode = "source";
        flushSync();

        expect(requested).toHaveLength(1);
    });

    it("refetches the same window after a reconnect", async () => {
        watch(() => "cf");
        await vi.waitFor(() => expect(cashFlowSources.status).toBe("ready"));

        await settings.setServerUrl("http://engine");
        flushSync();

        expect(requested.filter((url) => url.includes("/cashflow/sources"))).toHaveLength(2);
    });

    it("issues nothing on another tab", () => {
        watch(() => "nw");

        expect(requested).toEqual([]);
    });
});

describe("UNIT sourcesMatch", () => {
    it("matches only the identical window", () => {
        expect(sourcesMatch(QUERY, {...QUERY})).toBe(true);
        expect(sourcesMatch(null, QUERY)).toBe(false);
        expect(sourcesMatch({...QUERY, depth: 2}, QUERY)).toBe(false);
        expect(sourcesMatch({...QUERY, interval: "quarterly"}, QUERY)).toBe(false);
    });
});
