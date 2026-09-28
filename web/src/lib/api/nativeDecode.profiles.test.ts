// `decodeHoldingsProfiles` — `GET /api/holdings/profiles`. Its own file rather
// than another block in `nativeDecode.test.ts` only to keep that file's churn
// down; the rules are the same: decode what the engine sends, refuse what it
// could not have.

import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeHoldingsProfiles} from "./nativeDecode";

/** Exactly what `profiles_endpoints.rs` sees the engine answer for a tagged, Yahoo-classified fund. */
const WIRE = {
    yahoo: "partial",
    profiles: [
        {
            symbol: "BAL",
            yahooTicker: "VBAL-X",
            source: "mixed",
            fetchedAt: "2026-09-28",
            breakdown: {
                assetClass: [
                    {label: "Equity", weight: 0.6},
                    {label: "Bonds", weight: 0.4},
                ],
                sector: [{label: "Diversified", weight: 1}],
                industry: [],
                securityType: [{label: "Mutual fund", weight: 1}],
                category: [{label: "Moderate Allocation", weight: 1}],
                risk: [{label: "Below average", weight: 1}],
            },
        },
        {
            symbol: "PRIVATE",
            yahooTicker: "PRIVATE",
            source: "none",
            breakdown: {assetClass: [], sector: [], industry: [], securityType: [], category: [], risk: []},
        },
    ],
};

describe("UNIT decodeHoldingsProfiles", () => {
    it("decodes every dimension, keyed by symbol", () => {
        const decoded = decodeHoldingsProfiles(WIRE);
        expect(decoded.yahoo).toBe("partial");
        expect([...decoded.profiles.keys()]).toEqual(["BAL", "PRIVATE"]);
        const bal = decoded.profiles.get("BAL");
        expect(bal?.yahooTicker).toBe("VBAL-X");
        expect(bal?.source).toBe("mixed");
        expect(bal?.fetchedAt).toBe("2026-09-28");
        expect(bal?.breakdown.assetClass).toEqual([
            {label: "Equity", weight: 0.6},
            {label: "Bonds", weight: 0.4},
        ]);
        expect(bal?.breakdown.risk).toEqual([{label: "Below average", weight: 1}]);
    });

    it("reads an absent fetchedAt as null", () => {
        expect(decodeHoldingsProfiles(WIRE).profiles.get("PRIVATE")?.fetchedAt).toBeNull();
    });

    it("reads a dimension an older engine did not send as nothing known", () => {
        const older = {yahoo: "ok", profiles: [{symbol: "A", yahooTicker: "A", source: "tags", breakdown: {sector: [{label: "X", weight: 1}]}}]};
        const decoded = decodeHoldingsProfiles(older).profiles.get("A");
        expect(decoded?.breakdown.risk).toEqual([]);
        expect(decoded?.breakdown.sector).toEqual([{label: "X", weight: 1}]);
    });

    it("freezes what it returns", () => {
        const bal = decodeHoldingsProfiles(WIRE).profiles.get("BAL");
        expect(Object.isFrozen(bal)).toBe(true);
        expect(Object.isFrozen(bal?.breakdown.sector)).toBe(true);
    });

    it.each([
        ["a missing profiles array", {yahoo: "ok"}],
        ["an unknown yahoo status", {yahoo: "maybe", profiles: []}],
        ["an unknown source", {yahoo: "ok", profiles: [{...WIRE.profiles[1], source: "guess"}]}],
        ["a missing breakdown", {yahoo: "ok", profiles: [{symbol: "A", yahooTicker: "A", source: "none"}]}],
        ["a weight above one", {yahoo: "ok", profiles: [{...WIRE.profiles[1], breakdown: {sector: [{label: "X", weight: 1.5}]}}]}],
        ["a non-numeric weight", {yahoo: "ok", profiles: [{...WIRE.profiles[1], breakdown: {sector: [{label: "X", weight: "1"}]}}]}],
        ["a missing label", {yahoo: "ok", profiles: [{...WIRE.profiles[1], breakdown: {sector: [{weight: 1}]}}]}],
    ])("refuses %s", (_name, body) => {
        expect(() => decodeHoldingsProfiles(body)).toThrow(ApiShapeError);
    });
});
