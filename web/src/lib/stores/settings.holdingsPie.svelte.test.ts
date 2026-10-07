// The Holdings pie's persisted dimension: restored when valid, and a value no
// version of the pie offers falls back to "By holding" rather than selecting
// nothing. Each case re-imports the store so it reads localStorage afresh.

import {afterEach, describe, expect, it, vi} from "vitest";
import {SETTINGS_STORAGE_KEY} from "$lib/api/client";

async function loadWith(saved: Record<string, unknown>) {
    localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(saved));
    vi.resetModules();
    return (await import("./settings.svelte")).settings;
}

afterEach(() => localStorage.removeItem(SETTINGS_STORAGE_KEY));

describe("COMPONENT settings.holdingsPieDimension", () => {
    it("defaults to by-holding", async () => {
        expect((await loadWith({})).holdingsPieDimension).toBe("holding");
    });

    it("restores a saved category view", async () => {
        expect((await loadWith({holdingsPieDimension: "sector"})).holdingsPieDimension).toBe("sector");
    });

    it("falls back when the saved value is not a view", async () => {
        expect((await loadWith({holdingsPieDimension: "astrology"})).holdingsPieDimension).toBe("holding");
    });

    it("persists a change", async () => {
        const settings = await loadWith({});
        settings.holdingsPieDimension = "assetClass";
        expect(JSON.parse(localStorage.getItem(SETTINGS_STORAGE_KEY) ?? "{}").holdingsPieDimension).toBe("assetClass");
    });
});
