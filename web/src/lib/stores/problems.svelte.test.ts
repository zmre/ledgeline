// The problems store asks the engine once per (generation, today) — the badge
// is mounted by the layout and never unmounts, so how OFTEN and how MUCH it
// fetches is the thing worth pinning: a count-only answer for the badge, the
// full list only while the drawer is open, and a re-fetch whenever the journal
// (or the day) moves.

import {afterEach, describe, expect, it, vi} from "vitest";
import {FAKE_ENGINE, routes} from "#lib/testing/fakeEngine.js";
import {problems} from "./problems.svelte";

const entries = [
    {txnIndex: 2, tindex: 3, date: "2025-01-05", description: "Grocery", rule: "pending", severity: "warning", message: "transaction is marked pending (!)"},
    {account: "assets:bank", rule: "negative-cash", severity: "warning", message: "cash balance is negative: $ -5.00 as of 2025-01-17"},
];

/** A fake `/api/diagnostics` that honours `limit`, recording every URL it was asked. */
function engine(): {calls: string[]} {
    const calls: string[] = [];
    const answer = routes({"/api/diagnostics": {count: 2, maxSeverity: "warning", diagnostics: entries}});
    vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
        const url = String(input);
        calls.push(url);
        if (url.includes("limit=0")) return Promise.resolve(new Response(JSON.stringify({count: 2, maxSeverity: "warning", diagnostics: []})));
        return answer(url);
    });
    return {calls};
}

afterEach(() => {
    problems.drawerOpen = false;
    vi.unstubAllGlobals();
});

describe("UNIT problems store", () => {
    it("asks only for the count while the drawer is shut, once per (server, generation, today)", async () => {
        const {calls} = engine();
        await problems.load(FAKE_ENGINE, "gen-1", "2025-01-15");
        expect(problems.count).toBe(2);
        expect(problems.maxSeverity).toBe("warning");
        expect(calls).toHaveLength(1);
        expect(calls[0]).toContain("today=2025-01-15");
        expect(calls[0]).toContain("limit=0");

        await problems.load(FAKE_ENGINE, "gen-1", "2025-01-15");
        expect(calls).toHaveLength(1);

        await problems.load(FAKE_ENGINE, "gen-2", "2025-01-15");
        await problems.load(FAKE_ENGINE, "gen-2", "2025-01-16");
        expect(calls).toHaveLength(3);
    });

    it("loads the whole list when the drawer opens, and keeps it in step while open", async () => {
        const {calls} = engine();
        await problems.load(FAKE_ENGINE, "gen-3", "2025-01-15");
        expect(problems.listLoading).toBe(true);

        problems.drawerOpen = true;
        await vi.waitFor(() => expect(problems.all).toHaveLength(2));
        expect(problems.listLoading).toBe(false);
        expect(calls.at(-1)).not.toContain("limit=");
        expect(problems.all[0]).toMatchObject({txnIndex: 3, date: "2025-01-05", description: "Grocery"});

        // The journal moved while the drawer stayed open: the list follows.
        await problems.load(FAKE_ENGINE, "gen-4", "2025-01-15");
        expect(calls.at(-1)).not.toContain("limit=");
        expect(problems.listLoading).toBe(false);
    });

    it("keeps the last good count when a load fails", async () => {
        engine();
        await problems.load(FAKE_ENGINE, "gen-5", "2025-01-15");
        vi.stubGlobal("fetch", vi.fn(routes({})));
        await problems.load(FAKE_ENGINE, "gen-6", "2025-01-15");
        expect(problems.count).toBe(2);
    });
});
