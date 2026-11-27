import {describe, expect, it} from "vitest";
import {TAB_ORDER} from "#lib/holdings/params.js";
import type {HoldingsScope} from "#lib/holdings/types.js";
import {scopeToSearch, searchToScope, searchToState, stateToSearch} from "./urlCodec";

const TODAY = "2026-07-08";

function scope(overrides: Partial<HoldingsScope> = {}): HoldingsScope {
    return {accounts: new Set<string>(), mode: "include", asOf: TODAY, gainPeriod: "12mo", holdings: new Set<string>(), ...overrides};
}

describe("UNIT holdings urlCodec — the holdings filter (plan 47)", () => {
    it("writes hold= sorted and percent-encoded per item, and omits it when empty", () => {
        expect(scopeToSearch(scope(), TODAY)).toBe("");
        expect(scopeToSearch(scope({holdings: new Set(["VTI", "AAPL"])}), TODAY)).toBe("hold=AAPL%2CVTI");
        // An Other-tab account name with a comma survives the join.
        expect(scopeToSearch(scope({holdings: new Set(["assets:house, lake"])}), TODAY)).toBe("hold=assets%253Ahouse%252C%2520lake");
    });

    it("round-trips through the query string", () => {
        const holdings = new Set(["assets:house, lake", "assets:van"]);
        const parsed = searchToScope(scopeToSearch(scope({holdings}), TODAY), TODAY);
        expect([...parsed.holdings].sort()).toEqual([...holdings].sort());
    });

    it("reads an absent, empty or malformed hold= as no filter, never throwing", () => {
        expect(searchToScope("", TODAY).holdings.size).toBe(0);
        expect(searchToScope("hold=", TODAY).holdings.size).toBe(0);
        expect([...searchToScope("hold=%", TODAY).holdings]).toEqual(["%"]);
        expect([...searchToScope("hold=VTI,,AAPL", TODAY).holdings].sort()).toEqual(["AAPL", "VTI"]);
    });

    it("travels with the tab through the one writer", () => {
        const state = {scope: scope({holdings: new Set(["assets:van"])}), tab: "other" as const, view: "value" as const};
        expect(stateToSearch(state, TODAY)).toBe("hold=assets%253Avan&tab=other");
        const back = searchToState(stateToSearch(state, TODAY), TODAY);
        expect(back.tab).toBe("other");
        expect([...back.scope.holdings]).toEqual(["assets:van"]);
    });
});

describe("UNIT holdings urlCodec — the chart view (plan 47 A1b)", () => {
    const state = (view: "value" | "perf", tab: "stocks" | "other" = "stocks") => ({scope: scope(), tab, view});

    it("writes view=perf, and nothing for the Value default", () => {
        expect(stateToSearch(state("value"), TODAY)).toBe("");
        expect(stateToSearch(state("perf"), TODAY)).toBe("view=perf");
        expect(stateToSearch({...state("perf", "other"), scope: scope({gainPeriod: "ytd"})}, TODAY)).toBe("gain=ytd&tab=other&view=perf");
    });

    it("round-trips both views", () => {
        expect(searchToState(stateToSearch(state("perf"), TODAY), TODAY)).toEqual(state("perf"));
        expect(searchToState(stateToSearch(state("value"), TODAY), TODAY)).toEqual(state("value"));
    });

    it("opens the Value chart on an absent, empty or unknown view", () => {
        expect(searchToState("", TODAY).view).toBe("value");
        expect(searchToState("?view=", TODAY).view).toBe("value");
        expect(searchToState("?view=returns", TODAY).view).toBe("value");
        expect(searchToState("?view=value", TODAY).view).toBe("value");
    });

    it("stays out of the scope, so switching refetches nothing", () => {
        expect(searchToScope("?view=perf", TODAY)).toEqual(scope());
    });
});

