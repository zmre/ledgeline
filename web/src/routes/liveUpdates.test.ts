// Live updates reach every screen, not only the journal.
//
// The poller used to start in the journal page's own effect, so a report,
// holdings, budget or projections screen never learned the file had changed on
// disk and kept the old figures until the user left and came back. Mounting
// `+layout.svelte` is not available here (it reads `$app/state`'s page, which a
// real navigation populates), so this reads the source, on the same trade
// `effectLatch.test.ts` and `refreshAll.test.ts` make.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";

const source = (path: string): string => readFileSync(new URL(path, import.meta.url), "utf8");

describe("live updates", () => {
    it("are started by the root layout, which every screen sits inside", () => {
        expect(source("./+layout.svelte")).toContain("return startPolling();");
    });

    it("are not started by any one page, which would only cover that page", () => {
        expect(source("./+page.svelte")).not.toContain("startPolling");
    });

    it("refetch each journal-derived screen when the journal changes", () => {
        for (const page of ["./reports/+page.svelte", "./holdings/+page.svelte", "./budget/+page.svelte", "./projections/+page.svelte"]) {
            expect(source(page), page).toMatch(/changes: journal\.changes/);
        }
    });
});
