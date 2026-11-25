import {describe, expect, it, vi} from "vitest";
import {nextColumn, sortKeys} from "./sortKeys.js";

describe("UNIT sortKeys", () => {
    it("steps to the next column in header order, wrapping", () => {
        expect(nextColumn(["number", "customer", "date"], "number")).toBe("customer");
        expect(nextColumn(["number", "customer", "date"], "date")).toBe("number");
        // An unknown current (the default order's) starts at the first.
        expect(nextColumn(["number", "customer"], "nope")).toBe("number");
    });

    it("sorts by the next column with s and the same one (a flip) with S, as a header click does", () => {
        const onSort = vi.fn();
        const [s, S] = sortKeys({group: "Invoices", columns: ["number", "date"], shown: () => ({sort: "number", dir: "desc"}), onSort: () => onSort});
        s.run();
        S.run();
        expect(onSort.mock.calls).toEqual([["date"], ["number"]]);
    });

    it("is off without onSort", () => {
        const [s] = sortKeys({group: "Invoices", columns: ["number"], shown: () => ({sort: "number", dir: "asc"}), onSort: () => undefined});
        expect(s.enabled?.()).toBe(false);
    });
});
