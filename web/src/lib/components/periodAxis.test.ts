import {afterEach, describe, expect, it, vi} from "vitest";
import {fittedTicks, labelFormatter, tickIndices, type AxisFit} from "./periodAxis";
import {textMeasurer} from "./textWidth";

// The measuring seam: jsdom has no canvas, so the real `textMeasurer` answers
// null. Stubbed to a deterministic measure; a test that wants the unmeasurable
// path makes it answer null again.
vi.mock("./textWidth", () => ({textMeasurer: vi.fn()}));

/** Twelve monthly labels, "Oct 2025" … "Sep 2026". */
const MONTHS = ["Oct 2025", "Nov 2025", "Dec 2025", "Jan 2026", "Feb 2026", "Mar 2026", "Apr 2026", "May 2026", "Jun 2026", "Jul 2026", "Aug 2026", "Sep 2026"];

/** A deterministic measure: 5.5px per character (≈ a 10px UI font). */
const measure = (text: string): number => text.length * 5.5;

vi.mocked(textMeasurer).mockReturnValue(measure);
afterEach(() => vi.mocked(textMeasurer).mockReturnValue(measure));

const fit = (plotWidth: number, labels: readonly string[] = MONTHS): AxisFit => ({plotWidth, labels});

/** No two neighbouring tick labels overlap, given `bucketPx` per bucket. */
function clear(ticks: readonly number[], labels: readonly string[], plotWidth: number): boolean {
    const bucketPx = plotWidth / labels.length;
    return ticks.slice(1).every((t, k) => {
        const prev = ticks[k];
        const needed = (measure(labels[prev]) + measure(labels[t])) / 2;
        return (t - prev) * bucketPx >= needed;
    });
}

describe("UNIT periodAxis tickIndices", () => {
    it("without a fit, aims for about six ticks and always ends on the last bucket", () => {
        expect(tickIndices(12)).toEqual([0, 2, 4, 6, 8, 10, 11]);
        expect(tickIndices(6)).toEqual([0, 1, 2, 3, 4, 5]);
        expect(tickIndices(1)).toEqual([0]);
        expect(tickIndices(0)).toEqual([]);
    });

    it("keeps the fixed target when the chart has not been laid out yet", () => {
        expect(tickIndices(12, fit(0))).toEqual(tickIndices(12));
    });

    it("keeps the fixed target when the labels fit easily", () => {
        expect(tickIndices(6, fit(900))).toEqual([0, 1, 2, 3, 4, 5]);
    });

    // The reported collision: twelve "Mon YYYY" labels in a ~390px phone chart
    // (plot ≈ 390 − 56 − 8 = 326px) crowded as "Jun 2026Jul 2026".
    it("thins month labels at phone width so none collide, and still ends on the last month", () => {
        const plot = 326;
        const ticks = tickIndices(12, fit(plot));
        expect(ticks.length).toBeLessThan(tickIndices(12).length);
        expect(ticks[ticks.length - 1]).toBe(11);
        expect(clear(ticks, MONTHS, plot)).toBe(true);
    });

    it("drops the stride tick that would crowd the forced last one", () => {
        // 20px per bucket and a 56px label slot (44px + gap) need a stride of 3:
        // 0, 3, 6, 9. Then 9 → 11 is only 40px, so 9 gives way to the last month.
        const ticks = tickIndices(12, fit(240));
        expect(ticks).toEqual([0, 3, 6, 11]);
        expect(clear(ticks, MONTHS, 240)).toBe(true);
    });

    it("stays clear across a range of widths and lengths", () => {
        for (const count of [3, 7, 12, 13, 24, 61]) {
            const labels = Array.from({length: count}, (_, i) => MONTHS[i % 12]);
            for (const plot of [120, 200, 326, 480, 800]) {
                const ticks = tickIndices(count, fit(plot, labels));
                expect(ticks[ticks.length - 1], `${count} @ ${plot}`).toBe(count - 1);
                if (ticks.length > 1) expect(clear(ticks, labels, plot), `${count} @ ${plot}: ${ticks.join(",")}`).toBe(true);
            }
        }
    });

    it("estimates label widths when the text cannot be measured", () => {
        vi.mocked(textMeasurer).mockReturnValue(null);
        const estimated = tickIndices(12, fit(326));
        expect(estimated.length).toBeLessThan(tickIndices(12).length);
        expect(estimated[estimated.length - 1]).toBe(11);
    });

    it("gives short labels more ticks than long ones in the same width", () => {
        const short = MONTHS.map((m) => m.slice(0, 3));
        expect(tickIndices(12, fit(326, short)).length).toBeGreaterThan(tickIndices(12, fit(326)).length);
    });
});

describe("UNIT periodAxis fittedTicks", () => {
    const scale = (start: number, end: number) => ({range: () => [start, end]});

    it("fits the labels to the plot width the axis scale was sized to", () => {
        // The ~390px phone chart again, as layerchart hands it over: a scale
        // whose range is the plot area (390 − 56 − 8).
        expect(fittedTicks(MONTHS)(scale(0, 326))).toEqual(tickIndices(12, fit(326)));
        expect(fittedTicks(MONTHS)(scale(0, 326)).length).toBeLessThan(tickIndices(12).length);
    });

    it("keeps the fixed target before layout, when the padding exceeds the box", () => {
        expect(fittedTicks(MONTHS)(scale(0, -64))).toEqual(tickIndices(12));
    });
});

describe("UNIT periodAxis labelFormatter", () => {
    it("rounds an index to its label and prints nothing out of range", () => {
        const labelOf = labelFormatter(["a", "b"]);
        expect(labelOf(1)).toBe("b");
        expect(labelOf(0.6)).toBe("b");
        expect(labelOf(5)).toBe("");
    });
});
