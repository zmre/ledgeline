// `problems.all` runs every check over every transaction — 30-46 ms and 21,429
// findings at 150k. The navbar badge reads it, the badge is mounted by the
// layout, and the layout never unmounts, so it is worth pinning HOW OFTEN that
// runs: once per journal swap, not once per tab.
//
// This is a memoization test, and memoization tests earn their keep by failing
// in both directions. Dropping the `$derived` (reading the checks eagerly, or
// through a getter that recomputes) makes the first assertion fail; breaking the
// dependency on the journal — the mistake that would leave a stale badge sitting
// over a corrected journal — makes the second fail.

import {describe, expect, it, vi} from "vitest";
import type {Problem} from "$lib/checks/engine";

const runChecks = vi.fn((): Problem[] => [{txnIndex: 1, rule: "pending", severity: "warning", message: "m"}]);

// Real rune state, replaced wholesale, exactly as the journal store does it
// (its payloads are `$state.raw`).
let txns = $state.raw<unknown[]>([{index: 1}]);

vi.mock("$lib/checks/engine", () => ({
    runChecks: (...args: unknown[]) => runChecks(...(args as [])),
    groupByTxn: () => new Map(),
    maxSeverity: () => "warning",
}));
vi.mock("$lib/stores/journal.svelte", () => ({
    journal: {
        get txns() {
            return txns;
        },
        get prices() {
            return [];
        },
        get accountDecls() {
            return [];
        },
        get diagnostics() {
            return [];
        },
        get engineChecked() {
            return true;
        },
    },
}));

const {problems} = await import("$lib/stores/problems.svelte");

describe("UNIT problems.all is computed once per journal, not once per read", () => {
    it("does not re-run the checks when the badge is read again on another tab", () => {
        // Settle first, so this measures repeat reads and not whatever ran before.
        txns = [{index: 1}];
        void problems.count;
        runChecks.mockClear();

        // Five reads with nothing changed — five tab visits' worth of badge.
        for (let i = 0; i < 5; i += 1) void problems.count;

        expect(runChecks).not.toHaveBeenCalled();
    });

    it("does re-run them when the journal is swapped", () => {
        void problems.count;
        runChecks.mockClear();

        txns = [{index: 1}, {index: 2}];
        void problems.count;

        expect(runChecks).toHaveBeenCalledTimes(1);
    });
});
