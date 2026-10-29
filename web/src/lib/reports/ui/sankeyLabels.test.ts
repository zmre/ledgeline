// The Sankey label layout.
//
// The claims `SankeyPanel` draws on, each stated twice: once on a column small
// enough to check by hand, and once over a few thousand random columns, because
// the failures this module exists to prevent (two labels on one line, a label
// off the chart, two leader lines crossing) show up in the shapes nobody thought
// to write down.
//
// The random columns come from a fixed seed, so a failure reproduces exactly;
// the seed is in every failure message.

import {describe, expect, it} from "vitest";
import {DISPLACED_PX, layoutLabels, placeLabels, type LabelItem, type LabelOptions, type PlacedLabel} from "./sankeyLabels";

const OPTS: LabelOptions = {top: 0, bottom: 300, twoLine: 26, oneLine: 14};

const items = (anchors: number[], sizes: number[] = anchors.map(() => 1)): LabelItem[] => anchors.map((anchor, i) => ({key: `n${i}`, anchor, size: sizes[i]}));

const height = (label: PlacedLabel, opts: LabelOptions): number => (label.twoLine ? opts.twoLine : opts.oneLine);
const gap = (a: PlacedLabel, b: PlacedLabel, opts: LabelOptions): number => (height(a, opts) + height(b, opts)) / 2;

/** Every invariant the layout promises, for one column. Returns the first broken one, or null. */
function broken(input: readonly LabelItem[], out: readonly PlacedLabel[], opts: LabelOptions): string | null {
    if (out.length !== input.length) return "lost or invented a label";
    for (let i = 0; i < out.length; i++) {
        if (out[i].key !== input[i].key) return `label ${i} reordered`;
        if (out[i].displaced !== Math.abs(out[i].y - input[i].anchor) > DISPLACED_PX) return `label ${i} misreports displaced`;
        if (i > 0 && out[i].y - out[i - 1].y < gap(out[i - 1], out[i], opts) - 1e-9) return `labels ${i - 1} and ${i} overlap`;
    }
    if (input.length * opts.oneLine <= opts.bottom - opts.top) {
        for (const [i, label] of out.entries()) {
            const half = height(label, opts) / 2;
            if (label.y - half < opts.top - 1e-9 || label.y + half > opts.bottom + 1e-9) return `label ${i} outside [top, bottom]`;
        }
    }
    return null;
}

