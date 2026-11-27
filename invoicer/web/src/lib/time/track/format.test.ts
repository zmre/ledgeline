// Track's pure helpers: a running timer as people read it, the platform key
// hints, the day card, and the optimistic toggle.
import {describe, expect, it} from "vitest";
import {
    accountLabel,
    addMinutes,
    dayCardRows,
    dayCardText,
    dayHeading,
    elapsedText,
    optimisticStop,
    optimisticToggle,
    slotAria,
    slotHint,
    slotName,
    slotSeconds,
    stopHint,
} from "./format.js";
import {running, trackState} from "./fixtures.js";

describe("UNIT time durations", () => {
    it("counts a running timer in seconds until it reaches an hour", () => {
        expect(elapsedText(42.7)).toBe("42s");
        expect(elapsedText(12 * 60 + 5)).toBe("12m 05s");
        expect(elapsedText(3600 + 22 * 60 + 9)).toBe("1h 22m");
    });

    it("names an account without a label by its parts", () => {
        expect(accountLabel("clients:acme:tax")).toBe("clients · acme · tax");
    });

    it("does date and time arithmetic by calendar, never by parsing text", () => {
        expect(dayHeading("2026-11-26")).toBe("Thu 26 Nov");
        expect(addMinutes("2026-11-25T22:30:00", 240)).toBe("2026-11-26T02:30");
    });
});

describe("UNIT time key hints (K)", () => {
    it("says ⌥⌘N on macOS and Ctrl+Shift+N elsewhere", () => {
        expect(slotHint(2, true)).toBe("⌥⌘2");
        expect(slotHint(0, false)).toBe("Ctrl+Shift+0");
        expect(stopHint(true)).toBe("⌥⌘.");
        expect(stopHint(false)).toBe("Ctrl+Shift+.");
    });
});

describe("UNIT the day card", () => {
    const state = trackState();
    const lines = [
        {account: "clients:acme:advisory", seconds: 3600, note: "call with Fred"},
        {account: "clients:acme:advisory", seconds: 600, note: "email"},
        {account: "internal:admin", seconds: 900, note: ""},
        {account: "internal:training", seconds: 0, note: ""},
    ];

    it("totals each account with its notes, the running timer added, labels from the slots", () => {
        const rows = dayCardRows(lines, state.slots, {account: "internal:sales", label: "Internal · sales", seconds: 1200});
        expect(rows).toEqual([
            {account: "clients:acme:advisory", label: "Acme Corp · advisory", seconds: 4200, notes: ["call with Fred", "email"]},
            {account: "internal:admin", label: "Internal · admin", seconds: 900, notes: []},
            {account: "internal:sales", label: "Internal · sales", seconds: 1200, notes: []},
        ]);
    });

    it("copies as aligned text, headed by the day, the person and the total", () => {
        const text = dayCardText("2026-11-26", dayCardRows(lines, state.slots, null), "pat");
        expect(text.split("\n")).toEqual([
            "Thu 26 Nov · pat · 1h 25m",
            "",
            "Acme Corp · advisory   1h 10m  call with Fred; email",
            "Internal · admin          15m",
        ]);
    });
});

describe("UNIT the optimistic toggle", () => {
    const at = new Date(2026, 10, 26, 15, 0, 0).getTime();

    it("starts a stopped slot now", () => {
        const next = optimisticToggle(trackState(), {slot: 3}, 0, at);
        expect(next?.running).toMatchObject({account: "clients:globex:audit", slot: 3, since: "2026-11-26T15:00:00"});
        expect(next?.slots.find((s) => s.key === 3)?.running).toBe(true);
    });

    it("switching banks the running one's rounded minutes into its slot", () => {
        const before = running(trackState(), 2, 600);
        const next = optimisticToggle(before, {slot: 4}, 610, at);
        expect(next?.running?.slot).toBe(4);
        const two = next?.slots.find((s) => s.key === 2);
        expect(two).toMatchObject({running: false});
        expect(two?.today.seconds).toBe(3600 + 600);
        expect(next?.banked_today.seconds).toBe(4500 + 600);
        expect(next?.last).toMatchObject({account: "clients:acme:advisory", slot: 2});
    });

    it("pressing the running slot stops it", () => {
        const next = optimisticToggle(running(trackState(), 2, 0), {slot: 2}, 125, at);
        expect(next?.running).toBeNull();
        expect(next?.slots.find((s) => s.key === 2)?.today.seconds).toBe(3600 + 120);
    });

    it("an account by name starts with its label, slotless", () => {
        const next = optimisticToggle(trackState(), {account: "internal:sales", label: "Internal · sales"}, 0, at);
        expect(next?.running).toMatchObject({account: "internal:sales", label: "Internal · sales", slot: null});
    });

    it("predicts nothing for an empty slot", () => {
        expect(optimisticToggle(trackState(), {slot: 7}, 0, at)).toBeNull();
    });

    it("a stop banks the time; a discard banks none", () => {
        const before = running(trackState(), 2, 0);
        expect(optimisticStop(before, 300).banked_today.seconds).toBe(4800);
        expect(optimisticStop(before, 300, true).banked_today.seconds).toBe(4500);
        expect(optimisticStop(before, 300, true).running).toBeNull();
    });
});

describe("UNIT time slot names", () => {
    it("names a slot, or what an empty one is for, and reads it out with today's total", () => {
        const state = running(trackState(), 2, 600);
        const [one, two, , four, five, , seven] = state.slots;
        const other = state.slots[9];
        expect([slotName(two), slotName(five), slotName(seven), slotName(other)]).toEqual(["Acme Corp · advisory", "Empty pin", "·", "Other…"]);
        expect(slotSeconds(two, 600)).toBe(4200);
        expect(slotSeconds(four, 600)).toBe(900);
        expect(slotAria(two, slotSeconds(two, 600))).toBe("Slot 2: Acme Corp · advisory, running, 1h 10m today");
        expect(slotAria(one, 0)).toBe("Slot 1: Acme Corp · tax return, stopped, nothing today");
        expect(slotAria(five, 0)).toBe("Slot 5: empty, choose an account to pin");
        expect(slotAria(other, 0)).toBe("Slot 0: choose an account");
    });
});
