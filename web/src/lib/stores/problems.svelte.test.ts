// The problems store asks the engine once per (generation, today) — the badge
// is mounted by the layout and never unmounts, so how OFTEN it fetches is the
// thing worth pinning, in both directions: not on every call, and always when
// the journal (or the day) moves.

import {afterEach, describe, expect, it, vi} from "vitest";
import {FAKE_ENGINE, routes} from "#lib/testing/fakeEngine.js";
import {problems} from "./problems.svelte";

const body = {
    count: 2,
    maxSeverity: "warning",
    diagnostics: [
        {
            txnIndex: 2,
            tindex: 3,
            date: "2025-01-05",
            description: "Grocery",
            rule: "pending",
            severity: "warning",
            message: "transaction is marked pending (!)",
        },
        {account: "assets:bank", rule: "negative-cash", severity: "warning", message: "cash balance is negative: $ -5.00 as of 2025-01-17"},
    ],
};

afterEach(() => vi.unstubAllGlobals());

describe("UNIT problems store", () => {
    it("loads once per (server, generation, today), and again when either moves", async () => {
        const fetchSpy = vi.fn(routes({"/api/diagnostics": body}));
        vi.stubGlobal("fetch", fetchSpy);

        await problems.load(FAKE_ENGINE, "gen-1", "2025-01-15");
        expect(problems.count).toBe(2);
        expect(problems.maxSeverity).toBe("warning");
        expect(problems.byTxn.get(3)?.[0].rule).toBe("pending");
        // Account findings flag no row.
        expect([...problems.byTxn.keys()]).toEqual([3]);
        expect(String(fetchSpy.mock.calls[0][0])).toContain("today=2025-01-15");

        await problems.load(FAKE_ENGINE, "gen-1", "2025-01-15");
        expect(fetchSpy).toHaveBeenCalledTimes(1);

        await problems.load(FAKE_ENGINE, "gen-2", "2025-01-15");
        await problems.load(FAKE_ENGINE, "gen-2", "2025-01-16");
        expect(fetchSpy).toHaveBeenCalledTimes(3);
    });

    it("keeps the last good list when a load fails", async () => {
        vi.stubGlobal("fetch", vi.fn(routes({"/api/diagnostics": body})));
        await problems.load(FAKE_ENGINE, "gen-3", "2025-01-15");
        vi.stubGlobal("fetch", vi.fn(routes({})));
        await problems.load(FAKE_ENGINE, "gen-4", "2025-01-15");
        expect(problems.count).toBe(2);
    });
});
