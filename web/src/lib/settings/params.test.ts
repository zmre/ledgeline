import {describe, expect, it} from "vitest";
import {defaultSettingsParams, isTab, paramsToSearch, searchToParams, TAB_LABELS, TAB_ORDER, type SettingsParams} from "./params";

const DFLT = defaultSettingsParams();

describe("UNIT settings/params", () => {
    describe("defaultSettingsParams", () => {
        it("lands on Accounts", () => {
            expect(DFLT).toEqual({tab: "accounts"});
        });

        it("hands back a fresh object each call, so a caller's edit cannot move the default", () => {
            const first = defaultSettingsParams();
            first.tab = "aliases";
            expect(defaultSettingsParams()).toEqual({tab: "accounts"});
        });
    });

    describe("TAB_ORDER", () => {
        it("puts Accounts first and labels every tab", () => {
            expect(TAB_ORDER).toEqual(["accounts", "aliases"]);
            expect(TAB_ORDER[0]).toBe(DFLT.tab);
            expect(TAB_ORDER.map((t) => TAB_LABELS[t])).toEqual(["Accounts", "Account Aliases"]);
        });
    });

    describe("isTab", () => {
        it("accepts exactly the known ids", () => {
            expect(TAB_ORDER.every(isTab)).toBe(true);
            expect(isTab("")).toBe(false);
            expect(isTab("Accounts")).toBe(false);
            expect(isTab("toString")).toBe(false);
        });
    });

    describe("searchToParams / paramsToSearch", () => {
        it("round-trips every tab losslessly", () => {
            for (const tab of TAB_ORDER) {
                const params: SettingsParams = {tab};
                expect(searchToParams(paramsToSearch(params), DFLT)).toEqual(params);
            }
        });

        it("accepts a search with or without the leading '?'", () => {
            expect(searchToParams("?tab=aliases", DFLT)).toEqual({tab: "aliases"});
            expect(searchToParams("tab=aliases", DFLT)).toEqual({tab: "aliases"});
        });

        it("falls back for an absent, empty or unknown tab", () => {
            expect(searchToParams("", DFLT)).toEqual(DFLT);
            expect(searchToParams("?tab=bs", DFLT)).toEqual(DFLT);
        });
    });
});
