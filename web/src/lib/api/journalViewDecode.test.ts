// The journal-view decoders: strict where a missing field would render as a
// quiet zero (cash balances, decimals), lenient per entry where a finding is
// advisory (diagnostics — a junk entry costs that one finding, never the badge).

import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeCashBalances, decodeDiagnostics, decodeViewDec, decodeViewStyle} from "./journalViewDecode";

const STYLE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

describe("UNIT decodeViewDec / decodeViewStyle", () => {
    it("keeps every digit of a mantissa past 2^53", () => {
        expect(decodeViewDec({m: "-123456789012345678901", p: 3}, "x")).toEqual({m: -123456789012345678901n, p: 3});
    });

    it("refuses a numeric or non-integer mantissa and bad places", () => {
        expect(() => decodeViewDec({m: 5, p: 0}, "x")).toThrow(ApiShapeError);
        expect(() => decodeViewDec({m: "1.5", p: 0}, "x")).toThrow(ApiShapeError);
        expect(() => decodeViewDec({m: "1", p: -1}, "x")).toThrow(ApiShapeError);
        expect(() => decodeViewDec(null, "x")).toThrow(ApiShapeError);
    });

    it("decodes a style, including no grouping", () => {
        expect(decodeViewStyle(STYLE, "s")).toEqual({side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]});
        expect(decodeViewStyle({...STYLE, digitGroups: null}, "s").digitGroups).toBeNull();
        expect(() => decodeViewStyle({...STYLE, side: "X"}, "s")).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeCashBalances", () => {
    const summary = {
        commodity: "$",
        style: STYLE,
        cash: [{account: "assets:bank", qty: {m: "100", p: 2}, asOf: "2026-01-02", confirmed: true}],
        liabilities: [],
        totalCash: {m: "100", p: 2},
        totalLiabilities: {m: "0", p: 0},
        net: {m: "100", p: 2},
    };
    const body = {asOf: "2026-09-13", commodities: ["$"], summaries: [summary], guessedLongTerm: ["liabilities:mortgage"]};

    it("decodes the whole view", () => {
        const view = decodeCashBalances(body);
        expect(view.commodities).toEqual(["$"]);
        expect(view.summaries[0].cash[0]).toEqual({account: "assets:bank", qty: {m: 100n, p: 2}, asOf: "2026-01-02", confirmed: true});
        expect(view.guessedLongTerm).toEqual(["liabilities:mortgage"]);
    });

    it("refuses a missing total rather than reading it as zero", () => {
        const partial: Record<string, unknown> = {...summary};
        delete partial.net;
        expect(() => decodeCashBalances({...body, summaries: [partial]})).toThrow(ApiShapeError);
    });

    it("refuses summaries that do not line up with the commodities", () => {
        expect(() => decodeCashBalances({...body, commodities: ["$", "EUR"]})).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeDiagnostics", () => {
    const pending = {
        txnIndex: 2,
        tindex: 3,
        date: "2025-01-05",
        description: "Grocery",
        rule: "pending",
        severity: "warning",
        message: "transaction is marked pending (!)",
    };
    const negative = {
        account: "assets:bank:checking",
        rule: "negative-cash",
        severity: "warning",
        message: "cash balance is negative: $ -5.00 as of 2025-01-17",
    };
    const envelope = (diagnostics: unknown[]): unknown => ({count: diagnostics.length, maxSeverity: "warning", diagnostics});

    it("maps a transaction finding to its 1-based tindex and keeps its date and description", () => {
        const {problems} = decodeDiagnostics(envelope([pending]));
        expect(problems).toEqual([{txnIndex: 3, date: "2025-01-05", description: "Grocery", rule: "pending", severity: "warning", message: pending.message}]);
    });

    it("keeps an account-anchored finding with a null txnIndex", () => {
        expect(decodeDiagnostics(envelope([negative])).problems).toEqual([{txnIndex: null, ...negative}]);
    });

    it("takes count and maxSeverity from the envelope, so a truncated list still drives the badge", () => {
        const payload = decodeDiagnostics({count: 21_429, maxSeverity: "error", diagnostics: []});
        expect(payload.count).toBe(21_429);
        expect(payload.maxSeverity).toBe("error");
        expect(payload.problems).toEqual([]);
    });

    it("refuses an envelope that is not the engine's", () => {
        expect(() => decodeDiagnostics({diagnostics: []})).toThrow(ApiShapeError);
        expect(() => decodeDiagnostics({count: 0, maxSeverity: "fatal", diagnostics: []})).toThrow(ApiShapeError);
        expect(() => decodeDiagnostics([])).toThrow(ApiShapeError);
    });

    it("skips junk entries but keeps the good ones around them", () => {
        const {problems} = decodeDiagnostics(
            envelope([
                pending,
                {...pending, severity: "critical"},
                {...pending, message: "  "},
                {...pending, rule: ""},
                {...pending, tindex: 0},
                {...pending, tindex: 1.5},
                {rule: "x", severity: "info", message: "no anchor"},
                {...negative, account: "  "},
                null,
                3,
                negative,
            ])
        );
        expect(problems.map((p) => p.rule)).toEqual(["pending", "negative-cash"]);
    });

    it("refuses a junk tindex rather than demoting it to an account anchor", () => {
        expect(decodeDiagnostics(envelope([{...negative, tindex: -4}])).problems).toEqual([]);
    });

    it("accepts a rule it has never heard of: the engine is the only producer", () => {
        expect(decodeDiagnostics(envelope([{...pending, rule: "brand-new"}])).problems).toHaveLength(1);
    });

    it("collapses exact duplicates (the drawer keys its rows on anchor + message)", () => {
        expect(decodeDiagnostics(envelope([pending, {...pending}, negative])).problems).toHaveLength(2);
    });
});
