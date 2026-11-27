// Plan 47's additions to the holdings wire: the series' y-axis `anchor`, the
// `liveAt` every holdings response carries, and `POST /api/prices/intraday`.
// Its own file, like the benchmark decoder's tests, to keep
// `nativeDecode.test.ts`'s churn down.
//
// The engine ALWAYS sends `anchor` and `liveAt` (null when it has nothing), so
// a missing key is a shape error: that is what lets the golden sweep notice a
// rename of a field whose golden value is null. Decimals are the wire's
// `{mantissa, places}` Dec, like every other money field.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeHoldingsReport, decodeHoldingsSeries, decodeIntradayResponse, decodeOtherHoldingsReport} from "./nativeDecode";

function golden(name: string): Record<string, unknown> {
    return JSON.parse(readFileSync(new URL(`../../../../fixtures/native/v1/${name}.json`, import.meta.url), "utf8")) as Record<string, unknown>;
}

function without(raw: Record<string, unknown>, key: string): Record<string, unknown> {
    const rest = {...raw};
    delete rest[key];
    return rest;
}

const dec = (mantissa: string, places: number) => ({mantissa, places});

describe("UNIT nativeDecode — holdings series anchor (plan 47 D1)", () => {
    const series = golden("holdings-series");

    it("decodes the golden's null anchor and liveAt as null", () => {
        const decoded = decodeHoldingsSeries(series);
        expect(decoded.anchor).toBeNull();
        expect(decoded.liveAt).toBeNull();
    });

    it.each(["anchor", "liveAt"])("refuses a series without %s (a renamed key must not be absorbed)", (key) => {
        expect(() => decodeHoldingsSeries(without(series, key))).toThrow(ApiShapeError);
    });

    it("decodes the wire Decs to display numbers", () => {
        const decoded = decodeHoldingsSeries({...series, anchor: {min: dec("9039460", 2), max: dec("12000000", 2)}, liveAt: 1_795_000_000});
        expect(decoded.anchor).toEqual({min: 90394.6, max: 120000});
        expect(decoded.liveAt).toBe(1_795_000_000);
    });

    it.each([
        ["a bare decimal string", {min: "1", max: "2"}],
        ["a missing bound", {min: dec("1", 0)}],
        ["a non-object", "1..2"],
    ])("refuses %s", (_, anchor) => {
        expect(() => decodeHoldingsSeries({...series, anchor})).toThrow(ApiShapeError);
    });

    it("refuses a liveAt that is not unix seconds", () => {
        expect(() => decodeHoldingsSeries({...series, liveAt: "2:35 PM"})).toThrow(ApiShapeError);
    });
});

// Amendment A1's additions, on LITERALS (the goldens gain them with the engine
// change): each point's `flow` (a Dec) and `returnPct` (a fraction or null),
// and the series' `returnAnchor` (fractions or null). All always present.
describe("UNIT nativeDecode — holdings series flow, returnPct and returnAnchor (plan 47 A1)", () => {
    const POINT = {
        date: "2026-05-31",
        bucket: "2026-05",
        label: "May 2026",
        marketValue: dec("10045300", 3),
        basis: null,
        flow: dec("1200000", 2),
        returnPct: 0.123,
    };
    const SERIES = {base: "$", points: [POINT], hasBasis: false, anchor: null, returnAnchor: {min: -0.05, max: 0.2}, liveAt: null};

    it("decodes the flow as a Dec and the returns as plain fractions", () => {
        const decoded = decodeHoldingsSeries(SERIES);
        expect(decoded.points[0].flow).toEqual({m: 1200000n, p: 2});
        expect(decoded.points[0].returnPct).toBe(0.123);
        expect(decoded.returnAnchor).toEqual({min: -0.05, max: 0.2});
    });

    it("keeps a null return and a null returnAnchor null", () => {
        const decoded = decodeHoldingsSeries({...SERIES, points: [{...POINT, returnPct: null}], returnAnchor: null});
        expect(decoded.points[0].returnPct).toBeNull();
        expect(decoded.returnAnchor).toBeNull();
    });

    it.each(["flow", "returnPct"])("refuses a point without %s (a renamed key must not be absorbed)", (key) => {
        expect(() => decodeHoldingsSeries({...SERIES, points: [without(POINT, key)]})).toThrow(ApiShapeError);
    });

    it("refuses a series without returnAnchor", () => {
        expect(() => decodeHoldingsSeries(without(SERIES, "returnAnchor"))).toThrow(ApiShapeError);
    });

    it.each([
        ["a flow that is a bare number", {flow: 12000}],
        ["a return that is a string", {returnPct: "12.3%"}],
        ["a return that is not finite", {returnPct: Number.NaN}],
    ])("refuses %s", (_, bad) => {
        expect(() => decodeHoldingsSeries({...SERIES, points: [{...POINT, ...bad}]})).toThrow(ApiShapeError);
    });

    it.each([
        ["Dec bounds (returns are fractions, not money)", {min: dec("1", 2), max: dec("2", 2)}],
        ["a missing bound", {min: 0}],
        ["a non-object", "0..1"],
    ])("refuses a returnAnchor of %s", (_, returnAnchor) => {
        expect(() => decodeHoldingsSeries({...SERIES, returnAnchor})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — holdings reports' liveAt (plan 47 D10)", () => {
    it("is null when null, the number when sent, and an error when missing, on both tabs' reports", () => {
        const stocks = golden("holdings");
        const other = golden("holdings-other");
        expect(decodeHoldingsReport(stocks).liveAt).toBeNull();
        expect(decodeHoldingsReport({...stocks, liveAt: 1_795_000_000}).liveAt).toBe(1_795_000_000);
        expect(() => decodeHoldingsReport(without(stocks, "liveAt"))).toThrow(ApiShapeError);
        expect(decodeOtherHoldingsReport(other).liveAt).toBeNull();
        expect(decodeOtherHoldingsReport({...other, liveAt: 1_795_000_000}).liveAt).toBe(1_795_000_000);
        expect(() => decodeOtherHoldingsReport(without(other, "liveAt"))).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — POST /api/prices/intraday (plan 47 D11)", () => {
    it("decodes quotes, skipped symbols and the rate-limit flag", () => {
        const decoded = decodeIntradayResponse({
            quotes: [
                {symbol: "VTI", price: dec("30125", 2), time: 1_795_000_000, currency: "USD"},
                {symbol: "FXAIX", price: dec("2101", 1), time: 1_795_000_000, currency: null},
            ],
            skipped: ["AAPL"],
            rateLimited: true,
        });
        expect(decoded).toEqual({
            quotes: [
                {symbol: "VTI", price: 301.25, time: 1_795_000_000, currency: "USD"},
                {symbol: "FXAIX", price: 210.1, time: 1_795_000_000, currency: null},
            ],
            skipped: ["AAPL"],
            rateLimited: true,
        });
    });

    it("reads an empty round as nothing to reload", () => {
        expect(decodeIntradayResponse({quotes: [], skipped: [], rateLimited: false})).toEqual({quotes: [], skipped: [], rateLimited: false});
    });

    it("refuses a reply without a quotes array, or a quote it cannot read", () => {
        expect(() => decodeIntradayResponse({skipped: []})).toThrow(ApiShapeError);
        expect(() => decodeIntradayResponse({quotes: [{symbol: "VTI", price: "301.25", time: 1, currency: "USD"}]})).toThrow(ApiShapeError);
    });
});
