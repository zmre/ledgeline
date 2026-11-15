import {describe, expect, it} from "vitest";
import {fixtureManual, FIXTURE_TODAY} from "./fixtures";
import {manualStale} from "./manualStale";

describe("UNIT sync manualStale", () => {
    const [vanguard, cash] = fixtureManual();

    it("says when it was imported and how long ago", () => {
        expect(manualStale(vanguard, FIXTURE_TODAY, "en-US")).toEqual({stale: true, line: "Last imported Oct 2 (43 days ago) · last transaction Sep 30"});
    });

    it("turns stale only after 35 days, and only with the nudge on", () => {
        const fresh = {...vanguard, stale: false, lastTransaction: "2026-10-10"};
        expect(manualStale(fresh, FIXTURE_TODAY).stale).toBe(false);
        expect(manualStale({...fresh, lastTransaction: "2026-10-09"}, FIXTURE_TODAY).stale).toBe(true);
        expect(manualStale({...vanguard, nudge: false}, FIXTURE_TODAY).stale).toBe(false);
    });

    it("falls back to the last transaction, then to nothing", () => {
        expect(manualStale(cash, FIXTURE_TODAY, "en-US")).toEqual({stale: false, line: "Last transaction Nov 10 (4 days ago)"});
        expect(manualStale({...cash, lastTransaction: null}, FIXTURE_TODAY).line).toBe("No transactions yet");
        expect(manualStale({...cash, lastTransaction: FIXTURE_TODAY}, FIXTURE_TODAY, "en-US").line).toBe("Last transaction Nov 14 (today)");
    });
});
