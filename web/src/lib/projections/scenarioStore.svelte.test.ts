// The scenario store, driven through the REAL `LedgelineApi` against a stubbed
// `fetch` (`$lib/testing/fakeEngine`) — the seam `web/README.md` names: a test
// that mocked the store would prove the page renders what it is handed, and what
// it is handed is the thing worth checking.
//
// Three behaviours live here and nowhere else:
//
//   1. THE SEED LATCH. Seeding is once per (server, reconnect), and it must
//      never run over unsaved work — an engine restarting under a half-edited
//      scenario replacing it with an average of the user's history is the worst
//      thing this tab could do.
//   2. THE 250 ms DEBOUNCE. Every edit is a `POST` carrying the whole scenario,
//      and typing an amount is several edits. A burst has to cost one request.
//   3. THE REVISION IN THE QUERY. A projection carries the scenario version it
//      answers, which is what lets the page refuse to chart a stale one (FE-1).
//
// It is a `.svelte.test.ts` because the store owns runes state and the fake
// engine needs `localStorage`, both of which want the jsdom project.

import {beforeEach, afterEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {scenarioStore, sameProjectionQuery, type ProjectionWindow} from "./scenarioStore.svelte";

const SEED_ROUTE = "/api/projections/seed?count=12&depth=2";
const RUN_ROUTE = "/api/projections/run";

const amount = (mantissa: string) => ({commodity: "$", quantity: {mantissa, places: 2}, precision: 2});

const SEED_BODY = {
    name: "",
    created: null,
    updated: null,
    lines: [
        {
            id: "gap:expenses:housing:$",
            group: "gap:expenses:housing:$",
            role: "flow",
            account: "expenses:housing",
            amount: amount("187500"),
            period: {raw: "monthly", simple: "monthly", from: null, to: null},
            growth: null,
            opening: null,
            note: "unbudgeted — average over 2025-08-01 to 2026-07-08",
            source: "unbudgeted",
        },
    ],
    events: [],
};

const RUN_BODY = {
    buckets: ["2026-08"],
    start: "2026-08-01",
    netIncome: {buckets: ["2026-08"], rows: [], totals: [{$: {mantissa: "0", places: 2}}]},
    cash: {opening: {$: {mantissa: "500000", places: 2}}, values: [{$: {mantissa: "500000", places: 2}}]},
    netWorth: {opening: {$: {mantissa: "900000", places: 2}}, values: [{$: {mantissa: "900000", places: 2}}]},
    runway: null,
    warnings: [],
};

const WINDOW: ProjectionWindow = {interval: "monthly", count: 24, depth: 2};

/** Every URL the store asked for, in order. */
let asked: string[] = [];

beforeEach(async () => {
    asked = [];
    await connectFakeEngine({
        [SEED_ROUTE]: SEED_BODY,
        [RUN_ROUTE]: RUN_BODY,
    });
    // Wrap the stub so the calls can be counted without a second `fetch` shim.
    const inner = globalThis.fetch;
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
        asked.push(String(url));
        return inner(url as unknown as RequestInfo, init);
    });
    // Back to a known, not-dirty state: the store is a module singleton shared
    // by every test in this file.
    scenarioStore.adopt({name: "", created: null, updated: null, lines: [], events: []});
});

