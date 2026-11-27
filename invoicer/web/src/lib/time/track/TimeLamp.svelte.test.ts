// The sidebar lamp (T-light, T17): green ▶ running with the live elapsed
// time and its key, grey ■ stopped with the resume key; the accessible name
// says which; a click opens the tracker (LampPanel.svelte.test.ts has the
// panel itself); a quiet setup button before time tracking exists; nothing
// before the state is known.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {fakeApi, running, trackState} from "./fixtures.js";
import TimeLamp from "./TimeLamp.svelte";
import {Tracker} from "./tracker.svelte.js";
import type {TrackState} from "#lib/time/types.js";

async function lamp(state: TrackState | null, mac = true): Promise<{tracker: Tracker; api: ReturnType<typeof fakeApi>}> {
    const api = fakeApi(() => state ?? trackState());
    const tracker = new Tracker(api, () => 1_000_000);
    if (state !== null) await tracker.refresh();
    render(TimeLamp, {tracker, trackHref: "/app/time/track", mac});
    return {tracker, api};
}

describe("COMPONENT the time lamp", () => {
    it("shows nothing while the state is unknown", async () => {
        await lamp(null);
        expect(screen.queryByTestId("time-lamp")).toBeNull();
    });

    it("running: green ▶, the account, the elapsed time and its key; click opens the tracker", async () => {
        const {api} = await lamp(running(trackState(), 2, 3600 + 22 * 60));
        const button = screen.getByTestId("time-lamp-running");
        expect(button.getAttribute("aria-label")).toBe("Time tracking running: Acme Corp · advisory, 1h 22m");
        expect(button.getAttribute("aria-haspopup")).toBe("dialog");
        expect(button.getAttribute("aria-expanded")).toBe("false");
        expect(button.textContent).toContain("▶");
        expect(button.textContent).toContain("⌥⌘2");
        expect(button.className).toContain("success");
        expect(button.className).not.toMatch(/error|red/);
        expect(screen.getByTestId("time-lamp-elapsed").textContent).toBe("1h 22m");
        await fireEvent.click(button);
        expect(button.getAttribute("aria-expanded")).toBe("true");
        expect(screen.getByTestId("time-lamp-panel").id).toBe(button.getAttribute("aria-controls"));
        expect(api.stop).not.toHaveBeenCalled();
    });

    it("stopped: grey ■, the last account and the key that resumes it", async () => {
        const {api} = await lamp(trackState(), false);
        const button = screen.getByTestId("time-lamp-stopped");
        expect(button.getAttribute("aria-label")).toBe("Time tracking stopped, last Internal · admin");
        expect(button.textContent).toContain("■");
        expect(screen.getByTestId("time-lamp-hint").textContent).toBe("Ctrl+Shift+4 to resume");
        await fireEvent.click(button);
        expect(screen.getByTestId("time-lamp-panel")).toBeTruthy();
        expect(api.toggle).not.toHaveBeenCalled();
    });

    it("a slotless last account has no resume key to show", async () => {
        await lamp(trackState({last: {account: "internal:training", label: "Internal · training", slot: null, date: "2026-11-25"}}));
        expect(screen.getByTestId("time-lamp-stopped").textContent).toContain("Internal · training");
        expect(screen.queryByTestId("time-lamp-hint")).toBeNull();
    });

    it("offers setup, quietly, before time tracking exists", async () => {
        await lamp(trackState({configured: false}));
        const button = screen.getByTestId("time-lamp-setup");
        expect(button.textContent?.trim()).toBe("Set up time tracking");
        await fireEvent.click(button);
        expect(screen.getByTestId("time-lamp-panel-setup").getAttribute("href")).toBe("/app/time/track");
    });
});
