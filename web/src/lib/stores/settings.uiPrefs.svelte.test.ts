// The view state on the engine (plan 38 WP-5): `loadUiPrefs` takes the
// engine's values, migrates an old localStorage blob into a ledger that has no
// UI preferences yet — once, then drops it — and every later change is one
// debounced PATCH naming only the keys that changed. An engine that leaves the
// view state to the browser (the e2e harness) or cannot answer keeps the
// pre-WP-5 behaviour: localStorage.

import {afterEach, describe, expect, it, vi} from "vitest";
import {SETTINGS_STORAGE_KEY} from "#lib/api/client.js";
import {defaultUiPrefs, type UiPrefs, type UiPrefsStorage} from "#lib/settings/uiPrefs.js";

const ENGINE = "http://engine.test";

interface Call {
    method: string;
    body: unknown;
}

/** A fake `/api/ui-prefs`: GET answers `answer`; PATCH merges into it (or fails with `patchStatus`). */
function engine(options: {storage?: UiPrefsStorage; ledgerHasUi?: boolean; prefs?: Partial<UiPrefs>; patchStatus?: number; getStatus?: number} = {}) {
    const calls: Call[] = [];
    let prefs: UiPrefs = {...defaultUiPrefs(), ...options.prefs};
    let ledgerHasUi = options.ledgerHasUi ?? false;
    const answer = () => ({prefs, storage: options.storage ?? "ledger", ledgerHasUi, issues: []});
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const method = init?.method ?? "GET";
        const body: unknown = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
        calls.push({method, body});
        if (!String(input).endsWith("/api/ui-prefs")) return Promise.resolve(new Response("not found", {status: 404}));
        if (method === "GET" && options.getStatus !== undefined) return Promise.resolve(new Response("nope", {status: options.getStatus}));
        if (method === "PATCH") {
            if (options.patchStatus !== undefined) return Promise.resolve(new Response("ledgeline.toml is not valid TOML", {status: options.patchStatus}));
            const patch = body as Partial<UiPrefs>;
            prefs = {...prefs, ...patch, columns: {...prefs.columns, ...(patch.columns ?? {})}};
            ledgerHasUi = true;
        }
        return Promise.resolve(new Response(JSON.stringify(answer()), {status: 200}));
    });
    return {calls, current: () => prefs};
}

async function loadWith(saved: Record<string, unknown> | null) {
    if (saved === null) localStorage.removeItem(SETTINGS_STORAGE_KEY);
    else localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(saved));
    vi.resetModules();
    return (await import("./settings.svelte")).settings;
}

const persisted = (): Record<string, unknown> => JSON.parse(localStorage.getItem(SETTINGS_STORAGE_KEY) ?? "{}") as Record<string, unknown>;

