import {describe, expect, it} from "vitest";
import {depthLimit, groupStartsOpen, newOpenSeed, seedOpenGroups} from "./confDepth";

const spec = (all: number | null, perRegex: [string, number][] = []) => ({all, perRegex: perRegex.map(([regex, depth]) => ({regex, depth}))});

describe("UNIT confDepth", () => {
    it("matches hledger's per-account resolution (verified against hledger 1.52)", () => {
        const fund = "assets:inv:broker:fund";
        // The rightmost match wins, in either declaration order.
        expect(
            depthLimit(
                spec(null, [
                    ["inv", 4],
                    ["broker", 3],
                ]),
                fund
            )
        ).toBe(3);
        expect(
            depthLimit(
                spec(null, [
                    ["broker", 4],
                    ["inv", 3],
                ]),
                fund
            )
        ).toBe(4);
        expect(
            depthLimit(
                spec(null, [
                    ["fund", 4],
                    ["assets", 2],
                ]),
                fund
            )
        ).toBe(4);
        // Case-insensitive, anywhere in the full name — `inv$` does not match a deeper account.
        expect(depthLimit(spec(null, [["INV", 2]]), fund)).toBe(2);
        expect(depthLimit(spec(null, [["inv$", 2]]), fund)).toBe(Infinity);
        // The plain depth covers what no regex matches; an invalid regex is skipped.
        expect(
            depthLimit(
                spec(1, [
                    ["inv", 3],
                    ["(", 9],
                ]),
                "assets:bank:checking"
            )
        ).toBe(1);
    });

    it("opens a group only when its accounts would show as more than one line", () => {
        const cash = ["assets:cash", "assets:cash:checking", "assets:cash:savings"];
        expect(groupStartsOpen(cash, spec(2))).toBe(false);
        expect(groupStartsOpen(cash, spec(3))).toBe(true);
        expect(groupStartsOpen(cash, spec(2, [["savings", 3]]))).toBe(true);
        // No depth configured: the long-standing default, all collapsed.
        expect(groupStartsOpen(cash, spec(null))).toBe(false);
        expect(groupStartsOpen(cash, null)).toBe(false);
        // Only a regex: what it does not match is unlimited, as in hledger.
        expect(groupStartsOpen(cash, spec(null, [["inv", 1]]))).toBe(true);
    });

    it("decides each group once per spec, and only ever opens", () => {
        const seed = newOpenSeed();
        const groups = [
            {key: "a", accounts: ["assets:a", "assets:a:x"]},
            {key: "b", accounts: ["assets:b"]},
        ];
        expect(seedOpenGroups(seed, groups, spec(3))).toEqual(["a"]);
        // A refetch of the same report under the same spec: nothing re-opens.
        expect(seedOpenGroups(seed, groups, spec(3))).toEqual([]);
        // A new group appears: it alone is decided.
        expect(seedOpenGroups(seed, [...groups, {key: "c", accounts: ["x:c", "x:c:d"]}], spec(3))).toEqual(["c"]);
        // The spec changes: everything is decided afresh.
        expect(seedOpenGroups(seed, groups, spec(3))).toEqual([]);
        expect(seedOpenGroups(seed, groups, spec(9))).toEqual(["a"]);
    });
});
