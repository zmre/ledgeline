// The review deck's motion (plan 32 D29), as numbers: every moment's duration,
// its frames at both ends and on the way, the peeks' slots, and the
// reduced-motion swap to a cross-fade.

import {describe, expect, it, vi} from "vitest";
import {
    afterHold,
    backdrop,
    barIn,
    cardIn,
    cardOut,
    dialogIn,
    DURATIONS,
    enterDuration,
    exitDuration,
    finishMotion,
    footerFade,
    layerOut,
    PEEK_DEPTH,
    peekIn,
    peekOut,
    peekStyle,
    poseCss,
    stampPop,
    TOP,
    PEEKS,
    type CardExit,
    type DeckMoment,
} from "./motion";

const node = {} as Element;
/** A frame at linear progress `t` (Svelte's `t`; an outro runs it 1 → 0). */
const css = (config: ReturnType<typeof cardOut>, t: number): string => config.css?.(t, 1 - t) ?? "";
const EXITS: CardExit[] = ["reviewed", "saved", "skip", "back", "ignore"];
const full = (exit: DeckMoment) => ({exit, reduced: false});

describe("UNIT review deck motion", () => {
    it("uses the deck's durations: exits 260–300ms after the stamp, rises 220ms, the open 180ms", () => {
        expect(cardOut(node, full("reviewed")).duration).toBe(90 + 280);
        expect(cardOut(node, full("saved")).duration).toBe(280);
        expect(cardOut(node, full("skip")).duration).toBe(260);
        expect(cardOut(node, full("ignore")).duration).toBe(300);
        expect(cardOut(node, full("back")).duration).toBe(220);
        expect(cardIn(node, full("reviewed")).duration).toBe(90 + 220);
        for (const exit of ["saved", "skip", "ignore"] as const) expect(cardIn(node, full(exit)).duration).toBe(220);
        expect(cardIn(node, full("back")).duration).toBe(280);
        expect(cardIn(node, full("open")).duration).toBe(180);
        expect(stampPop(node, {reduced: false}).duration).toBe(90);
        expect(barIn(node, {reduced: false}).duration).toBe(180);
        expect(layerOut(node, {reduced: false}).duration).toBe(160);
        expect(backdrop(node, {reduced: false}).duration).toBe(180);
        expect(backdrop(node, {reduced: false, closing: true}).duration).toBe(160);
        expect(peekIn(node, {reduced: false}).duration).toBe(220);
        expect(peekOut(node, {reduced: false}).duration).toBe(220);
        expect(footerFade(node, {reduced: false}).duration).toBe(80);
        for (const exit of EXITS) expect(exitDuration(exit)).toBe(cardOut(node, full(exit)).duration);
        for (const exit of EXITS) expect(enterDuration(exit)).toBe(cardIn(node, full(exit)).duration);
    });

    it("eases inside css and never uses a Svelte delay on a card, so finishMotion ends it in one call", () => {
        for (const exit of EXITS) {
            expect(cardOut(node, full(exit)).delay).toBeUndefined();
            expect(cardOut(node, full(exit)).easing).toBeUndefined();
            expect(cardIn(node, full(exit)).delay).toBeUndefined();
        }
    });

    it("animates transforms, opacity and filter only", () => {
        const frames = [
            ...EXITS.flatMap((exit) => [css(cardOut(node, full(exit)), 0.5), css(cardIn(node, full(exit)), 0.5)]),
            css(cardIn(node, full("open")), 0.5),
            css(stampPop(node, {reduced: false}), 0.5),
            css(barIn(node, {reduced: false}), 0.5),
            css(layerOut(node, {reduced: false}), 0.5),
            css(peekIn(node, {reduced: false}), 0.5),
        ];
        for (const frame of frames) {
            const properties = frame
                .split(";")
                .map((part) => part.split(":")[0]?.trim())
                .filter((name) => name !== "" && name !== undefined);
            // `filter` is the peeks' dimming and the skip's grey, composited like opacity.
            expect(properties.every((name) => ["transform", "opacity", "filter"].includes(name as string))).toBe(true);
        }
    });

    it("sends each exit off the viewport in its own direction, from rest at identity", () => {
        // An outro's t = 1 is where it starts, t = 0 where it ends.
        expect(css(cardOut(node, full("reviewed")), 1)).toBe("transform: translate(0vw, 0vh) rotate(0deg)");
        expect(css(cardOut(node, full("reviewed")), 0)).toBe("transform: translate(120vw, -8vh) rotate(12deg)");
        expect(css(cardOut(node, full("saved")), 0)).toBe("transform: translate(120vw, -8vh) rotate(12deg)");
        expect(css(cardOut(node, full("skip")), 0)).toBe("transform: translate(-120vw, 0vh) rotate(-8deg); filter: saturate(0.3)");
        expect(css(cardOut(node, full("ignore")), 0)).toBe("transform: translate(0vw, 105vh) rotate(4deg)");
        // ← : the card on top settles into the peek slot, fading out there.
        expect(css(cardOut(node, full("back")), 0)).toBe(`${poseCss(PEEKS[0]!, PEEKS[0]!, 1)}; opacity: 0`);
    });

    it("holds a Reviewed card still while its stamp pops, then flies (ease-in)", () => {
        const out = cardOut(node, full("reviewed"));
        const hold = 90 / 370;
        // Still at identity until the stamp has popped…
        expect(css(out, 1 - hold * 0.99)).toBe("transform: translate(0vw, 0vh) rotate(0deg)");
        // …then accelerating: a quarter of the flight covers a sixteenth of the distance.
        const quarter = 1 - (hold + (1 - hold) * 0.25);
        expect(css(out, quarter)).toBe("transform: translate(7.5vw, -0.5vh) rotate(0.75deg)");
        expect(afterHold(0.5, 0, 100)).toBe(0.5);
        expect(afterHold(0.1, 20, 100)).toBe(0);
        expect(afterHold(0.6, 20, 100)).toBeCloseTo(0.5, 10);
    });

    it("raises the peek to the top as the old card leaves, and brings the previous card in from the left on ←", () => {
        const peek = PEEKS[0]!;
        expect(css(cardIn(node, full("skip")), 0)).toBe(poseCss(peek, peek, 1));
        expect(css(cardIn(node, full("skip")), 1)).toBe(poseCss(TOP, TOP, 1));
        expect(css(cardIn(node, full("skip")), 0)).toBe("transform: translateY(14px) scale(0.95); filter: brightness(0.6)");
        // After Reviewed it waits out the stamp at the peek pose, so both moves start together.
        expect(css(cardIn(node, full("reviewed")), 0.2)).toBe(poseCss(peek, peek, 1));
        expect(css(cardIn(node, full("back")), 0)).toBe("transform: translate(-120vw, 0vh) rotate(-8deg)");
        expect(css(cardIn(node, full("back")), 1)).toBe("transform: translate(0vw, 0vh) rotate(0deg)");
        expect(css(cardIn(node, full("open")), 0)).toBe("transform: translateY(24px) scale(0.97); opacity: 0");
        expect(css(cardIn(node, full("open")), 1)).toBe("transform: translateY(0px) scale(1); opacity: 1");
    });

    it("lays the waiting cards out as a pile: 0.95 / 14px, then 0.9 / 28px, dimmer each time", () => {
        expect(PEEK_DEPTH).toBe(2);
        expect(peekStyle(1, false)).toMatch(/^transform: translateY\(14px\) scale\(0\.95\); filter: brightness\(0\.6\); transition: transform 220ms/);
        expect(peekStyle(2, false)).toMatch(/^transform: translateY\(28px\) scale\(0\.9\); filter: brightness\(0\.38\)/);
        // Reduced motion: still a pile, but no travel between slots.
        expect(peekStyle(1, true)).toBe("transform: translateY(14px) scale(0.95); filter: brightness(0.6)");
        expect(peekStyle(9, true)).toBe(peekStyle(2, true));
        // A beat late on opening, at once afterwards.
        expect(peekIn(node, {reduced: false, opening: true}).delay).toBe(120);
        expect(peekIn(node, {reduced: false}).delay).toBe(0);
    });

    it("does nothing of a card's own while the layer closes; the layer fades and drops", () => {
        expect(cardOut(node, full("close")).duration).toBe(0);
        expect(cardIn(node, full("close")).duration).toBe(0);
        expect(css(layerOut(node, {reduced: false}), 0)).toBe("transform: translateY(16px); opacity: 0");
        expect(css(barIn(node, {reduced: false}), 0)).toBe("transform: translateY(-8px); opacity: 0");
    });

    it("cross-fades with no travel under reduced motion", () => {
        const configs = [
            ...EXITS.flatMap((exit) => [cardOut(node, {exit, reduced: true}), cardIn(node, {exit, reduced: true})]),
            cardIn(node, {exit: "open", reduced: true}),
            barIn(node, {reduced: true}),
            layerOut(node, {reduced: true}),
            stampPop(node, {reduced: true}),
            peekOut(node, {reduced: true}),
        ];
        for (const config of configs) {
            expect(config.duration).toBe(DURATIONS.reduced);
            expect(css(config, 0.5)).toBe("opacity: 0.5");
        }
        // A change made without a keystroke (a revalidation, a failed save's rewind) is a fade too.
        expect(css(cardOut(node, full("open")), 0.5)).toBe("opacity: 0.5");
    });

    it("finishes every running animation under a root at once", () => {
        const finish = vi.fn();
        const stuck = vi.fn(() => {
            throw new Error("infinite");
        });
        const root = {getAnimations: () => [{finish}, {finish: stuck}, {finish}]} as unknown as Element;
        finishMotion(root);
        expect(finish).toHaveBeenCalledTimes(2);
        expect(() => finishMotion(null)).not.toThrow();
    });
});

describe("UNIT review motion — single mode (D31)", () => {
    it("opens like a dialog: scale and fade over 140ms, with no travel", () => {
        const config = dialogIn(node, {reduced: false});
        expect(config.duration).toBe(DURATIONS.dialog);
        expect(DURATIONS.dialog).toBe(140);
        expect(config.css?.(0, 1)).toBe("transform: scale(0.96); opacity: 0");
        expect(config.css?.(1, 0)).toBe("transform: scale(1); opacity: 1");
        expect(config.css?.(0.5, 0.5)).not.toContain("translate");
    });

    it("is the usual cross-fade under reduced motion", () => {
        const config = dialogIn(node, {reduced: true});
        expect(config.duration).toBe(DURATIONS.reduced);
        expect(config.css?.(0.5, 0.5)).toBe("opacity: 0.5");
    });
});
