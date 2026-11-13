// The pure half of the view-state move (plan 38 WP-5): reading an old
// localStorage blob, turning it into the one-time migration patch, and keeping
// pending saves to the keys that changed.

import {describe, expect, it} from "vitest";
import {
    applyPatch,
    cleanUiPrefs,
    defaultUiPrefs,
    isThemeId,
    legacyUiPatch,
    mergePatch,
    PINNABLE_UI_PREF_KEYS,
    THEME_IDS,
    UI_PREF_KEYS,
    UI_SCALE_DEFAULT,
    UI_SCALE_RANGE,
    withoutLocalKeys,
    withoutUiFields,
} from "./uiPrefs";

describe("UNIT uiPrefs", () => {
    it("names every view setting, and never the connection", () => {
        expect([...UI_PREF_KEYS].sort()).toEqual(Object.keys(defaultUiPrefs()).sort());
        expect(UI_PREF_KEYS).toHaveLength(21);
        expect(UI_PREF_KEYS).not.toContain("serverUrl");
        expect(UI_PREF_KEYS).not.toContain("serverToken");
    });

    it("cleans a stale blob the way the store always has", () => {
        const prefs = cleanUiPrefs({
            columns: {date: false, amount: "no"},
            insightsTab: "gone",
            holdingsPieDimension: "sector",
            lastProjectionId: "",
            benchmarks: ["SPY", "spy", "SPY", 3],
            matchWindow: 0,
            matchLookback: 99999,
            reviewRecentLimit: "lots",
        });
        expect(prefs.columns).toEqual({date: false, status: true, description: true, accounts: true, amount: true});
        expect(prefs.insightsTab).toBe("activity");
        expect(prefs.holdingsPieDimension).toBe("sector");
        expect(prefs.lastProjectionId).toBeNull();
        expect(prefs.benchmarks).toEqual(["SPY"]);
        expect(prefs.matchWindow).toBe(1);
        expect(prefs.matchLookback).toBe(36500);
        expect(prefs.reviewRecentLimit).toBe(50);
    });

    it("lists the 22 theme ids once each, and defaults to the OS-following one at 100 %", () => {
        expect(THEME_IDS).toHaveLength(22);
        expect(new Set(THEME_IDS).size).toBe(22);
        expect(defaultUiPrefs().theme).toBe("default");
        expect(defaultUiPrefs().uiScale).toBe(100);
        expect(UI_SCALE_DEFAULT).toBe(100);
        expect(UI_SCALE_RANGE).toEqual([50, 300]);
        expect(PINNABLE_UI_PREF_KEYS).toEqual(["theme", "uiScale"]);
        expect(isThemeId("tokyo-night-day")).toBe(true);
        expect(isThemeId("Nord")).toBe(false);
        expect(isThemeId(undefined)).toBe(false);
    });

    it("cleans a stale theme to the default and clamps the scale", () => {
        expect(cleanUiPrefs({theme: "nord", uiScale: 125})).toMatchObject({theme: "nord", uiScale: 125});
        expect(cleanUiPrefs({theme: "solarized-dark", uiScale: 112.6})).toMatchObject({theme: "solarized-dark", uiScale: 113});
        expect(cleanUiPrefs({theme: "retired-theme", uiScale: 10})).toMatchObject({theme: "default", uiScale: 50});
        expect(cleanUiPrefs({theme: 3, uiScale: 9999})).toMatchObject({theme: "default", uiScale: 300});
        expect(cleanUiPrefs({uiScale: "big"}).uiScale).toBe(100);
    });

    it("migrates a blob's theme and scale like any other key", () => {
        expect(legacyUiPatch({theme: "dracula", uiScale: 400})).toEqual({theme: "dracula", uiScale: 300});
        expect(withoutLocalKeys({theme: "dracula", uiScale: 90}, ["theme"])).toEqual({uiScale: 90});
        expect(withoutUiFields({serverUrl: "http://x", theme: "nord", uiScale: 90})).toEqual({serverUrl: "http://x"});
    });

    it("migrates only the keys and column members a blob actually holds, cleaned", () => {
        const blob = {serverUrl: "http://x", serverToken: "t", insightsOpen: false, matchWindow: 90, columns: {status: false, bogus: true}, retiredKey: 1};
        expect(legacyUiPatch(blob)).toEqual({insightsOpen: false, matchWindow: 60, columns: {status: false}});
        expect(legacyUiPatch({serverUrl: "http://x"})).toEqual({});
        expect(legacyUiPatch({columns: {}})).toEqual({});
    });

    it("keeps a blob's null projection id as an explicit none", () => {
        expect(legacyUiPatch({lastProjectionId: null})).toEqual({lastProjectionId: null});
    });

    it("strips every UI field from a blob and keeps the rest", () => {
        const blob = {serverUrl: "http://x", serverToken: "t", insightsOpen: false, columns: {}, cashFlowChartMode: "source"};
        expect(withoutUiFields(blob)).toEqual({serverUrl: "http://x", serverToken: "t", cashFlowChartMode: "source"});
    });

    it("leaves the ledger's own keys out of a migration, column member by member", () => {
        const patch = {insightsOpen: false, matchWindow: 60, columns: {status: false, date: false}};
        expect(withoutLocalKeys(patch, ["matchWindow", "columns.status"])).toEqual({insightsOpen: false, columns: {date: false}});
        expect(withoutLocalKeys(patch, ["columns.status", "columns.date"])).toEqual({insightsOpen: false, matchWindow: 60});
        expect(withoutLocalKeys(patch, [])).toEqual(patch);
    });

    it("applies a patch to a full set of prefs, column member by member", () => {
        const applied = applyPatch(defaultUiPrefs(), {matchWindow: 6, columns: {amount: false}});
        expect(applied.matchWindow).toBe(6);
        expect(applied.columns).toEqual({...defaultUiPrefs().columns, amount: false});
    });

    it("merges pending saves key by key and column member by member", () => {
        const merged = mergePatch({matchWindow: 5, columns: {date: false}}, {matchWindow: 6, columns: {amount: false}, reviewMarkers: false});
        expect(merged).toEqual({matchWindow: 6, columns: {date: false, amount: false}, reviewMarkers: false});
        expect(mergePatch({}, {insightsOpen: true})).toEqual({insightsOpen: true});
    });
});
