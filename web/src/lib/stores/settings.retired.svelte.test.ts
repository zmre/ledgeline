// Settings a later version dropped must still load harmlessly: a saved blob
// written before the Cash Flow chart lost its "By source" / "By account"
// toggle carries `cashFlowChartMode`. It must not break the load, must not
// reset the neighbouring flags, and is not written back on the next save.

import {afterEach, describe, expect, it, vi} from "vitest";
import {SETTINGS_STORAGE_KEY} from "$lib/api/client";

async function loadWith(saved: Record<string, unknown>) {
    localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(saved));
    vi.resetModules();
    return (await import("./settings.svelte")).settings;
}

afterEach(() => localStorage.removeItem(SETTINGS_STORAGE_KEY));

describe("COMPONENT settings — retired cashFlowChartMode", () => {
    it("loads a blob that still carries it, keeping the flags beside it", async () => {
        const settings = await loadWith({cashFlowChartMode: "source", cashFlowChartOpen: false});
        expect(settings.cashFlowChartOpen).toBe(false);
        expect(settings.storageError).toBeNull();
    });

    it("drops it on the next save", async () => {
        const settings = await loadWith({cashFlowChartMode: "account"});
        settings.cashFlowChartOpen = true;
        const saved = JSON.parse(localStorage.getItem(SETTINGS_STORAGE_KEY) ?? "{}") as Record<string, unknown>;
        expect(saved.cashFlowChartOpen).toBe(true);
        expect("cashFlowChartMode" in saved).toBe(false);
    });
});
