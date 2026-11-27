// The shared UI prefs (SETTINGS-PLAN §2): the theme saves at once, the text
// size once a zoom burst settles (and as the page hides), the matching
// settings only on Save, and the books' pin through its own route.
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {isVariantId} from "#ui/theme/catalog.js";
import {resolveTheme} from "#ui/theme/resolve.js";
import {DEDUPE_MS, resetZoomForTests, setScaleSink, zoomIn} from "#ui/theme/zoom.svelte.js";
import {type Matching, UiPrefs, type UiPrefsAnswer, type UiPrefsApi} from "./uiPrefs.svelte.js";

function fakeApi(answer: Partial<UiPrefsAnswer> = {}) {
    const body = (prefs: Record<string, unknown> = {}): UiPrefsAnswer => ({
        prefs: {theme: "dracula", uiScale: 125, settlementLookbackDays: 365, settlementToleranceBps: 250, settlementLeadDays: 4, ...prefs},
        storage: "files",
        localKeys: [],
        issues: [],
        ...answer,
    });
    const api = {
        load: vi.fn(() => Promise.resolve(body())),
        patch: vi.fn((values: Record<string, unknown>) => Promise.resolve(body(values))),
        pin: vi.fn((key: string, pinned: boolean) => Promise.resolve({...body(), localKeys: pinned ? [key] : []})),
        beacon: vi.fn(),
    } satisfies UiPrefsApi;
    return api;
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
    vi.useRealTimers();
    resetZoomForTests();
});

describe("COMPONENT UiPrefs", () => {
    it("starts on a theme that resolves in both schemes, and not ready", () => {
        const prefs = new UiPrefs(fakeApi(), 400);
        expect(isVariantId(resolveTheme(prefs.theme, false))).toBe(true);
        expect(isVariantId(resolveTheme(prefs.theme, true))).toBe(true);
        expect(prefs.uiPrefsReady).toBe(false);
        expect(prefs.canPin).toBe(false);
    });

    it("loads the saved theme, scale and matching settings, leaving Ledgeline's tolerance out", async () => {
        const prefs = new UiPrefs(fakeApi(), 400);
        await prefs.load();
        expect([prefs.theme, prefs.uiScale, prefs.uiPrefsReady, prefs.canPin]).toEqual(["dracula", 125, true, true]);
        // The prefs carry `settlementToleranceBps` (Ledgeline's); it does nothing here.
        expect(prefs.matching).toEqual({settlementLookbackDays: 365, settlementLeadDays: 4});
    });

    it("never sends the tolerance, even when asked to save it", async () => {
        const api = fakeApi();
        const prefs = new UiPrefs(api, 400);
        await prefs.load();
        await prefs.saveMatching({settlementLeadDays: 6, settlementToleranceBps: 100} as Partial<Matching>);
        expect(api.patch).toHaveBeenCalledWith({settlementLeadDays: 6});
    });

    it("loads and saves whether the sidebar's Library is open, at once", async () => {
        const api = fakeApi();
        api.load.mockResolvedValueOnce({prefs: {sidebarLibraryOpen: true}, storage: "files", localKeys: [], issues: []});
        const prefs = new UiPrefs(api, 400);
        expect(prefs.libraryOpen).toBe(false);
        await prefs.load();
        expect(prefs.libraryOpen).toBe(true);
        prefs.libraryOpen = false;
        await vi.advanceTimersByTimeAsync(0);
        expect(api.patch).toHaveBeenCalledWith({sidebarLibraryOpen: false});
        prefs.libraryOpen = false;
        await vi.advanceTimersByTimeAsync(0);
        expect(api.patch).toHaveBeenCalledTimes(1);
    });

    it("saves the theme at once and the text size after a zoom burst settles", async () => {
        const api = fakeApi();
        const prefs = new UiPrefs(api, 400);
        prefs.theme = "catppuccin";
        await vi.advanceTimersByTimeAsync(0);
        expect(api.patch).toHaveBeenLastCalledWith({theme: "catppuccin"});

        // The zoom keys step the saved scale through the sink, as the layout wires it.
        setScaleSink(
            (p) => (prefs.uiScale = p),
            () => prefs.uiScale
        );
        zoomIn();
        vi.advanceTimersByTime(DEDUPE_MS);
        zoomIn();
        expect(prefs.uiScale).toBe(125);
        expect(api.patch).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(400);
        expect(api.patch).toHaveBeenCalledTimes(2);
        expect(api.patch).toHaveBeenLastCalledWith({uiScale: 125});
    });

    it("sends a waiting scale as the page hides, with a request that outlives it", () => {
        const api = fakeApi();
        const prefs = new UiPrefs(api, 400);
        const stop = prefs.flushWhenLeaving(window);
        prefs.uiScale = 150;
        window.dispatchEvent(new Event("pagehide"));
        expect(api.beacon).toHaveBeenCalledWith({uiScale: 150});
        expect(api.patch).not.toHaveBeenCalled();
        stop();
    });

    it("saves only the matching settings that changed", async () => {
        const api = fakeApi();
        const prefs = new UiPrefs(api, 400);
        await prefs.load();
        await prefs.saveMatching({settlementLookbackDays: 365, settlementLeadDays: 7});
        expect(api.patch).toHaveBeenCalledWith({settlementLeadDays: 7});
        expect(prefs.matching.settlementLeadDays).toBe(7);
    });

    it("pins a key to these books, and says so when it can't", async () => {
        const api = fakeApi();
        const prefs = new UiPrefs(api, 400);
        await prefs.load();
        expect(await prefs.pin("settlementLeadDays", true)).toBe(true);
        expect(api.pin).toHaveBeenCalledWith("settlementLeadDays", true);
        expect(prefs.isPinned("settlementLeadDays")).toBe(true);

        api.pin.mockRejectedValueOnce(new Error("no ledger file is open"));
        expect(await prefs.pin("theme", true)).toBe(false);
        expect(prefs.failure).toContain("no ledger file is open");
        expect(prefs.isPinned("settlementLeadDays")).toBe(true);
    });

    it("offers no pin when the prefs live in the browser", async () => {
        const api = fakeApi({storage: "browser"});
        const prefs = new UiPrefs(api, 400);
        await prefs.load();
        expect(prefs.canPin).toBe(false);
        expect(await prefs.pin("theme", true)).toBe(false);
        expect(api.pin).not.toHaveBeenCalled();
    });
});
