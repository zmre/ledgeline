import {describe, expect, it} from "vitest";
import {FLOW_MARK_SHARE, flowLine, flowMarkers} from "./flowMarkers";

describe("UNIT holdings flowMarkers — net bought/sold markers (plan 47 A1e)", () => {
    // A 100k span: the bar is 5 % of it, $5,000.
    const DOMAIN: [number, number] = [100_000, 200_000];
    const VALUES = [150_000, 160_000, 110_000, 115_000, 120_000];

    it("marks a bucket whose net flow reaches 5 % of the y-axis span, and no other", () => {
        expect(FLOW_MARK_SHARE).toBe(0.05);
        const markers = flowMarkers([0, 12_000, -48_200, 4_999, 5_000], VALUES, DOMAIN);
        expect(markers).toEqual([
            {index: 1, direction: "in", flow: 12_000, value: 160_000},
            {index: 2, direction: "out", flow: -48_200, value: 110_000},
            {index: 4, direction: "in", flow: 5_000, value: 120_000},
        ]);
    });

    it("leaves a monthly contribution on a big portfolio unmarked", () => {
        expect(flowMarkers([0, 1_000, 1_000, 1_000], [1_000_000, 1_010_000, 1_020_000, 1_030_000], [800_000, 1_100_000])).toEqual([]);
    });

    it("skips a bucket with no value to sit on, and a zero or non-finite flow", () => {
        expect(flowMarkers([0, 50_000, Number.NaN, 0], [150_000, null, 160_000, 170_000], DOMAIN)).toEqual([]);
    });

    it("measures against the values' extent with zero when the chart computes its own domain", () => {
        // Span 0..100k → a $5,000 bar.
        const markers = flowMarkers([0, 6_000, 4_000], [90_000, 100_000, 96_000], null);
        expect(markers.map((m) => m.index)).toEqual([1]);
    });

    it("marks nothing on an empty or zero-span axis", () => {
        expect(flowMarkers([], [], null)).toEqual([]);
        expect(flowMarkers([0, 10], [0, 0], [0, 0])).toEqual([]);
    });

    it("words the tooltip line with the page's money formatter", () => {
        const money = (n: number): string => `$${n.toLocaleString("en-US")}`;
        expect(flowLine({flow: 12_000}, money)).toBe("Net bought $12,000");
        expect(flowLine({flow: -48_200}, money)).toBe("Net sold $48,200");
    });
});
