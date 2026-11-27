// The lamp's tracker panel, opened from the lamp over a fake API: a click or
// `T` opens it (a labelled dialog, focus on the running slot), `T`, Escape
// or a click outside closes it with focus back on the lamp; the ten slots
// with today's totals; a digit or a click toggles a slot, 0 picks another
// account, an empty slot sends you to Track's picker; +/- move the running
// start; `n` writes the running line's note; before setup it points to
// Track's setup.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {listenForKeys, press} from "#lib/testing/keys.js";
import type {TrackState} from "#lib/time/types.js";
import {fakeApi, running, trackState} from "./fixtures.js";
import TimeLamp from "./TimeLamp.svelte";
import {Tracker} from "./tracker.svelte.js";

async function mount(state: TrackState = trackState()) {
    const api = fakeApi(() => state);
    const tracker = new Tracker(api, () => 1_000_000);
    await tracker.refresh();
    const onPick = vi.fn();
    render(TimeLamp, {tracker, trackHref: "/app/time/track", onPick, mac: true});
    return {api, tracker, onPick};
}

const lampButton = (): HTMLElement => within(screen.getByTestId("time-lamp")).getAllByRole("button")[0];
const panel = (): HTMLElement | null => screen.queryByTestId("time-lamp-panel");

async function settle(): Promise<void> {
    await new Promise((done) => setTimeout(done, 0));
}

