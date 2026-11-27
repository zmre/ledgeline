// The long-running guard: says how long and since when, and maps Keep all,
// Stop at last activity, Stop at… and Discard onto the stop route's choices.
// Escape keeps (nothing lost).
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {Guard} from "#lib/time/types.js";
import {qty, running, trackState} from "./fixtures.js";
import GuardDialog from "./GuardDialog.svelte";

function mount(guard: Partial<Guard> = {}) {
    const timer = running(trackState(), 2, 26 * 3600, "2026-11-25T14:05:12").running;
    if (timer === null) throw new Error("no timer");
    const handlers = {onKeep: vi.fn(), onStopAt: vi.fn(), onDiscard: vi.fn()};
    render(GuardDialog, {
        guard: {age: qty(26 * 3600), previous_day: true, threshold_minutes: 240, last_activity: null, ...guard},
        running: timer,
        ...handlers,
    });
    return handlers;
}

describe("COMPONENT the long-running guard", () => {
    it("says it started on an earlier day, and Keep all only stops asking", async () => {
        const {onKeep, onStopAt, onDiscard} = mount();
        const dialog = screen.getByTestId("time-guard");
        expect(dialog.textContent).toContain("Still working on Acme Corp · advisory?");
        expect(dialog.textContent).toContain("started on an earlier day, Wed 25 Nov at 14:05");
        expect(screen.queryByTestId("time-guard-last")).toBeNull();
        expect(document.activeElement).toBe(screen.getByTestId("time-guard-keep"));
        await fireEvent.click(screen.getByTestId("time-guard-keep"));
        expect(onKeep).toHaveBeenCalled();
        expect(onStopAt).not.toHaveBeenCalled();
        expect(onDiscard).not.toHaveBeenCalled();
    });

    it("stops at the last activity when one is known", async () => {
        const {onStopAt} = mount({last_activity: "2026-11-25T17:30:00"});
        await fireEvent.click(screen.getByTestId("time-guard-last"));
        expect(onStopAt).toHaveBeenCalledWith("2026-11-25T17:30:00");
    });

    it("stops at a time you give, after the start only", async () => {
        const {onStopAt} = mount();
        await fireEvent.click(screen.getByTestId("time-guard-stop-at"));
        const at = screen.getByTestId("time-guard-at") as HTMLInputElement;
        expect(at.value).toBe("2026-11-25T18:05");
        await fireEvent.input(at, {target: {value: "2026-11-25T12:00"}});
        await fireEvent.click(screen.getByTestId("time-guard-at-confirm"));
        expect(onStopAt).not.toHaveBeenCalled();
        await fireEvent.input(at, {target: {value: "2026-11-25T19:15"}});
        await fireEvent.click(screen.getByTestId("time-guard-at-confirm"));
        expect(onStopAt).toHaveBeenCalledWith("2026-11-25T19:15:00");
    });

    it("discards", async () => {
        const {onDiscard} = mount();
        await fireEvent.click(screen.getByTestId("time-guard-discard"));
        expect(onDiscard).toHaveBeenCalled();
    });

    it("keeps on Escape", async () => {
        const {onKeep} = mount();
        await fireEvent.keyDown(screen.getByTestId("time-guard-keep"), {key: "Escape"});
        expect(onKeep).toHaveBeenCalled();
    });
});
