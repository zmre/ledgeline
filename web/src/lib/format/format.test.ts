// The shared display primitives (DRY-6). Each of these replaced two or more
// drifted copies, so the tests pin the ONE behaviour that won.

import {describe, expect, it} from "vitest";
import {dec} from "$lib/domain/money";
import {absDec, DEFAULT_AMOUNT_STYLE, EM_DASH, fmt, fmtSignedAmount, fmtSignedPct, ZERO} from "./amounts";
import {CATEGORICAL, colorAt, OTHER_COLOR, OTHER_LABEL} from "./palette";
import {NEUTRAL_CLASS, sentimentClass, signClass} from "./sign";

describe("UNIT signClass", () => {
    it("is green above zero and red below it", () => {
        expect(signClass(1)).toBe("text-success");
        expect(signClass(-1)).toBe("text-error");
        expect(signClass(1n)).toBe("text-success");
        expect(signClass(-1n)).toBe("text-error");
    });

    it("is NEUTRAL at exactly zero — not green", () => {
        // `HoldingsStats` carried a copy taking an already-computed `negative:
        // boolean`, so a gain of exactly zero fell to the else-branch and was
        // announced in success green.
        expect(signClass(0)).toBe(NEUTRAL_CLASS);
        expect(signClass(0n)).toBe(NEUTRAL_CLASS);
        expect(signClass(0)).not.toBe("text-success");
    });

    it("is neutral for an absent figure", () => {
        expect(signClass(null)).toBe(NEUTRAL_CLASS);
    });
});

describe("UNIT sentimentClass colours by whether the change is welcome", () => {
    it("reads the same delta oppositely for revenue and expenses", () => {
        expect(sentimentClass(50, true)).toBe("text-success"); // revenue up: good
        expect(sentimentClass(50, false)).toBe("text-error"); // expenses up: bad
        expect(sentimentClass(-50, true)).toBe("text-error");
        expect(sentimentClass(-50, false)).toBe("text-success");
    });

    it("is neutral at zero and for an absent figure, whichever way is good", () => {
        expect(sentimentClass(0, true)).toBe(NEUTRAL_CLASS);
        expect(sentimentClass(0, false)).toBe(NEUTRAL_CLASS);
        expect(sentimentClass(null, true)).toBe(NEUTRAL_CLASS);
    });
});

describe("UNIT fmtSignedPct — one rendering of a signed percent", () => {
    it("signs positives with '+' and one decimal", () => {
        expect(fmtSignedPct(21.256)).toBe("+21.3%");
    });

    it("uses the typographic minus U+2212, never an ASCII hyphen", () => {
        // The holdings copy used ASCII, the insights copy U+2212.
        expect(fmtSignedPct(-3.44)).toBe("−3.4%");
        expect(fmtSignedPct(-3.44)).not.toContain("-");
    });

    it("leaves zero unsigned", () => {
        expect(fmtSignedPct(0)).toBe("0.0%");
    });

    it("renders an absent percent as the em-dash", () => {
        expect(fmtSignedPct(null)).toBe(EM_DASH);
    });
});

describe("UNIT the default AmountStyle groups thousands", () => {
    it("renders 1,234.56 rather than 1234.56", () => {
        // Five literal copies existed in two behaviours; the chart module's had
        // `digitGroups: null`, so a commodity with no style charted ungrouped.
        expect(fmt("$", dec(123456n, 2), new Map())).toBe("$1,234.56");
        expect(DEFAULT_AMOUNT_STYLE.digitGroups).not.toBeNull();
    });

    it("still prefers a style the journal actually supplied", () => {
        const styles = new Map([["$", {...DEFAULT_AMOUNT_STYLE, digitGroups: null}]]);
        expect(fmt("$", dec(123456n, 2), styles)).toBe("$1234.56");
    });
});

describe("UNIT fmtSignedAmount", () => {
    it("signs the amount and formats its magnitude", () => {
        expect(fmtSignedAmount(dec(123456n, 2), "$", new Map())).toBe("+$1,234.56");
        expect(fmtSignedAmount(dec(-4000n, 2), "$", new Map())).toBe("−$40.00");
        expect(fmtSignedAmount(null, "$", new Map())).toBe(EM_DASH);
    });
});

describe("UNIT absDec / ZERO", () => {
    it("absDec returns the magnitude and leaves a non-negative input alone", () => {
        expect(absDec(dec(-5n, 2))).toEqual(dec(5n, 2));
        const positive = dec(5n, 2);
        expect(absDec(positive)).toBe(positive); // same reference: no needless allocation
    });

    it("ZERO is an exact zero", () => {
        expect(ZERO.m).toBe(0n);
    });
});

describe("UNIT the categorical palette folds, never cycles", () => {
    it("has exactly 8 distinct slots", () => {
        expect(CATEGORICAL).toHaveLength(8);
        expect(new Set(CATEGORICAL).size).toBe(8);
    });

    it("hands out each slot in fixed order", () => {
        CATEGORICAL.forEach((hex, i) => expect(colorAt(i)).toBe(hex));
    });

    it("folds past the last slot to the muted tail colour instead of reusing slot 1", () => {
        // `ChartWidget` did `PALETTE[slot++ % PALETTE.length]` over a 6-entry
        // copy, so a 7th account was painted slot 1's blue and became
        // indistinguishable from the 1st — the dataviz non-negotiable.
        expect(colorAt(CATEGORICAL.length)).toBe(OTHER_COLOR);
        expect(colorAt(CATEGORICAL.length)).not.toBe(CATEGORICAL[0]);
        expect(colorAt(99)).toBe(OTHER_COLOR);
    });

    it("keeps the tail colour out of the categorical slots", () => {
        expect(CATEGORICAL).not.toContain(OTHER_COLOR);
    });

    it("names the folded tail once", () => {
        expect(OTHER_LABEL).toBe("(other)");
    });
});
