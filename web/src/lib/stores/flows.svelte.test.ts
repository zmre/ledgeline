// The flows fetch is gated on somebody looking at it.
//
// The diagrams live behind their OWN endpoint partly so a shut panel costs
// nothing: building them is a second pass over every posting in the window, on
// top of the statement's. A gate that only checked the tab threw that away, and
// nothing about the screen would look wrong while it did, which is exactly the
// kind of cost that survives a review.
//
// The effect is driven here rather than the page: `loadFlowsWhenWatched` IS the
// page's effect (the page only supplies the tab and the window), so this
// exercises the real code path, not a copy of it.

import {flushSync} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {FLOW_REPORT} from "$lib/testing/flowsFixture";
import {loadFlowsWhenWatched} from "./flows.svelte";
import {settings} from "./settings.svelte";

const QUERY = {from: "2026-01-01", to: "2026-07-08"};

/** URLs the gate actually asked for, in order. */
let requested: string[] = [];
let stop: () => void = () => {};

/** Run the page's effect inside a root, and return a handle that flushes it. */
function watch(tab: () => string): void {
    stop = $effect.root(() => {
        loadFlowsWhenWatched(() => ({tab: tab(), query: QUERY}));
    });
    flushSync();
}

beforeEach(async () => {
    requested = [];
    vi.stubGlobal("fetch", (input: string) => {
        const url = String(input);
        requested.push(url);
        // `setServerUrl` probes /version before it will store anything, and it
        // wants a bare JSON string back.
        const body = url.endsWith("/version") ? "1.52" : FLOW_REPORT;
        return Promise.resolve(new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}}));
    });
    await settings.setServerUrl("http://engine");
});

afterEach(() => {
    stop();
    vi.unstubAllGlobals();
    settings.flowsInOpen = true;
    settings.flowsOutOpen = true;
});

describe("UNIT flows are fetched only while a panel is watching", () => {
    it("issues nothing when both panels are shut", () => {
        settings.flowsInOpen = false;
        settings.flowsOutOpen = false;
        requested = [];

        watch(() => "is");

        expect(requested).toEqual([]);
    });

    it("issues exactly one request when a shut panel is expanded", () => {
        settings.flowsInOpen = false;
        settings.flowsOutOpen = false;
        requested = [];
        watch(() => "is");

        // The flags are read INSIDE the effect, which is what makes this fire.
        settings.flowsInOpen = true;
        flushSync();

        expect(requested.length).toBe(1);
        expect(requested[0]).toContain("/api/reports/incomestatement/flows");
        expect(requested[0]).toContain("from=2026-01-01");
        expect(requested[0]).toContain("to=2026-07-08");
    });

    it("issues nothing on a tab that draws no diagrams, however open the panels are", () => {
        requested = [];
        watch(() => "bs");

        expect(settings.flowsInOpen).toBe(true);
        expect(requested).toEqual([]);
    });

    it("fetches once the P&L tab is the one being viewed", () => {
        // A real signal, as `params.tab` is on the page: the effect has to
        // re-run off the tab itself, not off something else that happened to
        // change at the same moment.
        const nav = $state({tab: "bs"});
        requested = [];
        watch(() => nav.tab);
        expect(requested).toEqual([]);

        nav.tab = "is";
        flushSync();

        expect(requested.length).toBe(1);
    });
});
