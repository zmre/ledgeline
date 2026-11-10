// The Find Matches settings (plan 35 D8): a blob saved before they existed
// loads with their defaults, a stale or hand-edited number is clamped to its
// range, and every setter clamps and persists.

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

describe("COMPONENT settings — matches", () => {
    it("defaults to a 10-day window, a 365-day lookback and in transit after 10 days", async () => {
        const settings = await loadWith(null);
        expect(settings.matchWindow).toBe(10);
        expect(settings.matchLookback).toBe(365);
        expect(settings.matchTransitDays).toBe(10);
    });

    it("loads a blob saved before the match settings existed, keeping the review ones", async () => {
        const settings = await loadWith({reviewMaxDays: 30});
        expect(settings.reviewMaxDays).toBe(30);
        expect(settings.matchWindow).toBe(10);
        expect(settings.storageError).toBeNull();
    });

    it("clamps a stored value into its range, and replaces one that is not a number", async () => {
        const settings = await loadWith({matchWindow: 0, matchLookback: 99999, matchTransitDays: "soon"});
        expect(settings.matchWindow).toBe(1);
        expect(settings.matchLookback).toBe(36500);
        expect(settings.matchTransitDays).toBe(10);
    });

    it("clamps and persists each setter, keeping a lookback of 0 as 'no limit'", async () => {
        const settings = await loadWith(null);
        settings.matchWindow = 90;
        settings.matchLookback = 0;
        settings.matchTransitDays = 0.4;
        expect(settings.matchWindow).toBe(60);
        expect(settings.matchTransitDays).toBe(1);
        expect(persisted()).toMatchObject({matchWindow: 60, matchLookback: 0, matchTransitDays: 1});
    });
});