describe("UNIT layoutLabels", () => {
    it("leaves a column alone when nothing overlaps", () => {
        const input = items([20, 60, 120, 250]);
        const out = layoutLabels(input, OPTS);

        expect(out.map((label) => label.y)).toEqual([20, 60, 120, 250]);
        expect(out.every((label) => label.twoLine && !label.displaced)).toBe(true);
    });

    it("pushes a crowded run apart, in order, and flags only what moved", () => {
        // Three bars 4px apart: the first keeps its place, the next two are
        // pushed one two-line box apart, and the far one is untouched.
        const out = layoutLabels(items([100, 104, 108, 250]), OPTS);

        expect(out.map((label) => label.y)).toEqual([100, 126, 152, 250]);
        expect(out.map((label) => label.displaced)).toEqual([false, true, true, false]);
    });

    it("pulls a run back up off the bottom edge rather than spilling it", () => {
        const out = layoutLabels(items([280, 285, 290]), OPTS);

        // The last box ends exactly at the bottom; the others stack above it.
        expect(out.map((label) => label.y)).toEqual([235, 261, 287]);
        expect(broken(items([280, 285, 290]), out, OPTS)).toBeNull();
    });

    it("pushes a label at the very top down inside the chart", () => {
        const out = layoutLabels(items([2]), OPTS);
        expect(out[0].y).toBe(13);
        expect(out[0].displaced).toBe(true);
    });

    it("drops the amount from the SMALLEST bars first, and only as many as it must", () => {
        // 12 two-line labels need 312px of the 300; one demotion saves 12px.
        const sizes = [90, 80, 70, 60, 50, 40, 5, 30, 20, 10, 8, 6];
        const out = layoutLabels(
            items(
                sizes.map((_, i) => i * 25),
                sizes
            ),
            OPTS
        );

        expect(out.filter((label) => !label.twoLine).map((label) => label.key)).toEqual(["n6"]);
        expect(broken(items(sizes.map((_, i) => i * 25)), out, OPTS)).toBeNull();
    });

    it("breaks a size tie by demoting the LOWER label", () => {
        const out = layoutLabels(items([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), OPTS);
        expect(out.filter((label) => !label.twoLine).map((label) => label.key)).toEqual(["n11"]);
    });

    it("demotes every label, and then spills off the bottom, when even one line each cannot fit", () => {
        const input = items(Array.from({length: 30}, () => 150));
        const out = layoutLabels(input, OPTS);

        expect(out.every((label) => !label.twoLine)).toBe(true);
        // The head stays on the chart; only the tail runs off.
        expect(out[0].y).toBe(OPTS.oneLine / 2);
        expect(broken(input, out, OPTS)).toBeNull();
    });

    it("places an empty column as nothing", () => {
        expect(layoutLabels([], OPTS)).toEqual([]);
    });
});

describe("UNIT placeLabels", () => {
    it("lays each column out on its own, in bar order, and steps out only the crowded one", () => {
        // Listed out of order on purpose: the Sankey's array order is not its stacking order.
        const spots = placeLabels(
            [
                {key: "b", side: "source", value: 1, y0: 102, y1: 106},
                {key: "a", side: "source", value: 9, y0: 0, y1: 200},
                {key: "x", side: "target", value: 5, y0: 0, y1: 100},
                {key: "y", side: "target", value: 5, y0: 100, y1: 200},
            ],
            OPTS
        );

        // "b" sits on "a"'s middle, so it is pushed below it; the targets are roomy.
        expect(spots.a).toMatchObject({y: 100, displaced: false, leaders: true});
        expect(spots.b).toMatchObject({y: 126, displaced: true, leaders: true});
        expect(spots.x).toMatchObject({y: 50, displaced: false, leaders: false});
        expect(spots.y).toMatchObject({y: 150, displaced: false, leaders: false});
    });
});

/** mulberry32: small, fast, and the same sequence on every machine. */
function rng(seed: number): () => number {
    let a = seed >>> 0;
    return () => {
        a = (a + 0x6d2b79f5) >>> 0;
        let t = a;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

describe("UNIT layoutLabels, randomised", () => {
    const SEED = 0x5a4b3c;

    it("keeps every invariant over thousands of random columns", () => {
        const random = rng(SEED);
        for (let run = 0; run < 3000; run++) {
            const n = Math.floor(random() * 60);
            const opts: LabelOptions = {top: -8, bottom: 100 + random() * 900, twoLine: 26, oneLine: 14};
            // Anchors sorted, as a column's bar middles are; sometimes bunched
            // into a narrow band, sometimes spread over the whole height.
            const band = random() < 0.5 ? 40 : opts.bottom;
            const anchors = Array.from({length: n}, () => random() * band).sort((a, b) => a - b);
            const sizes = anchors.map(() => Math.round(random() * 1000));
            const input = items(anchors, sizes);
            const out = layoutLabels(input, opts);

            expect(broken(input, out, opts), `seed ${SEED}, run ${run}`).toBeNull();
            // Demotion is smallest-first: no one-line label's bar outranks a two-line one's.
            const kept = sizes.filter((_, i) => out[i].twoLine);
            const dropped = sizes.filter((_, i) => !out[i].twoLine);
            expect(Math.max(...dropped) <= Math.min(...kept), `seed ${SEED}, run ${run}`).toBe(true);
        }
    });

    it("moves nothing in a column that already fits with room between", () => {
        const random = rng(SEED + 1);
        for (let run = 0; run < 500; run++) {
            const n = 1 + Math.floor(random() * 10);
            // Spaced at least a two-line box apart, starting a half box in.
            let at = OPTS.twoLine / 2;
            const anchors = Array.from({length: n}, () => {
                const here = at;
                at += OPTS.twoLine + random() * 10;
                return here;
            });
            const opts = {...OPTS, bottom: at + OPTS.twoLine};
            const out = layoutLabels(items(anchors), opts);

            expect(
                out.map((label) => label.y),
                `seed ${SEED + 1}, run ${run}`
            ).toEqual(anchors);
            expect(out.some((label) => label.displaced || !label.twoLine)).toBe(false);
        }
    });
});