describe("UNIT holdings urlCodec", () => {
    describe("scopeToSearch", () => {
        it("serializes the fresh-visit default to an empty string", () => {
            expect(scopeToSearch(scope(), TODAY)).toBe("");
        });

        it("writes asof only when it differs from today (never a remembered date)", () => {
            expect(scopeToSearch(scope({asOf: "2025-01-01"}), TODAY)).toBe("asof=2025-01-01");
        });

        it("writes accounts sorted and mode only when exclude", () => {
            const s = scope({accounts: new Set(["expenses", "assets:broker"]), mode: "exclude"});
            expect(scopeToSearch(s, TODAY)).toBe("acct=assets%253Abroker%2Cexpenses&mode=exclude");
        });

        it("writes gain only when the window isn't the 12-month default", () => {
            expect(scopeToSearch(scope({gainPeriod: "12mo"}), TODAY)).toBe("");
            expect(scopeToSearch(scope({gainPeriod: "ytd"}), TODAY)).toBe("gain=ytd");
            expect(scopeToSearch(scope({gainPeriod: "all"}), TODAY)).toBe("gain=all");
        });

        it("round-trips account names containing commas", () => {
            const s = scope({accounts: new Set(["assets:a,b", "assets:c"])});
            const parsed = searchToScope(scopeToSearch(s, TODAY), TODAY);
            expect([...parsed.accounts].sort()).toEqual(["assets:a,b", "assets:c"]);
        });
    });

    describe("searchToScope", () => {
        it("absent params always mean today/empty/include", () => {
            expect(searchToScope("", TODAY)).toEqual(scope());
            expect(searchToScope("?", TODAY)).toEqual(scope());
        });

        it("parses a full query with or without the leading question mark", () => {
            const expected = scope({asOf: "2025-01-01", accounts: new Set(["assets:broker"]), mode: "exclude"});
            expect(searchToScope("?asof=2025-01-01&acct=assets%3Abroker&mode=exclude", TODAY)).toEqual(expected);
            expect(searchToScope("asof=2025-01-01&acct=assets%3Abroker&mode=exclude", TODAY)).toEqual(expected);
        });

        it("falls back to today on malformed asof and include on unknown mode", () => {
            expect(searchToScope("?asof=notadate&mode=banana", TODAY)).toEqual(scope());
        });

        it("parses gain and falls back to the 12-month default on an unknown window", () => {
            expect(searchToScope("?gain=ytd", TODAY)).toEqual(scope({gainPeriod: "ytd"}));
            expect(searchToScope("?gain=all", TODAY)).toEqual(scope({gainPeriod: "all"}));
            expect(searchToScope("?gain=banana", TODAY)).toEqual(scope());
        });

        it("round-trips every gain window, the new ones included", () => {
            for (const gainPeriod of ["1wk", "1mo", "3mo", "ytd", "12mo", "5yr", "all"] as const) {
                const s = scope({gainPeriod});
                expect(searchToScope(scopeToSearch(s, TODAY), TODAY)).toEqual(s);
            }
            expect(scopeToSearch(scope({gainPeriod: "1wk"}), TODAY)).toBe("gain=1wk");
            expect(scopeToSearch(scope({gainPeriod: "5yr"}), TODAY)).toBe("gain=5yr");
        });

        it("refuses a near-miss window rather than guessing", () => {
            expect(searchToScope("?gain=1w", TODAY)).toEqual(scope());
            expect(searchToScope("?gain=5YR", TODAY)).toEqual(scope());
        });

        it("ignores empty account segments", () => {
            expect([...searchToScope("?acct=a,,b", TODAY).accounts].sort()).toEqual(["a", "b"]);
        });

        // SEC-12: this ran inside onMount, so a URIError here blanked the page.
        it("survives a malformed percent escape in acct instead of throwing (SEC-12)", () => {
            expect(() => searchToScope("?acct=%", TODAY)).not.toThrow();
            expect([...searchToScope("?acct=%", TODAY).accounts]).toEqual(["%"]);
        });

        it("keeps the well-formed accounts and the other params when a segment is malformed", () => {
            const parsed = searchToScope("?acct=assets%3Abroker,%zz&asof=2025-01-01&mode=exclude", TODAY);
            expect([...parsed.accounts].sort()).toEqual(["%zz", "assets:broker"]);
            expect(parsed.asOf).toBe("2025-01-01");
            expect(parsed.mode).toBe("exclude");
        });

        it("round-trips a non-default scope", () => {
            const s = scope({asOf: "2024-12-31", accounts: new Set(["assets:broker:taxable:vti"]), mode: "exclude", gainPeriod: "ytd"});
            expect(searchToScope(scopeToSearch(s, TODAY), TODAY)).toEqual(s);
        });

        it("ignores the tab, which is not part of the scope", () => {
            // The tab must never reach HoldingsScope: the scope is the report
            // resource's refetch key, so a tab in it refetches on every click.
            expect(searchToScope("?tab=other", TODAY)).toEqual(scope());
        });
    });

    // plans/14: the tab travels in the SAME query string, through the same codec,
    // because this screen must have exactly one writer to it.
    describe("stateToSearch", () => {
        it("omits the default tab, so a fresh-visit URL stays bare", () => {
            expect(stateToSearch({scope: scope(), tab: "stocks", view: "value"}, TODAY)).toBe("");
        });

        it("writes tab last, beside the scope params rather than instead of them", () => {
            expect(stateToSearch({scope: scope(), tab: "other", view: "value"}, TODAY)).toBe("tab=other");
            expect(stateToSearch({scope: scope({asOf: "2025-01-01", gainPeriod: "ytd"}), tab: "other", view: "value"}, TODAY)).toBe(
                "asof=2025-01-01&gain=ytd&tab=other"
            );
        });

        it("serializes the scope exactly as scopeToSearch does, so the two cannot drift", () => {
            const s = scope({accounts: new Set(["expenses", "assets:broker"]), mode: "exclude"});
            expect(stateToSearch({scope: s, tab: "stocks", view: "value"}, TODAY)).toBe(scopeToSearch(s, TODAY));
        });

        it("keeps the default tab derived from TAB_ORDER, not restated", () => {
            expect(stateToSearch({scope: scope(), tab: TAB_ORDER[0], view: "value"}, TODAY)).toBe("");
        });
    });

    describe("searchToState", () => {
        it("absent params mean today/empty/include and the first tab", () => {
            expect(searchToState("", TODAY)).toEqual({scope: scope(), tab: "stocks", view: "value"});
            expect(searchToState("?", TODAY)).toEqual({scope: scope(), tab: "stocks", view: "value"});
        });

        it("parses the tab with or without the leading question mark", () => {
            expect(searchToState("?tab=other", TODAY).tab).toBe("other");
            expect(searchToState("tab=other", TODAY).tab).toBe("other");
        });

        it("opens Stocks on an empty or unknown tab rather than stranding the page on a blank sub-screen", () => {
            expect(searchToState("?tab=", TODAY).tab).toBe("stocks");
            expect(searchToState("?tab=banana", TODAY).tab).toBe("stocks");
            // A stale link from another surface (Imports uses ?tab= too).
            expect(searchToState("?tab=rules&asof=2025-01-01", TODAY)).toEqual({scope: scope({asOf: "2025-01-01"}), tab: "stocks", view: "value"});
        });

        it("round-trips every tab beside a non-default scope", () => {
            for (const tab of TAB_ORDER) {
                const state = {
                    scope: scope({asOf: "2024-12-31", accounts: new Set(["assets:property:house"]), mode: "exclude", gainPeriod: "12mo"}),
                    tab,
                    view: "value" as const,
                };
                expect(searchToState(stateToSearch(state, TODAY), TODAY)).toEqual(state);
            }
        });

        it("survives a malformed percent escape beside a tab (SEC-12 still holds through the new entry point)", () => {
            expect(() => searchToState("?acct=%&tab=other", TODAY)).not.toThrow();
            expect(searchToState("?acct=%&tab=other", TODAY).tab).toBe("other");
        });
    });
});
