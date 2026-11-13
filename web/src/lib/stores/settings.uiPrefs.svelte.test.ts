// The view state on the engine (plan 38 WP-5): `loadUiPrefs` takes the
// engine's values, moves an old localStorage blob's view state to the engine —
// once, then drops it — and every later change is one debounced PATCH naming
// only the keys that changed. Which file a key lands in is the engine's
// routing; the SPA only leaves out of the migration the keys the ledger's own
// file sets. An engine that leaves the view state to the browser (the e2e
// harness) or cannot answer keeps the pre-WP-5 behaviour: localStorage.

import {afterEach, describe, expect, it, vi} from "vitest";
import {SETTINGS_STORAGE_KEY} from "#lib/api/client.js";
import {defaultUiPrefs, type UiPrefs, type UiPrefsStorage} from "#lib/settings/uiPrefs.js";

const ENGINE = "http://engine.test";

interface Call {
    method: string;
    path: string;
    body: unknown;
}

/**
 * A fake `/api/ui-prefs`: GET answers `answer`; PATCH merges into it (or fails
 * with `patchStatus`); `POST /pin` adds or drops the key in `localKeys` (or
 * fails with `pinStatus`), an unpinned key falling back to `globalPrefs`.
 */
function engine(
    options: {
        storage?: UiPrefsStorage;
        localKeys?: string[];
        prefs?: Partial<UiPrefs>;
        globalPrefs?: Partial<UiPrefs>;
        patchStatus?: number;
        getStatus?: number;
        pinStatus?: number;
    } = {}
) {
    const calls: Call[] = [];
    let prefs: UiPrefs = {...defaultUiPrefs(), ...options.prefs};
    let localKeys = options.localKeys ?? [];
    const answer = () => ({prefs, storage: options.storage ?? "files", localKeys, issues: []});
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const method = init?.method ?? "GET";
        const body: unknown = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
        const path = new URL(String(input)).pathname;
        calls.push({method, path, body});
        if (path === "/api/ui-prefs/pin" && method === "POST") {
            if (options.pinStatus !== undefined) return Promise.resolve(new Response("no ledger file is open", {status: options.pinStatus}));
            const {key, pinned} = body as {key: "theme" | "uiScale"; pinned: boolean};
            localKeys = pinned ? [...localKeys.filter((k) => k !== key), key] : localKeys.filter((k) => k !== key);
            if (!pinned) prefs = {...prefs, [key]: {...defaultUiPrefs(), ...options.globalPrefs}[key]};
            return Promise.resolve(new Response(JSON.stringify(answer()), {status: 200}));
        }
        if (path !== "/api/ui-prefs") return Promise.resolve(new Response("not found", {status: 404}));
        if (method === "GET" && options.getStatus !== undefined) return Promise.resolve(new Response("nope", {status: options.getStatus}));
        if (method === "PATCH") {
            if (options.patchStatus !== undefined) return Promise.resolve(new Response("ledgeline.toml is not valid TOML", {status: options.patchStatus}));
            const patch = body as Partial<UiPrefs>;
            prefs = {...prefs, ...patch, columns: {...prefs.columns, ...(patch.columns ?? {})}};
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
    it("takes the engine's values, and never migrates over a key the ledger's own file sets", async () => {
        const fake = engine({localKeys: ["matchWindow", "columns.amount"], prefs: {matchWindow: 20, columns: {...defaultUiPrefs().columns, amount: false}}});
        const settings = await loadWith({serverUrl: ENGINE, matchWindow: 5, insightsOpen: false, columns: {amount: true, date: false}});
        expect(settings.uiPrefsReady).toBe(false);
        await settings.loadUiPrefs(ENGINE);
        expect(settings.uiPrefsReady).toBe(true);
        expect(fake.calls.map((c) => c.method)).toEqual(["GET", "PATCH"]);
        expect(fake.calls[1].body).toEqual({insightsOpen: false, columns: {date: false}});
        expect(settings.matchWindow).toBe(20);
        expect(settings.columns.amount).toBe(false);
        expect(settings.columns.date).toBe(false);
        expect(settings.insightsOpen).toBe(false);
        expect(persisted()).toEqual({serverUrl: ENGINE});
    });

    it("drops a blob whose every view setting the ledger's own file sets, without a PATCH", async () => {
        const fake = engine({localKeys: ["matchWindow"], prefs: {matchWindow: 20}});
        const settings = await loadWith({serverUrl: ENGINE, matchWindow: 5});
        await settings.loadUiPrefs(ENGINE);
        expect(fake.calls.map((c) => c.method)).toEqual(["GET"]);
        expect(settings.matchWindow).toBe(20);
        expect(persisted()).toEqual({serverUrl: ENGINE});
    });

    it("migrates an old blob's view state to the engine, once, then drops it from localStorage", async () => {
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

    it("saves theme and scale like any other key, clamping the scale and ignoring an unknown theme", async () => {
        const fake = engine();
        const settings = await loadWith({serverUrl: ENGINE});
        await settings.loadUiPrefs(ENGINE);
        expect(settings.theme).toBe("default");
        expect(settings.uiScale).toBe(100);
        vi.useFakeTimers();
        settings.theme = "catppuccin-mocha";
        settings.theme = "not-a-theme" as never;
        settings.uiScale = 1000;
        expect(settings.theme).toBe("catppuccin-mocha");
        expect(settings.uiScale).toBe(300);
        await vi.advanceTimersByTimeAsync(500);
        const patches = fake.calls.filter((c) => c.method === "PATCH");
        expect(patches.map((c) => c.body)).toEqual([{theme: "catppuccin-mocha", uiScale: 300}]);
        expect(persisted()).toEqual({serverUrl: ENGINE});
    });

    it("pins a key to the ledger after sending any pending save, and reads the pin from localKeys", async () => {
        const fake = engine({localKeys: ["matchWindow"]});
        const settings = await loadWith({serverUrl: ENGINE});
        await settings.loadUiPrefs(ENGINE);
        expect(settings.canPinToLedger).toBe(true);
        expect(settings.isPinned("theme")).toBe(false);
        vi.useFakeTimers();
        settings.theme = "nord";
        expect(await settings.pinToLedger("theme", true)).toBe(true);
        expect(fake.calls.filter((c) => c.method !== "GET").map((c) => [c.method, c.path, c.body])).toEqual([
            ["PATCH", "/api/ui-prefs", {theme: "nord"}],
            ["POST", "/api/ui-prefs/pin", {key: "theme", pinned: true}],
        ]);
        expect(settings.isPinned("theme")).toBe(true);
        expect(settings.isPinned("uiScale")).toBe(false);
        expect(settings.theme).toBe("nord");
    });

    it("unpinning applies the value the key falls back to", async () => {
        engine({localKeys: ["theme"], prefs: {theme: "dracula"}, globalPrefs: {theme: "github"}});
        const settings = await loadWith({serverUrl: ENGINE});
        await settings.loadUiPrefs(ENGINE);
        expect(settings.isPinned("theme")).toBe(true);
        expect(await settings.pinToLedger("theme", false)).toBe(true);
        expect(settings.isPinned("theme")).toBe(false);
        expect(settings.theme).toBe("github");
    });

    it("keeps a change made while a pin is in flight over the pin's answer", async () => {
        engine({localKeys: ["theme"], prefs: {theme: "dracula"}});
        const settings = await loadWith({serverUrl: ENGINE});
        await settings.loadUiPrefs(ENGINE);
        vi.useFakeTimers();
        const pinning = settings.pinToLedger("uiScale", false);
        settings.uiScale = 150;
        await pinning;
        expect(settings.uiScale).toBe(150);
    });

    it("reports a refused pin and leaves the pin state as it was", async () => {
        const fake = engine({pinStatus: 409});
        const settings = await loadWith({serverUrl: ENGINE});
        vi.spyOn(console, "warn").mockImplementation(() => undefined);
        await settings.loadUiPrefs(ENGINE);
        expect(await settings.pinToLedger("theme", true)).toBe(false);
        expect(settings.isPinned("theme")).toBe(false);
        expect(settings.uiPrefsNotices.join(" ")).toContain("Couldn't keep this setting for this ledger");
        expect(fake.calls.filter((c) => c.path === "/api/ui-prefs/pin")).toHaveLength(1);
    });

    it("cannot pin when the view state is kept in the browser", async () => {
        const fake = engine({storage: "browser"});
        const settings = await loadWith({serverUrl: ENGINE, theme: "nord", uiScale: 90});
        await settings.loadUiPrefs(ENGINE);
        expect(settings.canPinToLedger).toBe(false);
        expect(settings.theme).toBe("nord");
        expect(settings.uiScale).toBe(90);
        expect(await settings.pinToLedger("theme", true)).toBe(false);
        expect(fake.calls.map((c) => c.method)).toEqual(["GET"]);
        settings.uiScale = 110;
        expect(persisted()).toMatchObject({theme: "nord", uiScale: 110});
    });
});