afterEach(() => {
    localStorage.removeItem(SETTINGS_STORAGE_KEY);
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

describe("COMPONENT settings — view state on the engine", () => {
    it("takes the ledger's values over the browser's, and leaves the blob alone when the ledger already has some", async () => {
        const fake = engine({ledgerHasUi: true, prefs: {matchWindow: 20, columns: {...defaultUiPrefs().columns, amount: false}}});
        const settings = await loadWith({serverUrl: ENGINE, matchWindow: 5});
        expect(settings.uiPrefsReady).toBe(false);
        await settings.loadUiPrefs(ENGINE);
        expect(settings.uiPrefsReady).toBe(true);
        expect(settings.matchWindow).toBe(20);
        expect(settings.columns.amount).toBe(false);
        expect(fake.calls.map((c) => c.method)).toEqual(["GET"]);
        expect(persisted()).toEqual({serverUrl: ENGINE, matchWindow: 5});
    });

    it("migrates an old blob's view state into a ledger with none, once, then drops it from localStorage", async () => {
        const fake = engine();
        const settings = await loadWith({serverUrl: ENGINE, serverToken: "tok", insightsOpen: false, columns: {date: false}, matchWindow: 90});
        await settings.loadUiPrefs(ENGINE);
        expect(fake.calls.map((c) => c.method)).toEqual(["GET", "PATCH"]);
        expect(fake.calls[1].body).toEqual({insightsOpen: false, columns: {date: false}, matchWindow: 60});
        expect(settings.insightsOpen).toBe(false);
        expect(settings.columns.date).toBe(false);
        expect(persisted()).toEqual({serverUrl: ENGINE, serverToken: "tok"});

        // The next launch finds nothing to migrate.
        const again = await loadWith(persisted());
        await again.loadUiPrefs(ENGINE);
        expect(fake.calls.map((c) => c.method)).toEqual(["GET", "PATCH", "GET"]);
        expect(again.insightsOpen).toBe(false);
    });

    it("keeps the blob, and its values for this session, when the migration cannot be written", async () => {
        engine({patchStatus: 409});
        const settings = await loadWith({serverUrl: ENGINE, insightsOpen: false});
        vi.spyOn(console, "warn").mockImplementation(() => undefined);
        await settings.loadUiPrefs(ENGINE);
        expect(settings.insightsOpen).toBe(false);
        expect(persisted()).toEqual({serverUrl: ENGINE, insightsOpen: false});
    });

    it("saves one debounced PATCH naming only what changed, and stops writing view state to localStorage", async () => {
        const fake = engine();
        const settings = await loadWith({serverUrl: ENGINE});
        await settings.loadUiPrefs(ENGINE);
        vi.useFakeTimers();
        settings.matchWindow = 12;
        settings.columns = {...settings.columns, status: false};
        settings.toggleBenchmark("SPY", true);
        settings.matchWindow = 14;
        expect(fake.calls.filter((c) => c.method === "PATCH")).toHaveLength(0);
        await vi.advanceTimersByTimeAsync(500);
        const patches = fake.calls.filter((c) => c.method === "PATCH");
        expect(patches).toHaveLength(1);
        expect(patches[0].body).toEqual({matchWindow: 14, columns: {status: false}, benchmarks: ["SPY"]});
        expect(fake.current().matchWindow).toBe(14);
        expect(persisted()).toEqual({serverUrl: ENGINE});
    });

    it("reports a failed save rather than throwing", async () => {
        engine({patchStatus: 409});
        const settings = await loadWith({serverUrl: ENGINE});
        vi.spyOn(console, "warn").mockImplementation(() => undefined);
        await settings.loadUiPrefs(ENGINE);
        settings.reviewMarkers = false;
        await settings.flushUiPrefs();
        expect(settings.reviewMarkers).toBe(false);
        expect(settings.uiPrefsNotices.join(" ")).toContain("Couldn't save view settings");
    });

    it("leaves the view state in localStorage when the engine keeps it in the browser", async () => {
        const fake = engine({storage: "browser"});
        const settings = await loadWith({serverUrl: ENGINE, insightsOpen: false});
        await settings.loadUiPrefs(ENGINE);
        expect(settings.insightsOpen).toBe(false);
        settings.matchWindow = 30;
        expect(persisted()).toMatchObject({serverUrl: ENGINE, insightsOpen: false, matchWindow: 30});
        expect(fake.calls.map((c) => c.method)).toEqual(["GET"]);
    });

    it("falls back to localStorage when the engine has no such route", async () => {
        engine({getStatus: 404});
        vi.spyOn(console, "warn").mockImplementation(() => undefined);
        const settings = await loadWith({serverUrl: ENGINE, reviewMaxDays: 30});
        await settings.loadUiPrefs(ENGINE);
        expect(settings.uiPrefsReady).toBe(true);
        expect(settings.reviewMaxDays).toBe(30);
        settings.reviewMaxDays = 40;
        expect(persisted()).toMatchObject({reviewMaxDays: 40});
    });
});
