import {describe, expect, it} from "vitest";
import {defaultImportParams, isTab, paramsToSearch, searchToParams, TAB_LABELS, TAB_ORDER, type ImportParams} from "./params";

const DFLT = defaultImportParams();

describe("UNIT imports/params", () => {
    describe("defaultImportParams", () => {
        it("lands on New Transactions", () => {
            expect(DFLT).toEqual({tab: "new"});
        });

        it("hands back a fresh object each call, so a caller's edit cannot move the default", () => {
            const first = defaultImportParams();
            first.tab = "rules";
            expect(defaultImportParams()).toEqual({tab: "new"});
        });
    });

    describe("TAB_ORDER", () => {
        it("puts New Transactions first and labels every tab", () => {
            expect(TAB_ORDER).toEqual(["new", "rules", "aliases"]);
            expect(TAB_ORDER[0]).toBe(DFLT.tab);
            expect(TAB_ORDER.map((t) => TAB_LABELS[t])).toEqual(["New Transactions", "Edit Rules", "Account Aliases"]);
        });
    });

    describe("isTab", () => {
        it("accepts exactly the known ids", () => {
            expect(TAB_ORDER.every(isTab)).toBe(true);
            expect(isTab("rules")).toBe(true);
            // Neither a label nor a near-miss is a tab id.
            expect(isTab("Edit Rules")).toBe(false);
            expect(isTab("")).toBe(false);
            expect(isTab("newx")).toBe(false);
            // `Array.prototype.includes` on a plain array, so no prototype key sneaks through.
            expect(isTab("toString")).toBe(false);
        });
    });

    describe("paramsToSearch", () => {
        it("writes the tab, and only the tab", () => {
            expect(paramsToSearch({tab: "new"})).toBe("tab=new");
            expect(paramsToSearch({tab: "rules"})).toBe("tab=rules");
        });
    });

    describe("searchToParams", () => {
        it("round-trips every tab losslessly", () => {
            for (const tab of TAB_ORDER) {
                const params: ImportParams = {tab};
                expect(searchToParams(paramsToSearch(params), DFLT)).toEqual(params);
            }
        });

        it("accepts a search with or without the leading '?'", () => {
            expect(searchToParams("?tab=rules", DFLT)).toEqual({tab: "rules"});
            expect(searchToParams("tab=rules", DFLT)).toEqual({tab: "rules"});
        });

        it("falls back for an absent, empty or unknown tab rather than rendering nothing", () => {
            expect(searchToParams("", DFLT)).toEqual(DFLT);
            expect(searchToParams("?", DFLT)).toEqual(DFLT);
            expect(searchToParams("?tab=", DFLT)).toEqual(DFLT);
            expect(searchToParams("?tab=bs", DFLT)).toEqual(DFLT);
            // A stale link from another surface must not strand the page on a blank tab.
            expect(searchToParams("?tab=insights&asof=2026-01-01", DFLT)).toEqual(DFLT);
        });

        it("falls back to the CALLER's params, not to the module default", () => {
            expect(searchToParams("?tab=nope", {tab: "rules"})).toEqual({tab: "rules"});
        });

        it("ignores keys it does not own", () => {
            expect(searchToParams("?file=scratch.csv.rules&tab=rules", DFLT)).toEqual({tab: "rules"});
        });
    });
});