afterEach(() => {
    scenarioStore.stop();
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

describe("COMPONENT scenarioStore — the seed latch", () => {
    it("seeds the table and leaves it not-dirty: seeding is not an edit", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 1);
        expect(scenarioStore.scenario.lines.map((l) => l.account)).toEqual(["expenses:housing"]);
        expect(scenarioStore.scenario.lines[0].source).toBe("unbudgeted");
        expect(scenarioStore.dirty).toBe(false);
    });

    it("adopts a MUTABLE copy — the decoder freezes what it returns", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 2);
        scenarioStore.scenario.lines[0].account = "expenses:rent";
        expect(scenarioStore.scenario.lines[0].account).toBe("expenses:rent");
    });

    it("reads the journal once per (server, reconnect), not once per visit", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 3);
        const first = asked.filter((url) => url.includes("/projections/seed")).length;
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 3);
        expect(asked.filter((url) => url.includes("/projections/seed")).length).toBe(first);
    });

    it("never seeds over unsaved work, even after a reconnect", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 4);
        scenarioStore.scenario.lines[0].account = "expenses:rent";
        scenarioStore.touch();
        expect(scenarioStore.dirty).toBe(true);

        // A new nonce IS a reconnect: the request goes out, and the answer is
        // deliberately not adopted.
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 5);
        expect(scenarioStore.scenario.lines[0].account).toBe("expenses:rent");
        expect(scenarioStore.dirty).toBe(true);
    });

    it("an explicit reseed DOES replace the table, and clears the edited flag", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 6);
        scenarioStore.scenario.lines[0].account = "expenses:rent";
        scenarioStore.touch();

        await scenarioStore.reseed(FAKE_ENGINE);
        expect(scenarioStore.scenario.lines[0].account).toBe("expenses:housing");
        expect(scenarioStore.dirty).toBe(false);
    });
});

describe("COMPONENT scenarioStore — the debounced recompute", () => {
    it("coalesces a burst of edits into ONE request", async () => {
        vi.useFakeTimers();
        for (let i = 0; i < 5; i += 1) {
            scenarioStore.touch();
            scenarioStore.schedule(FAKE_ENGINE, WINDOW);
        }
        expect(asked.filter((url) => url.endsWith(RUN_ROUTE))).toHaveLength(0);

        await vi.advanceTimersByTimeAsync(250);
        expect(asked.filter((url) => url.endsWith(RUN_ROUTE))).toHaveLength(1);
    });

    it("sends the scenario as it stands when the debounce FIRES, not when it was scheduled", async () => {
        vi.useFakeTimers();
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 7);
        scenarioStore.schedule(FAKE_ENGINE, WINDOW);
        // The point of coalescing: only the last edit is sent.
        scenarioStore.scenario.lines[0].account = "expenses:rent";
        scenarioStore.touch();

        await vi.advanceTimersByTimeAsync(250);
        const run = asked.filter((url) => url.endsWith(RUN_ROUTE));
        expect(run).toHaveLength(1);
        expect(scenarioStore.projection.value).not.toBeNull();
    });

    it("`stop` cancels a pending recompute, so an unmounted tab issues nothing", async () => {
        vi.useFakeTimers();
        scenarioStore.schedule(FAKE_ENGINE, WINDOW);
        scenarioStore.stop();
        await vi.advanceTimersByTimeAsync(500);
        expect(asked.filter((url) => url.endsWith(RUN_ROUTE))).toHaveLength(0);
    });

    it("a Retry skips the debounce entirely", async () => {
        await scenarioStore.runNow(FAKE_ENGINE, WINDOW);
        expect(asked.filter((url) => url.endsWith(RUN_ROUTE))).toHaveLength(1);
        expect(scenarioStore.projection.value?.start).toBe("2026-08-01");
    });
});

describe("COMPONENT scenarioStore — the payload and the question it answers", () => {
    it("carries the scenario revision it was computed for", async () => {
        await scenarioStore.runNow(FAKE_ENGINE, WINDOW);
        const held = scenarioStore.projection.query;
        expect(held).not.toBeNull();
        expect(sameProjectionQuery(held!, {...WINDOW, revision: scenarioStore.revision})).toBe(true);

        // One edit later, the held answer no longer answers the table.
        scenarioStore.touch();
        expect(sameProjectionQuery(held!, {...WINDOW, revision: scenarioStore.revision})).toBe(false);
    });

    it("a different window is a different question, at the same revision", async () => {
        await scenarioStore.runNow(FAKE_ENGINE, WINDOW);
        const held = scenarioStore.projection.query;
        expect(sameProjectionQuery(held!, {...WINDOW, count: 12, revision: scenarioStore.revision})).toBe(false);
        expect(sameProjectionQuery(held!, {...WINDOW, interval: "yearly", revision: scenarioStore.revision})).toBe(false);
        expect(sameProjectionQuery(held!, {...WINDOW, depth: 3, revision: scenarioStore.revision})).toBe(false);
    });
});
