import {describe, expect, it} from "vitest";
import {isTab, TAB_LABELS, TAB_ORDER, type HoldingsTab} from "./params";

describe("UNIT holdings/params", () => {
    describe("TAB_ORDER", () => {
        it("puts Stocks first — the screen Holdings has always opened on — and labels every tab", () => {
            expect(TAB_ORDER).toEqual(["stocks", "other"]);
            expect(TAB_ORDER.map((t) => TAB_LABELS[t])).toEqual(["Stocks", "Other"]);
        });

        it("has a label for every id and no label for anything else", () => {
            expect(Object.keys(TAB_LABELS).sort()).toEqual([...TAB_ORDER].sort());
        });
    });

    describe("isTab", () => {
        it("accepts exactly the known ids", () => {
            expect(TAB_ORDER.every(isTab)).toBe(true);
            expect(isTab("other")).toBe(true);
            // Neither a label nor a near-miss is a tab id.
            expect(isTab("Stocks")).toBe(false);
            expect(isTab("")).toBe(false);
            expect(isTab("stock")).toBe(false);
            // `Array.prototype.includes` on a plain array, so no prototype key sneaks through.
            expect(isTab("toString")).toBe(false);
        });
    });

    describe("round-trip", () => {
        it("narrows every id back to itself, so a serialized tab always restores", () => {
            for (const tab of TAB_ORDER) {
                const raw: string = tab;
                expect(isTab(raw)).toBe(true);
                // The narrowing is what the codec leans on; assert the value survives it.
                const restored: HoldingsTab | null = isTab(raw) ? raw : null;
                expect(restored).toBe(tab);
            }
        });
    });
});
