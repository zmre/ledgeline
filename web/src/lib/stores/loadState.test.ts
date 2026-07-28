import {describe, expect, it} from "vitest";
import {dataView} from "./loadState";

describe("UNIT dataView (error-as-data branch selection)", () => {
    it("reports an error even when a payload from an EARLIER load is still held", () => {
        // The whole of FE-5. Every surface asked for `payload === null && status === "error"`,
        // which is unsatisfiable once anything has loaded: move the as-of, take a
        // 500, and December's balance sheet stayed on screen under a June control.
        expect(dataView("error", true)).toBe("error");
    });

    it("reports an error before the first payload too", () => {
        expect(dataView("error", false)).toBe("error");
    });

    it("keeps the held payload visible across a refetch (only the first load spins)", () => {
        expect(dataView("loading", true)).toBe("data");
        expect(dataView("ready", true)).toBe("data");
    });

    it("spins before anything has loaded", () => {
        expect(dataView("idle", false)).toBe("loading");
        expect(dataView("loading", false)).toBe("loading");
    });

    it("treats a payload that answers a DIFFERENT request as not loaded (FE-1)", () => {
        // A loaded balance sheet must not stand in for the P&L tab while the
        // P&L loads — `bs` and `is` are both SectionedReport, so nothing about
        // the value itself would object.
        expect(dataView("loading", true, false)).toBe("loading");
        expect(dataView("ready", true, false)).toBe("loading");
    });

    it("still prefers the error over a mismatched payload", () => {
        expect(dataView("error", true, false)).toBe("error");
    });
});