describe("COMPONENT the lamp's tracker panel", () => {
    listenForKeys();

    it("opens on a click as a labelled dialog, focus on the running slot; Escape closes, focus back on the lamp", async () => {
        await mount(running(trackState(), 2, 600));
        await fireEvent.click(lampButton());
        const dialog = screen.getByRole("dialog", {name: "Time tracker"});
        expect(dialog).toBe(panel());
        expect(document.activeElement).toBe(screen.getByTestId("time-lamp-slot-2"));
        await press("Escape", document.activeElement as HTMLElement);
        await settle();
        expect(panel()).toBeNull();
        expect(document.activeElement).toBe(lampButton());
        expect(lampButton().getAttribute("aria-expanded")).toBe("false");
    });

    it("T opens and closes it from anywhere; a click outside closes it, a click on the lamp doesn't reopen it", async () => {
        await mount();
        await press("T");
        expect(panel()).not.toBeNull();
        expect(document.activeElement).toBe(screen.getByTestId("time-lamp-slot-1"));
        await press("T", document.activeElement as HTMLElement);
        await settle();
        expect(panel()).toBeNull();
        expect(document.activeElement).toBe(lampButton());

        await press("T");
        await fireEvent.pointerDown(document.body);
        await settle();
        expect(panel()).toBeNull();

        await fireEvent.click(lampButton());
        expect(panel()).not.toBeNull();
        await fireEvent.pointerDown(lampButton());
        await fireEvent.click(lampButton());
        await settle();
        expect(panel()).toBeNull();
    });

    it("shows the ten slots with their digit, name and today's total", async () => {
        await mount(running(trackState(), 2, 600));
        await press("T");
        const slots = within(screen.getByTestId("time-lamp-slots")).getAllByRole("button");
        expect(slots.map((b) => b.dataset.testid)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 0].map((k) => `time-lamp-slot-${k}`));
        expect(screen.getByTestId("time-lamp-slot-2").getAttribute("aria-label")).toBe("Slot 2: Acme Corp · advisory, running, 1h 10m today");
        expect(screen.getByTestId("time-lamp-slot-2").getAttribute("aria-pressed")).toBe("true");
        expect(screen.getByTestId("time-lamp-slot-4-today").textContent?.trim()).toBe("15m");
        expect(screen.getByTestId("time-lamp-slot-5").textContent).toContain("Empty pin");
        expect(screen.getByTestId("time-lamp-panel-today").textContent).toContain("1h 25m");
        expect(screen.getByTestId("time-lamp-panel-running").textContent).toContain("since 14:05");
    });

    it("a digit toggles its slot while it is open, and only then", async () => {
        const {api} = await mount();
        await press("3");
        expect(api.toggle).not.toHaveBeenCalled();
        await press("T");
        await press("3", document.activeElement as HTMLElement);
        expect(screen.getByTestId("time-lamp-slot-3").getAttribute("aria-pressed")).toBe("true");
        await vi.waitFor(() => expect(api.toggle).toHaveBeenCalledWith({slot: 3}));
    });

    it("a click on the running slot stops it", async () => {
        const {api} = await mount(running(trackState(), 2, 600));
        await press("T");
        await fireEvent.click(screen.getByTestId("time-lamp-slot-2"));
        expect(screen.getByTestId("time-lamp-slot-2").getAttribute("aria-pressed")).toBe("false");
        await vi.waitFor(() => expect(api.toggle).toHaveBeenCalledWith({slot: 2}));
    });

    it("Stop stops the running timer", async () => {
        const {api} = await mount(running(trackState(), 2, 600));
        await press("T");
        await fireEvent.click(screen.getByTestId("time-lamp-stop"));
        await vi.waitFor(() => expect(api.stop).toHaveBeenCalledTimes(1));
        expect(api.stop).toHaveBeenCalledWith({});
    });

    it("+/- and the step buttons move the running start", async () => {
        const {api} = await mount(running(trackState(), 2, 600));
        await press("T");
        await press("+", document.activeElement as HTMLElement);
        await vi.waitFor(() => expect(api.adjust).toHaveBeenCalledWith({account: "clients:acme:advisory"}, 5));
        await fireEvent.click(screen.getByTestId("time-lamp-minus"));
        await vi.waitFor(() => expect(api.adjust).toHaveBeenCalledWith({account: "clients:acme:advisory"}, -5));
    });

    it("n writes the running line's note; Escape in the note cancels only the note", async () => {
        const {api} = await mount(running(trackState(), 2, 600));
        await press("T");
        await press("n", document.activeElement as HTMLElement);
        const input = screen.getByTestId("time-note-input") as HTMLInputElement;
        expect(document.activeElement).toBe(input);
        expect(input.value).toBe("call with Fred about the K-1");
        await press("Escape", input);
        await settle();
        expect(screen.queryByTestId("time-note")).toBeNull();
        expect(panel()).not.toBeNull();
        expect(document.activeElement).toBe(screen.getByTestId("time-lamp-note-open"));

        await fireEvent.click(screen.getByTestId("time-lamp-note-open"));
        const again = screen.getByTestId("time-note-input");
        await fireEvent.input(again, {target: {value: "K-1 follow-up"}});
        await press("Enter", again);
        await vi.waitFor(() => expect(api.note).toHaveBeenCalledWith({account: "clients:acme:advisory"}, "K-1 follow-up"));
        await vi.waitFor(() => expect(screen.queryByTestId("time-note")).toBeNull());
    });

    it("0 picks another account and starts it; the picker's Escape leaves the panel open", async () => {
        const {api} = await mount();
        await press("T");
        await press("0", document.activeElement as HTMLElement);
        const picker = await screen.findByTestId("time-picker");
        await vi.waitFor(() => expect(within(picker).getAllByTestId("time-picker-option").length).toBeGreaterThan(0));
        await press("Escape", within(picker).getByTestId("time-picker-input"));
        await settle();
        expect(screen.queryByTestId("time-picker")).toBeNull();
        expect(panel()).not.toBeNull();

        await fireEvent.click(screen.getByTestId("time-lamp-slot-0"));
        const reopened = await screen.findByTestId("time-picker");
        await fireEvent.input(within(reopened).getByTestId("time-picker-input"), {target: {value: "initech"}});
        await fireEvent.click(within(reopened).getAllByTestId("time-picker-option")[0]);
        await vi.waitFor(() => expect(api.toggle).toHaveBeenCalledWith({account: "clients:initech:discovery"}));
        expect(screen.queryByTestId("time-picker")).toBeNull();
    });

    it("an empty slot sends you to Track's picker for it and closes", async () => {
        const {api, onPick} = await mount();
        await press("T");
        await press("5", document.activeElement as HTMLElement);
        expect(onPick).toHaveBeenCalledWith(5);
        expect(api.toggle).not.toHaveBeenCalled();
        await settle();
        expect(panel()).toBeNull();
    });

    it("Open Track links to the full page", async () => {
        await mount();
        await press("T");
        expect(screen.getByTestId("time-lamp-open-track").getAttribute("href")).toBe("/app/time/track");
    });

    it("before setup, points to Track's setup and ignores the digits", async () => {
        const {api} = await mount(trackState({configured: false}));
        await press("T");
        const link = screen.getByTestId("time-lamp-panel-setup");
        expect(link.getAttribute("href")).toBe("/app/time/track");
        expect(document.activeElement).toBe(link);
        expect(screen.queryByTestId("time-lamp-slots")).toBeNull();
        await press("1", link);
        expect(api.toggle).not.toHaveBeenCalled();
    });
});
