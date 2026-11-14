import {describe, expect, it} from "vitest";
import {isBelowMd, MD_REM} from "./appWidth.svelte";

describe("isBelowMd", () => {
    it("matches app.css's md breakpoint (48rem)", () => {
        expect(MD_REM).toBe(48);
        expect(isBelowMd(767, 16)).toBe(true);
        expect(isBelowMd(768, 16)).toBe(false);
    });

    it("moves with the interface scale, as the container query does", () => {
        // 1024px at 200 % is 32rem: a phone layout.
        expect(isBelowMd(1024, 32)).toBe(true);
        expect(isBelowMd(390, 11.2)).toBe(true);
        expect(isBelowMd(600, 11.2)).toBe(false);
    });

    it("treats an unmeasured width as the desktop layout", () => {
        expect(isBelowMd(null, 16)).toBe(false);
    });
});
