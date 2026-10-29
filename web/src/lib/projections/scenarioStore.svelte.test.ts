// The scenario store, driven through the REAL `LedgelineApi` against a stubbed
// `fetch` (`#lib/testing/fakeEngine`) — the seam `web/README.md` names: a test
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
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {rawProjectionRun} from "#lib/testing/projectionFixture.js";
import {scenarioStore, scenarioTableView, sameProjectionQuery, type ProjectionWindow} from "./scenarioStore.svelte";

const SEED_ROUTE = "/api/projections/seed?count=12&depth=2";
/** The same seed, asked for one level deeper — the page's depth slider at 3. */
const DEEP_SEED_ROUTE = "/api/projections/seed?count=12&depth=3";
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
            period: {raw: "monthly", simple: "monthly", base: null, from: null, to: null},
            growth: null,
            opening: null,
            note: "unbudgeted — average over 2025-08-01 to 2026-07-08",
            counter: null,
            gains: null,
            source: "unbudgeted",
        },
    ],
    events: [],
    cash: null,
    autoRules: [],
    accountTypes: {},
};

const RUN_BODY = rawProjectionRun();

const WINDOW: ProjectionWindow = {interval: "monthly", count: 24, lookback: 2, depth: 2, accounts: null, journal: false};

/** Every URL the store asked for, in order. */
let asked: string[] = [];

beforeEach(async () => {
    asked = [];
    await connectFakeEngine({
        [SEED_ROUTE]: SEED_BODY,
        [DEEP_SEED_ROUTE]: {
            ...SEED_BODY,
            lines: [{...SEED_BODY.lines[0], id: "gap:expenses:housing:rent:$", group: "gap:expenses:housing:rent:$", account: "expenses:housing:rent"}],
        },
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
    scenarioStore.adopt({name: "", created: null, updated: null, lines: [], events: [], cash: null, autoRules: []});
});

afterEach(() => {
    scenarioStore.stop();
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

describe("COMPONENT scenarioStore — the seed latch", () => {
    it("seeds the table and leaves it not-dirty: seeding is not an edit", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 1, 2);
        expect(scenarioStore.scenario.lines.map((l) => l.account)).toEqual(["expenses:housing"]);
        expect(scenarioStore.scenario.lines[0].source).toBe("unbudgeted");
        expect(scenarioStore.dirty).toBe(false);
    });

    it("adopts a MUTABLE copy — the decoder freezes what it returns", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 2, 2);
        scenarioStore.scenario.lines[0].account = "expenses:rent";
        expect(scenarioStore.scenario.lines[0].account).toBe("expenses:rent");
    });

    it("reads the journal once per (server, reconnect), not once per visit", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 3, 2);
        const first = asked.filter((url) => url.includes("/projections/seed")).length;
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 3, 2);
        expect(asked.filter((url) => url.includes("/projections/seed")).length).toBe(first);
    });

    it("never seeds over unsaved work, even after a reconnect", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 4, 2);
        scenarioStore.scenario.lines[0].account = "expenses:rent";
        scenarioStore.touch();
        expect(scenarioStore.dirty).toBe(true);

        // A new nonce IS a reconnect: the request goes out, and the answer is
        // deliberately not adopted.
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 5, 2);
        expect(scenarioStore.scenario.lines[0].account).toBe("expenses:rent");
        expect(scenarioStore.dirty).toBe(true);
    });

    it("an explicit reseed DOES replace the table, and clears the edited flag", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 6, 2);
        scenarioStore.scenario.lines[0].account = "expenses:rent";
        scenarioStore.touch();

        await scenarioStore.reseed(FAKE_ENGINE, 2);
        expect(scenarioStore.scenario.lines[0].account).toBe("expenses:housing");
        expect(scenarioStore.dirty).toBe(false);
    });
});

describe("COMPONENT scenarioStore — the seed's depth", () => {
    it("seeds at the depth it is given, and remembers it", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 20, 3);
        expect(asked.some((url) => url.endsWith(DEEP_SEED_ROUTE))).toBe(true);
        expect(scenarioStore.scenario.lines.map((l) => l.account)).toEqual(["expenses:housing:rent"]);
        expect(scenarioStore.seedDepth).toBe(3);
    });

    it("a later depth is NOT a re-seed: the seed is only the starting point", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 21, 2);
        const seeds = asked.filter((url) => url.includes("/projections/seed")).length;
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 21, 3);
        expect(asked.filter((url) => url.includes("/projections/seed")).length).toBe(seeds);
        expect(scenarioStore.scenario.lines.map((l) => l.account)).toEqual(["expenses:housing"]);
        expect(scenarioStore.seedDepth).toBe(2);
    });

    it("an explicit reseed at a new depth replaces the table at that depth", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 22, 2);
        await scenarioStore.reseed(FAKE_ENGINE, 3);
        expect(scenarioStore.scenario.lines.map((l) => l.account)).toEqual(["expenses:housing:rent"]);
        expect(scenarioStore.seedDepth).toBe(3);
        expect(scenarioStore.dirty).toBe(false);
    });

    it("a scenario from a FILE has no seed depth", async () => {
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 23, 2);
        scenarioStore.adopt({name: "From a file", created: null, updated: null, lines: [], events: [], cash: null, autoRules: []});
        expect(scenarioStore.seedDepth).toBe(null);
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
        await scenarioStore.ensureSeeded(FAKE_ENGINE, 7, 2);
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
        expect(scenarioStore.projection.value?.actualsEnd).toBe("2026-08-01");
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

    it("asking for the Accounts or Journal reading, or changing a toggle, is a different question", async () => {
        await scenarioStore.runNow(FAKE_ENGINE, WINDOW);
        const held = scenarioStore.projection.query!;
        const revision = scenarioStore.revision;
        expect(sameProjectionQuery(held, {...WINDOW, journal: true, revision})).toBe(false);
        expect(sameProjectionQuery(held, {...WINDOW, accounts: {empty: false}, revision})).toBe(false);
        const accounts = {...held, accounts: {empty: false}};
        expect(sameProjectionQuery(accounts, {...WINDOW, accounts: {empty: false}, revision})).toBe(true);
        expect(sameProjectionQuery(accounts, {...WINDOW, accounts: {empty: true}, revision})).toBe(false);
    });
});

describe("COMPONENT scenarioTableView — what gates the table", () => {
    it("draws a scenario reopened from a file although the seed was never read", () => {
        // The reload bug: reopening the remembered file skips the seed, whose
        // status therefore stays `idle` — and a table gated on it span forever.
        expect(scenarioTableView(true, "idle", false)).toBe("data");
    });

    it("waits for, and reports, the seed when nothing has been adopted yet", () => {
        expect(scenarioTableView(false, "idle", false)).toBe("loading");
        expect(scenarioTableView(false, "loading", false)).toBe("loading");
        expect(scenarioTableView(false, "error", false)).toBe("error");
        expect(scenarioTableView(false, "ready", true)).toBe("data");
    });

    it("does not hide an adopted table behind a failed seed", () => {
        expect(scenarioTableView(true, "error", false)).toBe("data");
    });

    it("`adopted` is set by a load as well as by a seed", () => {
        scenarioStore.adopt({name: "From a file", created: null, updated: null, lines: [], events: [], cash: null, autoRules: []});
        expect(scenarioStore.adopted).toBe(true);
    });
});
