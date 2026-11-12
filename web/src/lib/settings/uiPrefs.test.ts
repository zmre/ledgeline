// The pure half of the view-state move (plan 38 WP-5): reading an old
// localStorage blob, turning it into the one-time migration patch, and keeping
// pending saves to the keys that changed.

import {describe, expect, it} from "vitest";
import {cleanUiPrefs, defaultUiPrefs, legacyUiPatch, mergePatch, UI_PREF_KEYS, withoutUiFields} from "./uiPrefs";

describe("UNIT uiPrefs", () => {
    it("names exactly the nineteen fields the localStorage blob used to carry", () => {
        expect([...UI_PREF_KEYS].sort()).toEqual(Object.keys(defaultUiPrefs()).sort());
        expect(UI_PREF_KEYS).toHaveLength(19);
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

    it("merges pending saves key by key and column member by member", () => {
        const merged = mergePatch({matchWindow: 5, columns: {date: false}}, {matchWindow: 6, columns: {amount: false}, reviewMarkers: false});
        expect(merged).toEqual({matchWindow: 6, columns: {date: false, amount: false}, reviewMarkers: false});
        expect(mergePatch({}, {insightsOpen: true})).toEqual({insightsOpen: true});
    });
});
