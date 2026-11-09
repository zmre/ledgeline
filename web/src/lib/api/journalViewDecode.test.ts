// The journal-view decoders: strict where a missing field would render as a
// quiet zero (cash balances, decimals), lenient per entry where a finding is
// advisory (diagnostics — a junk entry costs that one finding, never the badge).

import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {balanceSeriesWire} from "#lib/testing/balanceSeriesFixture.js";
import {
    decodeBalanceSeries,
    decodeCashBalances,
    decodeDiagnostics,
    decodeRow,
    decodeRunningCell,
    decodeRunningInfo,
    decodeViewDec,
    decodeViewStyle,
} from "./journalViewDecode";

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

// ---------------------------------------------------------------------------
// Running balances (plan 36)
// ---------------------------------------------------------------------------

const wireUsd = (m: string): Record<string, unknown> => ({commodity: "$", qty: {m, p: 2}, style: STYLE});

const RUNNING = {
    change: wireUsd("-25000"),
    balance: wireUsd("1240000"),
    touches: true,
    postedOn: "2026-04-02",
    assertions: [
        {account: "assets:checking", op: "=", asserted: wireUsd("1240000"), ok: true},
        {account: "assets:checking:joint", op: "==*", asserted: wireUsd("500000"), calculated: wireUsd("475000"), ok: false},
    ],
};

