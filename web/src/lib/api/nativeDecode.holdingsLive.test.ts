// Plan 47's additions to the holdings wire: the series' y-axis `anchor`, the
// `liveAt` every holdings response carries, and `POST /api/prices/intraday`.
// Its own file, like the benchmark decoder's tests, to keep
// `nativeDecode.test.ts`'s churn down.
//
// The new fields are OPTIONAL on the wire (an engine from before plan 47 sends
// none of them), so absent decodes exactly like null; but a field that IS sent
// must be the right shape.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeHoldingsReport, decodeHoldingsSeries, decodeIntradayResponse, decodeOtherHoldingsReport} from "./nativeDecode";

function golden(name: string): Record<string, unknown> {
    return JSON.parse(readFileSync(new URL(`../../../../fixtures/native/v1/${name}.json`, import.meta.url), "utf8")) as Record<string, unknown>;
}

/** The golden with plan 47's keys removed, as an engine from before it answers. */
function withoutPlan47(raw: Record<string, unknown>): Record<string, unknown> {
    const rest = {...raw};
    delete rest.anchor;
    delete rest.liveAt;
    return rest;
}

describe("UNIT nativeDecode — holdings series anchor (plan 47 D1)", () => {
    const series = golden("holdings-series");

    it("decodes an absent anchor and liveAt as null (an older engine)", () => {
        const decoded = decodeHoldingsSeries(withoutPlan47(series));
        expect(decoded.anchor).toBeNull();
        expect(decoded.liveAt).toBeNull();
    });

    it("decodes the contract's decimal strings to display numbers", () => {
        const decoded = decodeHoldingsSeries({...series, anchor: {min: "90394.60", max: "120000"}, liveAt: 1_795_000_000});
        expect(decoded.anchor).toEqual({min: 90394.6, max: 120000});
        expect(decoded.liveAt).toBe(1_795_000_000);
    });

    it("accepts the {mantissa, places} Dec the rest of the wire uses, too", () => {
        const decoded = decodeHoldingsSeries({...series, anchor: {min: {mantissa: "9039460", places: 2}, max: {mantissa: "12000000", places: 2}}});
        expect(decoded.anchor).toEqual({min: 90394.6, max: 120000});
    });

    it("keeps an explicit null anchor null (nothing > 0 in the horizon)", () => {
        expect(decodeHoldingsSeries({...series, anchor: null}).anchor).toBeNull();
    });

    it.each([
        ["a non-decimal bound", {min: "lots", max: "1"}],
        ["a missing bound", {min: "1"}],
        ["a non-object", "1..2"],
    ])("refuses %s", (_, anchor) => {
        expect(() => decodeHoldingsSeries({...series, anchor})).toThrow(ApiShapeError);
    });

    it("refuses a liveAt that is not unix seconds", () => {
        expect(() => decodeHoldingsSeries({...series, liveAt: "2:35 PM"})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — holdings reports' liveAt (plan 47 D10)", () => {
    it("is null when absent and the number when sent, on both tabs' reports", () => {
        const stocks = golden("holdings");
        const other = golden("holdings-other");
        expect(decodeHoldingsReport(withoutPlan47(stocks)).liveAt).toBeNull();
        expect(decodeHoldingsReport({...stocks, liveAt: 1_795_000_000}).liveAt).toBe(1_795_000_000);
        expect(decodeOtherHoldingsReport(withoutPlan47(other)).liveAt).toBeNull();
        expect(decodeOtherHoldingsReport({...other, liveAt: 1_795_000_000}).liveAt).toBe(1_795_000_000);
    });
});

describe("UNIT nativeDecode — POST /api/prices/intraday (plan 47 D11)", () => {
    it("decodes quotes, skipped symbols and the rate-limit flag", () => {
        const decoded = decodeIntradayResponse({
            quotes: [{symbol: "VTI", price: "301.25", time: 1_795_000_000, currency: "USD"}],
            skipped: ["AAPL"],
            rateLimited: true,
        });
        expect(decoded).toEqual({quotes: [{symbol: "VTI", price: 301.25, time: 1_795_000_000, currency: "USD"}], skipped: ["AAPL"], rateLimited: true});
    });

    it("reads an empty round as nothing to reload", () => {
        expect(decodeIntradayResponse({quotes: [], skipped: [], rateLimited: false})).toEqual({quotes: [], skipped: [], rateLimited: false});
    });

    it("refuses a reply without a quotes array, or a quote it cannot read", () => {
        expect(() => decodeIntradayResponse({skipped: []})).toThrow(ApiShapeError);
        expect(() => decodeIntradayResponse({quotes: [{symbol: "VTI", price: "x", time: 1, currency: "USD"}]})).toThrow(ApiShapeError);
    });
});
