// Track, mounted over a fake API: the ten slots (pins, recent, Other…) with
// today's totals and the running one's since-time; its bare keys — a digit
// toggles, 0 picks another account (or creates one), +/- adjust the
// selected slot, Shift+- snaps, n writes a note, . stops, e changes a pin,
// c shows today's card, ↑/↓ select; and the help sheet's Track group.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {keymap} from "#ui/keys/keymap.svelte.js";
import {listenForKeys, press} from "#lib/testing/keys.js";
import type {TrackState} from "#lib/time/types.js";
import {fakeApi, running, trackState} from "./fixtures.js";
import {Tracker} from "./tracker.svelte.js";
import TrackView from "./TrackView.svelte";

async function mount(state: TrackState = trackState(), pick: number | null = null) {
    let current = state;
    const api = fakeApi(() => current);
    const tracker = new Tracker(api, () => 1_000_000);
    await tracker.refresh();
    render(TrackView, {tracker, pick});
    return {api, tracker, set: (next: TrackState) => (current = next)};
}

describe("COMPONENT Track", () => {
    listenForKeys();

    it("shows ten slots: pins, recent, and Other…, each with today's total", async () => {
        await mount(running(trackState(), 2, 600));
        const slots = within(screen.getByTestId("time-slots")).getAllByRole("button");
        expect(slots.map((b) => b.dataset.testid)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 0].map((k) => `time-slot-${k}`));
        expect(screen.getByTestId("time-slot-2").getAttribute("aria-label")).toBe("Slot 2: Acme Corp · advisory, running, 1h 10m today");
        expect(screen.getByTestId("time-slot-2").getAttribute("aria-pressed")).toBe("true");
        expect(screen.getByTestId("time-slot-2").textContent).toContain("since 14:05");
        expect(screen.getByTestId("time-slot-4-today").textContent?.trim()).toBe("15m");
        expect(screen.getByTestId("time-slot-1").textContent).toContain("yesterday");
        expect(screen.getByTestId("time-slot-5").getAttribute("aria-label")).toBe("Slot 5: empty, choose an account to pin");
        expect(screen.getByTestId("time-slot-0").textContent).toContain("Other…");
        expect(screen.getByTestId("time-track-today").textContent).toContain("Thu 26 Nov");
        expect(screen.getByTestId("time-track-today").textContent).toContain("1h 25m");
    });

    it("toggles a slot with its digit, lit at once", async () => {
        const {api} = await mount();
        await press("3");
        expect(screen.getByTestId("time-slot-3").getAttribute("aria-pressed")).toBe("true");
        await vi.waitFor(() => expect(api.toggle).toHaveBeenCalledWith({slot: 3}));
    });

    it("0 picks another account; Create account… declares it and starts it", async () => {
        const {api} = await mount();
        await press("0");
        const picker = await screen.findByTestId("time-picker");
        const input = within(picker).getByTestId("time-picker-input");
        await vi.waitFor(() => expect(within(picker).getAllByTestId("time-picker-option").length).toBeGreaterThan(0));
        await fireEvent.input(input, {target: {value: "clients:initech:kickoff"}});
        await fireEvent.click(within(picker).getByTestId("time-picker-create"));
        await vi.waitFor(() => expect(api.toggle).toHaveBeenCalledWith({account: "clients:initech:kickoff"}));
        expect(api.declare).toHaveBeenCalledWith("clients:initech:kickoff");
        expect(screen.queryByTestId("time-picker")).toBeNull();
    });

    it("chooses an existing account in the picker by keys", async () => {
        const {api} = await mount();
        await press("0");
        const input = await screen.findByTestId("time-picker-input");
        await vi.waitFor(() => expect(screen.getAllByTestId("time-picker-option").length).toBeGreaterThan(0));
        await fireEvent.input(input, {target: {value: "sales"}});
        await fireEvent.keyDown(input, {key: "Enter"});
        await vi.waitFor(() => expect(api.toggle).toHaveBeenCalledWith({account: "internal:sales"}));
    });

    it("+ and - move the selected slot by the step; ↑/↓ select", async () => {
        const {api} = await mount();
        await press("ArrowDown");
        await press("+");
        await vi.waitFor(() => expect(api.adjust).toHaveBeenLastCalledWith({account: "clients:acme:advisory"}, 5));
        await press("ArrowDown");
        await press("-");
        await vi.waitFor(() => expect(api.adjust).toHaveBeenLastCalledWith({account: "clients:globex:audit"}, -5));
        await press("ArrowUp");
        await press("=");
        await vi.waitFor(() => expect(api.adjust).toHaveBeenLastCalledWith({account: "clients:acme:advisory"}, 5));
    });

    it("Shift+- snaps and . stops the running timer", async () => {
        const {api} = await mount(running(trackState(), 2, 600));
        await press("_");
        await vi.waitFor(() => expect(api.snap).toHaveBeenCalled());
        await press(".");
        await vi.waitFor(() => expect(api.stop).toHaveBeenCalledWith({}));
    });

    it("n writes today's note on the selected slot", async () => {
        const {api} = await mount();
        await press("2");
        await press("n");
        const input = (await screen.findByTestId("time-note-input")) as HTMLInputElement;
        expect(input.value).toBe("call with Fred about the K-1");
        await fireEvent.input(input, {target: {value: "call about the K-1"}});
        await fireEvent.keyDown(input, {key: "Enter"});
        await vi.waitFor(() => expect(api.note).toHaveBeenCalledWith({account: "clients:acme:advisory"}, "call about the K-1"));
        await vi.waitFor(() => expect(screen.queryByTestId("time-note")).toBeNull());
    });

    it("e changes the selected pin, saved in its place", async () => {
        const {api} = await mount();
        await press("ArrowDown");
        await press("ArrowDown");
        await press("e");
        const picker = await screen.findByTestId("time-picker");
        expect(picker.textContent).toContain("Pin to slot 3");
        await vi.waitFor(() => expect(within(picker).getAllByTestId("time-picker-option").length).toBeGreaterThan(0));
        await fireEvent.input(within(picker).getByTestId("time-picker-input"), {target: {value: "internal:sales"}});
        await fireEvent.keyDown(within(picker).getByTestId("time-picker-input"), {key: "Enter"});
        await vi.waitFor(() =>
            expect(api.savePrefs).toHaveBeenCalledWith({pinned: ["clients:acme:tax-return", "clients:acme:advisory", "internal:sales", "internal:admin"]})
        );
    });

    it("an empty pin's digit asks what to pin", async () => {
        await mount();
        await press("5");
        expect((await screen.findByTestId("time-picker")).textContent).toContain("Pin to slot 5");
    });

    it("c shows today's card, per account with notes, with Copy", async () => {
        await mount();
        await press("c");
        const card = await screen.findByTestId("time-card");
        await vi.waitFor(() => expect(within(card).getAllByTestId("time-card-row")).toHaveLength(2));
        expect(card.textContent).toContain("call with Fred about the K-1");
        expect(within(card).getByTestId("time-card-total").textContent).toBe("1h 15m");
        const writeText = vi.fn(async () => undefined);
        Object.defineProperty(navigator, "clipboard", {value: {writeText}, configurable: true});
        await fireEvent.click(within(card).getByTestId("time-card-copy"));
        await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith(expect.stringContaining("Acme Corp · advisory")));
        await press("c");
        expect(screen.queryByTestId("time-card")).toBeNull();
    });

    it("opens the picker at once for ?pick=0", async () => {
        await mount(trackState(), 0);
        expect((await screen.findByTestId("time-picker")).textContent).toContain("Track another account");
    });

    it("lists its keys under Track in the help sheet, slot keys once", async () => {
        await mount(running(trackState(), 2, 60));
        const track = keymap.help.find((section) => section.group === "Track");
        const keys = track?.rows.map((r) => r.keys) ?? [];
        expect(keys).toEqual(["1", "ArrowDown", "ArrowUp", "+", "-", "_", "n", ".", "e", "c"]);
    });
});
