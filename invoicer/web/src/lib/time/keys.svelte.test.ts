// Time tracking's app-wide keys (plan 46 K), mounted: Ctrl+Shift+digit (as
// jsdom is not a Mac) toggles a slot, read from the physical key whatever the
// layout types; an empty slot asks for an account instead; Ctrl+Shift+. stops;
// Ctrl+Shift+↑/↓ move the running timer by the step; nothing fires before
// time tracking is set up, or while typing. The help sheet lists them under
// Time, one row for the ten slots.
import {render} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {keymap} from "#ui/keys/keymap.svelte.js";
import KeysProbe from "#ui/testing/fixtures/KeysProbe.svelte";
import {listenForKeys, press} from "#lib/testing/keys.js";
import {fakeApi, running, trackState} from "#lib/time/track/fixtures.js";
import {Tracker} from "#lib/time/track/tracker.svelte.js";
import type {TrackState} from "#lib/time/types.js";
import {timeAppLayer} from "./keys.js";

async function mounted(state: TrackState): Promise<{api: ReturnType<typeof fakeApi>; pick: ReturnType<typeof vi.fn>}> {
    const api = fakeApi(() => state);
    const tracker = new Tracker(api);
    await tracker.refresh();
    const pick = vi.fn();
    render(KeysProbe, {props: {layerOf: () => timeAppLayer(tracker, {pick})}});
    return {api, pick};
}

const ctrlShift = (code: string): KeyboardEventInit => ({ctrlKey: true, shiftKey: true, code});

describe("COMPONENT the app-wide time keys", () => {
    listenForKeys();

    it("toggles a slot with Ctrl+Shift+digit on US and German layouts", async () => {
        const {api} = await mounted(trackState());
        await press("@", document.body, ctrlShift("Digit2"));
        await Promise.resolve();
        expect(api.toggle).toHaveBeenLastCalledWith({slot: 2});
        // German Shift+3 types §.
        await press("§", document.body, ctrlShift("Digit3"));
        await vi.waitFor(() => expect(api.toggle).toHaveBeenLastCalledWith({slot: 3}));
    });

    it("asks for an account when the slot is empty", async () => {
        const {api, pick} = await mounted(trackState());
        await press(")", document.body, ctrlShift("Digit0"));
        expect(pick).toHaveBeenCalledWith(0);
        expect(api.toggle).not.toHaveBeenCalled();
    });

    it("stops, and moves the running timer by the step", async () => {
        const {api} = await mounted(running(trackState(), 2, 600));
        await press("ArrowUp", document.body, ctrlShift("ArrowUp"));
        await vi.waitFor(() => expect(api.adjust).toHaveBeenLastCalledWith({account: "clients:acme:advisory"}, 5));
        await press("ArrowDown", document.body, ctrlShift("ArrowDown"));
        await vi.waitFor(() => expect(api.adjust).toHaveBeenLastCalledWith({account: "clients:acme:advisory"}, -5));
        await press(">", document.body, ctrlShift("Period"));
        await vi.waitFor(() => expect(api.stop).toHaveBeenCalled());
    });

    it("does nothing before setup, or while typing in a field", async () => {
        const {api} = await mounted(trackState({configured: false}));
        await press("@", document.body, ctrlShift("Digit2"));
        const field = document.createElement("input");
        document.body.append(field);
        await press("@", field, ctrlShift("Digit2"));
        await Promise.resolve();
        expect(api.toggle).not.toHaveBeenCalled();
        field.remove();
    });

    it("lists the slots once in the help sheet's Time group, the stop and step keys while a timer runs", async () => {
        await mounted(running(trackState(), 2, 600));
        const time = keymap.help.find((section) => section.group === "Time");
        expect(time?.rows.map((r) => [r.keys, r.label])).toEqual([
            ["ctrl+shift+1", "Start or stop time slot 1 (2–9 and 0 likewise)"],
            ["ctrl+shift+.", "Stop the timer"],
            ["ctrl+shift+ArrowUp", "Add a step to the running timer"],
            ["ctrl+shift+ArrowDown", "Take a step off the running timer"],
        ]);
    });
});
