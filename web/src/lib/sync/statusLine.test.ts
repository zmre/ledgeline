import {describe, expect, it} from "vitest";
import {chaseLinked, emptyStatus, firstTechLinked, fixtureConnection, fixtureRun, fixtureStatus, FIXTURE_NOW, revokedProblem} from "./fixtures";
import {accountLine, dataThroughLine, quotaNote, runToast, statusLine} from "./statusLine";

describe("UNIT sync statusLine", () => {
    it("says auto-sync, last and next", () => {
        expect(statusLine(fixtureStatus(), FIXTURE_NOW, "en-US")).toBe("Auto-sync on · last synced 7:12 AM · next around 1:15 PM");
    });

    it("leaves out next when auto-sync is off", () => {
        expect(statusLine({...fixtureStatus(), autoSync: false}, FIXTURE_NOW, "en-US")).toBe("Auto-sync off · last synced 7:12 AM");
    });

    it("says not synced yet before the first success", () => {
        const status = fixtureStatus();
        status.connections[0].lastSuccess = null;
        expect(statusLine(status, FIXTURE_NOW, "en-US")).toBe("Auto-sync on · not synced yet · next around 1:15 PM");
    });

    it("says syncing while a run is going, and nothing with nothing linked or no feature", () => {
        expect(statusLine({...fixtureStatus(), running: true}, FIXTURE_NOW)).toBe("Syncing…");
        expect(statusLine(emptyStatus(), FIXTURE_NOW)).toBeNull();
        expect(statusLine({...fixtureStatus(), available: false}, FIXTURE_NOW)).toBeNull();
    });

    it("says next any minute when the next run is overdue", () => {
        expect(statusLine({...fixtureStatus(), nextAuto: FIXTURE_NOW - 60}, FIXTURE_NOW, "en-US")).toContain("next any minute");
    });

    it("summarizes a linked account", () => {
        expect(accountLine(firstTechLinked(), FIXTURE_NOW, "USD", "en-US")).toBe("Synced 7:12 AM · 4 new · 2 to review · balance matches ✓");
        const differs = {...firstTechLinked(), held: [], balanceDiff: "-12.40"};
        expect(accountLine(differs, FIXTURE_NOW, "USD", "en-US")).toBe("Synced 7:12 AM · 4 new · ledger differs by $12.40");
        expect(accountLine({...chaseLinked(), lastSuccess: null}, FIXTURE_NOW, "USD", "en-US")).toBe("Not synced yet · balance matches ✓");
        expect(accountLine({...chaseLinked(), enabled: false}, FIXTURE_NOW)).toBe("Paused");
    });

    it("words the Sync now toast", () => {
        expect(runToast(fixtureRun())).toEqual({text: "Synced 3 accounts · 9 new transactions", review: 9, failed: false});
        const withUncategorized = fixtureRun();
        withUncategorized.accounts[0].uncategorized = 3;
        withUncategorized.accounts[1].uncategorized = 1;
        expect(runToast(withUncategorized).review).toBe(4);
        const quiet = fixtureRun();
        quiet.accounts = quiet.accounts.map((a) => ({...a, added: 0, outcome: "nothing-new" as const}));
        expect(runToast(quiet)).toEqual({text: "Synced 3 accounts · nothing new", review: 0, failed: false});
        const held = fixtureRun();
        held.accounts[0].held = 2;
        expect(runToast(held).text).toBe("Synced 3 accounts · 9 new transactions · 2 held for a look");
    });

    it("says why a run failed", () => {
        const run = {...fixtureRun(), ok: false, problems: [revokedProblem()]};
        run.accounts = run.accounts.map((a) => ({...a, added: 0, outcome: "failed" as const}));
        expect(runToast(run)).toEqual({text: "Sync failed: Ledgeline's access to SimpleFIN was turned off or revoked.", review: 0, failed: true});
    });

    it("warns about the quota from 12 and blocks at 16", () => {
        expect(quotaNote([fixtureConnection()])).toBeNull();
        expect(quotaNote([{...fixtureConnection(), requestsToday: 12}])).toEqual({
            text: "12 syncs today. SimpleFIN allows about 24 before it disables access.",
            blocked: false,
        });
        expect(quotaNote([{...fixtureConnection(), requestsToday: 16}])?.blocked).toBe(true);
        expect(quotaNote([])).toBeNull();
    });

    it("says where data stops", () => {
        expect(dataThroughLine("2026-11-11", "2026-11-14", "en-US")).toBe("Data stops at Nov 11");
    });
});
