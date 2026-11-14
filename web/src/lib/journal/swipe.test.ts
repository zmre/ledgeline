import {describe, expect, it} from "vitest";
import {crossedCommit, decideAxis, dragOffset, restingOffset, scaledSwipe, settle, SWIPE_DEFAULTS, type SwipeConfig} from "./swipe";

const C: SwipeConfig = {slop: 10, axisRatio: 1.2, actionsWidth: 144, openAt: 0.4, commitAt: 96, flick: 0.6};

describe("decideAxis", () => {
    it("waits inside the slop, so a tap never moves the card", () => {
        expect(decideAxis(0, 0, C)).toBe("undecided");
        expect(decideAxis(6, 6, C)).toBe("undecided");
        expect(decideAxis(-9, 0, C)).toBe("undecided");
    });

    it("calls a mostly vertical drag vertical, leaving it to the page's scroll", () => {
        expect(decideAxis(3, 40, C)).toBe("vertical");
        expect(decideAxis(-12, -14, C)).toBe("vertical");
        // A diagonal is not clearly sideways: the scroll wins the tie.
        expect(decideAxis(20, 20, C)).toBe("vertical");
    });

    it("calls a clearly sideways drag horizontal, either way", () => {
        expect(decideAxis(30, 5, C)).toBe("horizontal");
        expect(decideAxis(-30, 10, C)).toBe("horizontal");
    });
});

describe("dragOffset", () => {
    it("follows the finger between the limits", () => {
        expect(dragOffset(0, -50, C)).toBe(-50);
        expect(dragOffset(0, 40, C)).toBe(40);
        expect(dragOffset(-144, 30, C)).toBe(-114);
    });

    it("rubber-bands past the actions' width and past the commit line", () => {
        expect(dragOffset(0, -144 - 30, C)).toBe(-154);
        expect(dragOffset(0, 96 + 30, C)).toBe(106);
    });
});

describe("settle", () => {
    it("springs back from a short drag either way", () => {
        expect(settle(-40, 0, C)).toBe("close");
        expect(settle(50, 0, C)).toBe("close");
    });

    it("opens the trailing actions past openAt of their width", () => {
        expect(settle(-0.4 * 144, 0, C)).toBe("open");
        expect(settle(-100, 0, C)).toBe("open");
    });

    it("runs the leading action at or past the commit line, whatever the speed", () => {
        expect(settle(96, 0, C)).toBe("commit");
        expect(settle(120, -2, C)).toBe("commit");
    });

    it("lets a flick decide by direction", () => {
        expect(settle(-20, -0.8, C)).toBe("open");
        expect(settle(-120, 0.8, C)).toBe("close");
    });

    it("rests where the end says: open at the actions' width, else closed", () => {
        expect(restingOffset("open", C)).toBe(-144);
        expect(restingOffset("close", C)).toBe(0);
        expect(restingOffset("commit", C)).toBe(0);
    });
});

describe("crossedCommit", () => {
    it("ticks once on the way over the line, and once on the way back", () => {
        expect(crossedCommit(90, 100, C)).toBe(true);
        expect(crossedCommit(100, 110, C)).toBe(false);
        expect(crossedCommit(100, 80, C)).toBe(true);
        expect(crossedCommit(-50, -60, C)).toBe(false);
    });
});

describe("scaledSwipe", () => {
    it("scales the distances with the interface, not the speeds or ratios", () => {
        const big = scaledSwipe(2, C);
        expect(big).toEqual({...C, slop: 20, actionsWidth: 288, commitAt: 192});
    });

    it("treats a nonsense factor as 100 %", () => {
        expect(scaledSwipe(Number.NaN)).toEqual(SWIPE_DEFAULTS);
        expect(scaledSwipe(0)).toEqual(SWIPE_DEFAULTS);
    });
});
