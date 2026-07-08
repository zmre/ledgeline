import {describe, expect, it} from "vitest";

// Placeholder unit test proving the vitest toolchain runs (WP-01).
// Real domain tests arrive with WP-02+.
describe("scaffold smoke test", () => {
    it("runs vitest under strict TypeScript", () => {
        const routes: ReadonlyArray<string> = ["/", "/reports"];
        expect(routes).toHaveLength(2);
    });
});
