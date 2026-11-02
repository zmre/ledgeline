// The review settings (plan 32 D19): a blob saved before they existed loads
// with their defaults, a stale or hand-edited number is clamped, and every
// setter persists.

import {afterEach, describe, expect, it, vi} from "vitest";
import {SETTINGS_STORAGE_KEY} from "#lib/api/client.js";

async function loadWith(saved: Record<string, unknown> | null) {
    if (saved === null) localStorage.removeItem(SETTINGS_STORAGE_KEY);
    else localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(saved));
    vi.resetModules();
    return (await import("./settings.svelte")).settings;
}

const persisted = (): Record<string, unknown> => JSON.parse(localStorage.getItem(SETTINGS_STORAGE_KEY) ?? "{}") as Record<string, unknown>;

afterEach(() => localStorage.removeItem(SETTINGS_STORAGE_KEY));

describe("COMPONENT settings — review", () => {
    it("defaults to 50 recents, a 365-day window and markers on", async () => {
        const settings = await loadWith(null);
        expect(settings.reviewRecentLimit).toBe(50);
        expect(settings.reviewMaxDays).toBe(365);
        expect(settings.reviewMarkers).toBe(true);
    });

    it("loads a blob saved before the review settings existed, keeping what was there", async () => {
        const settings = await loadWith({insightsOpen: false});
        expect(settings.insightsOpen).toBe(false);
        expect(settings.reviewRecentLimit).toBe(50);
        expect(settings.storageError).toBeNull();
    });

    it("clamps or replaces a value that cannot be right", async () => {
        const settings = await loadWith({reviewRecentLimit: -4, reviewMaxDays: "lots", reviewMarkers: "no"});
        expect(settings.reviewRecentLimit).toBe(1);
        expect(settings.reviewMaxDays).toBe(365);
        expect(settings.reviewMarkers).toBe(true);
    });

    it("persists each one, and keeps 0 days as 'no limit'", async () => {
        const settings = await loadWith(null);
        settings.reviewRecentLimit = 20.4;
        settings.reviewMaxDays = 0;
        settings.reviewMarkers = false;
        expect(persisted()).toMatchObject({reviewRecentLimit: 20, reviewMaxDays: 0, reviewMarkers: false});
    });
});
