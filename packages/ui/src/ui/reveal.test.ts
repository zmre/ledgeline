// The reveal arithmetic, pure. jsdom reports every rect as zero, so this is the
// only place the rules in `reveal.ts`'s header can be checked; the e2e suite's
// `toBeInViewport` checks them composed.

import {describe, expect, it} from "vitest";
import {createRevealArm, REVEAL_MIN_VISIBLE, revealDelta} from "./reveal";

/** A 600px window under a 64px sticky header: the e2e suite's short viewport. */
const VIEWPORT = {height: 600, headerOffset: 64};

describe("UNIT revealDelta", () => {
    it("leaves an element whose top is already on screen alone", () => {
        expect(revealDelta({top: 200, height: 900}, VIEWPORT)).toBe(0);
        expect(revealDelta({top: VIEWPORT.headerOffset, height: 2000}, VIEWPORT)).toBe(0);
    });

    it("leaves it alone with a heading's worth showing at the bottom edge", () => {
        expect(revealDelta({top: 600 - REVEAL_MIN_VISIBLE, height: 900}, VIEWPORT)).toBe(0);
    });

    it("nudges a top that is on screen by only a sliver", () => {
        // 10px showing is not "appeared" to anyone; bring 40% of a viewport up.
        expect(revealDelta({top: 590, height: 900}, VIEWPORT)).toBe(230);
    });

    it("brings a tall element below the fold up by 40% of the viewport past its top", () => {
        // top 700 → wants 700 + 240 visible in a 600px window.
        expect(revealDelta({top: 700, height: 2000}, VIEWPORT)).toBe(340);
    });

    it("brings all of a short element up, and no more", () => {
        // 100px high, so its bottom (800) lands on the bottom edge.
        expect(revealDelta({top: 700, height: 100}, VIEWPORT)).toBe(200);
    });

    it("never takes the top above the sticky header", () => {
        // Far below, the header is no constraint: 2000 + 240 - 600.
        expect(revealDelta({top: 2000, height: 5000}, VIEWPORT)).toBe(1640);
        // A share that would carry the top past the header stops it there.
        expect(revealDelta({top: 700, height: 5000}, VIEWPORT, 2)).toBe(700 - VIEWPORT.headerOffset);
    });

    it("never scrolls up — not for an element above the viewport, nor one under the header", () => {
        expect(revealDelta({top: -400, height: 300}, VIEWPORT)).toBe(0);
        expect(revealDelta({top: 20, height: 300}, VIEWPORT)).toBe(0);
    });

    it("works with no sticky header at all", () => {
        expect(revealDelta({top: 650, height: 1000}, {height: 600, headerOffset: 0})).toBe(290);
    });

    it("treats a zero-height element as one whose top is all there is", () => {
        expect(revealDelta({top: 300, height: 0}, VIEWPORT)).toBe(0);
        expect(revealDelta({top: 650, height: 0}, VIEWPORT)).toBe(50);
    });

    it("returns whole pixels", () => {
        expect(Number.isInteger(revealDelta({top: 700.4, height: 333.3}, {height: 601, headerOffset: 64.5}))).toBe(true);
    });
});

describe("UNIT createRevealArm", () => {
    it("starts disarmed and stays armed once armed", () => {
        const arm = createRevealArm();
        expect(arm.armed).toBe(false);
        arm.arm();
        arm.arm();
        expect(arm.armed).toBe(true);
    });

    it("arms each flow on its own", () => {
        const first = createRevealArm();
        const second = createRevealArm();
        first.arm();
        expect(second.armed).toBe(false);
    });
});