describe("UNIT decodeRunningCell", () => {
    it("decodes change, balance, the posting date and both kinds of mark", () => {
        const cell = decodeRunningCell(RUNNING);
        expect(cell.change?.qty).toEqual({m: -25000n, p: 2});
        expect(cell.balance.qty).toEqual({m: 1240000n, p: 2});
        expect(cell.touches).toBe(true);
        expect(cell.postedOn).toBe("2026-04-02");
        expect(cell.assertions[0]).toMatchObject({account: "assets:checking", op: "=", ok: true, calculated: null});
        expect(cell.assertions[1].calculated?.qty).toEqual({m: 475000n, p: 2});
        expect(Object.isFrozen(cell)).toBe(true);
    });

    it("reads a carried row: no change, no posting date", () => {
        const cell = decodeRunningCell({change: null, balance: wireUsd("100"), touches: false, assertions: []});
        expect(cell.change).toBeNull();
        expect(cell.postedOn).toBeNull();
    });

    it("refuses a missing change rather than reading it as carried", () => {
        const rest: Record<string, unknown> = {...RUNNING};
        delete rest.change;
        expect(() => decodeRunningCell(rest)).toThrow(ApiShapeError);
    });

    it("refuses a missing balance, a non-boolean touches, a bad date, an unknown operator", () => {
        expect(() => decodeRunningCell({...RUNNING, balance: undefined})).toThrow(ApiShapeError);
        expect(() => decodeRunningCell({...RUNNING, touches: "yes"})).toThrow(ApiShapeError);
        expect(() => decodeRunningCell({...RUNNING, postedOn: "04/02/2026"})).toThrow(ApiShapeError);
        expect(() => decodeRunningCell({...RUNNING, assertions: [{...RUNNING.assertions[0], op: "~"}]})).toThrow(ApiShapeError);
        expect(() => decodeRunningCell({...RUNNING, assertions: null})).toThrow(ApiShapeError);
    });

    it("refuses a failure that does not say what it found", () => {
        const bare: Record<string, unknown> = {...RUNNING.assertions[1]};
        delete bare.calculated;
        expect(() => decodeRunningCell({...RUNNING, assertions: [bare]})).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeRow running", () => {
    const row = {index: 7, date: "2026-03-28", status: "unmarked", description: "x", code: "", comment: "", tags: [], postings: [], flags: []};

    it("is null when the request pinned nothing", () => {
        expect(decodeRow(row).running).toBeNull();
    });

    it("is the decoded cell when present, and a bad one fails the row", () => {
        expect(decodeRow({...row, running: RUNNING}).running?.postedOn).toBe("2026-04-02");
        expect(() => decodeRow({...row, running: {...RUNNING, balance: null}})).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeRunningInfo", () => {
    const body = {
        generation: "g7",
        account: "assets:broker",
        commodities: [
            {commodity: "$", postings: 412},
            {commodity: "VTI", postings: 37},
        ],
        assertions: {
            $: [
                {tindex: 3, date: "2026-01-31", ok: true},
                {tindex: 812, date: "2026-03-31", ok: false},
            ],
            VTI: [],
        },
    };

    it("decodes commodities in the engine's order and the assertions per commodity", () => {
        const info = decodeRunningInfo(body);
        expect(info.commodities.map((c) => c.commodity)).toEqual(["$", "VTI"]);
        expect(info.assertions.$).toEqual([
            {tindex: 3, date: "2026-01-31", ok: true},
            {tindex: 812, date: "2026-03-31", ok: false},
        ]);
        expect(info.assertions.VTI).toEqual([]);
        expect(Object.isFrozen(info.assertions)).toBe(true);
    });

    it("refuses a malformed body", () => {
        expect(() => decodeRunningInfo({...body, generation: undefined})).toThrow(ApiShapeError);
        expect(() => decodeRunningInfo({...body, commodities: [{commodity: "$", postings: -1}]})).toThrow(ApiShapeError);
        expect(() => decodeRunningInfo({...body, assertions: []})).toThrow(ApiShapeError);
        expect(() => decodeRunningInfo({...body, assertions: {$: [{tindex: 0, date: "2026-01-31", ok: true}]}})).toThrow(ApiShapeError);
        expect(() => decodeRunningInfo({...body, assertions: {$: [{tindex: 3, date: "2026-01-31"}]}})).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeBalanceSeries", () => {
    it("decodes the window, the points and a period's failures", () => {
        const series = decodeBalanceSeries(balanceSeriesWire());
        expect(series.account).toBe("assets:bank");
        expect(series.commodity).toBe("$");
        expect(series.interval).toBe("weekly");
        expect({from: series.from, to: series.to}).toEqual({from: "2026-01-01", to: "2026-02-03"});
        expect(series.style.precision).toBe(2);
        expect(series.points).toHaveLength(6);
        expect(series.points[0]).toEqual({bucket: "2026-W01", label: "W01 2026", end: "2026-01-04", balance: {m: 600_000n, p: 2}, tindex: 1, failures: []});
        const failure = series.points[4].failures[0];
        expect(failure).toEqual({
            tindex: 11,
            date: "2026-01-31",
            account: "assets:bank:checking",
            op: "=",
            asserted: {m: 30_000n, p: 2},
            calculated: {m: 22_025n, p: 2},
            expected: {m: 548_200n, p: 2},
        });
        // Omitted, not empty: no posting in the period means nothing to scroll to.
        expect(series.points[5].tindex).toBeNull();
        expect(Object.isFrozen(series.points)).toBe(true);
    });

    it("refuses a malformed body", () => {
        const wire = balanceSeriesWire();
        const points = wire.points as Record<string, unknown>[];
        const withPoint = (patch: Record<string, unknown>) => balanceSeriesWire({points: [{...points[0], ...patch}]});
        expect(() => decodeBalanceSeries({...wire, generation: undefined})).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries({...wire, interval: "fortnightly"})).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries({...wire, from: "2026-03-01"})).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries({...wire, to: "Feb 3"})).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries({...wire, points: {}})).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries(withPoint({balance: 6000}))).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries(withPoint({tindex: 0}))).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries(withPoint({end: undefined}))).toThrow(ApiShapeError);
        const failure = (points[4].failures as Record<string, unknown>[])[0];
        expect(() => decodeBalanceSeries(withPoint({failures: [{...failure, op: "~"}]}))).toThrow(ApiShapeError);
        // A failure must say what it found and where the line should be.
        expect(() => decodeBalanceSeries(withPoint({failures: [{...failure, calculated: undefined}]}))).toThrow(ApiShapeError);
        expect(() => decodeBalanceSeries(withPoint({failures: [{...failure, expected: undefined}]}))).toThrow(ApiShapeError);
    });
});
