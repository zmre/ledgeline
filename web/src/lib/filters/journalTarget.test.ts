// One description of "show me this in the journal", shared by four surfaces.
//
// Worth its own tests because it replaced two independent implementations —
// BudgetSummary built a filter and navigated, SubscriptionsBox hand-rolled a
// query string — and the failure mode of a drill-down going to the wrong place
// is silent: you land on a journal that looks plausible and is filtered wrong.

import {describe, expect, it} from "vitest";
import {journalSearch, targetToFilter} from "./journalTarget";

describe("UNIT targetToFilter", () => {
    it("puts accounts into the filter's account set", () => {
        expect([...targetToFilter({accounts: ["expenses:gas"]}).accounts]).toEqual(["expenses:gas"]);
    });

    it("carries several accounts, for a holding spread across brokerages", () => {
        const filter = targetToFilter({accounts: ["assets:a:tsla", "assets:b:tsla"]});

        expect(filter.accounts.size).toBe(2);
    });

    it("carries a free-text query", () => {
        expect(targetToFilter({query: "Netflix"}).query).toBe("Netflix");
    });

    it("takes explicit dates and drops the preset, since the range is now hand-picked", () => {
        const filter = targetToFilter({from: "2026-01-01", to: "2026-01-31"});

        expect({from: filter.from, to: filter.to, preset: filter.preset}).toEqual({from: "2026-01-01", to: "2026-01-31", preset: null});
    });

    it("keeps a preset live rather than freezing it to dates", () => {
        // The whole point of storing the preset name: a restored "all" or "ytd"
        // recomputes against today instead of pinning whenever it was clicked.
        const filter = targetToFilter({preset: "all"});

        expect({preset: filter.preset, from: filter.from, to: filter.to}).toEqual({preset: "all", from: null, to: null});
    });

    it("falls back to the default preset when given no dates at all", () => {
        expect(targetToFilter({accounts: ["expenses:gas"]}).preset).not.toBeNull();
    });
});

describe("UNIT journalSearch", () => {
    it("encodes an account filter", () => {
        expect(journalSearch({accounts: ["expenses:gas"], preset: "all"})).toContain("acct=");
    });

    it("encodes a query, escaped", () => {
        // The payee drill-down passes arbitrary text; an unescaped `&` would
        // silently truncate the filter.
        expect(journalSearch({query: "AT&T", preset: "all"})).toContain(encodeURIComponent("AT&T"));
    });

    it("produces the all-dates payee link the subscriptions box relies on", () => {
        const search = journalSearch({query: "Netflix", preset: "all"});

        expect(search).toContain("preset=all");
        expect(search).toContain("q=Netflix");
    });

    it("omits anything left at its default, so links stay short and legible", () => {
        expect(journalSearch({})).toBe("");
    });
});
