import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeHoldingsReport, decodeHoldingsSeries, decodePeriodReport, decodeSectionedReport} from "./nativeDecode";

// Native-shape samples mirror crates/ledgeline-server/src/reports_api.rs and
// were captured live from ledgeline-server against fixtures/sample.journal.

const dec = (mantissa: number, places: number) => ({mantissa, places});

describe("UNIT nativeDecode — SectionedReport", () => {
    const raw = {
        asOf: "2026-07-08",
        sections: [
            {
                title: "Assets",
                rows: [
                    {account: "assets", depth: 1, own: {}, inclusive: {$: dec(4840256, 2), AAPL: dec(195, 1)}},
                    {account: "assets:bank", depth: 2, own: {}, inclusive: {$: dec(4179281, 2)}},
                ],
                total: {$: dec(4840256, 2), AAPL: dec(195, 1)},
            },
            {title: "Liabilities", rows: [{account: "liabilities", depth: 1, own: {}, inclusive: {$: dec(53115, 2)}}], total: {$: dec(53115, 2)}},
        ],
        grandTotal: {$: dec(4787141, 2), AAPL: dec(195, 1)},
    };

    it("decodes sections, rows, and exact Dec/MixedAmount, preserving asOf", () => {
        const report = decodeSectionedReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.from).toBeUndefined();
        expect(report.sections).toHaveLength(2);

        const assets = report.sections[0];
        expect(assets.title).toBe("Assets");
        expect(assets.rows[0]).toMatchObject({account: "assets", depth: 1});
        // {mantissa, places} → Dec {m: bigint, p: number}
        expect(assets.rows[0].inclusive.get("$")).toEqual({m: 4840256n, p: 2});
        expect(assets.rows[0].inclusive.get("AAPL")).toEqual({m: 195n, p: 1});
        // empty own object → empty Map
        expect(assets.rows[0].own.size).toBe(0);
        expect(assets.total.get("$")).toEqual({m: 4840256n, p: 2});

        expect(report.grandTotal.get("$")).toEqual({m: 4787141n, p: 2});
    });

    it("carries from/to on a range report (income statement)", () => {
        const report = decodeSectionedReport({from: "2026-01-01", to: "2026-07-08", sections: [], grandTotal: {}});
        expect(report.from).toBe("2026-01-01");
        expect(report.to).toBe("2026-07-08");
        expect(report.asOf).toBeUndefined();
        expect(report.grandTotal.size).toBe(0);
    });

    it("throws ApiShapeError when sections is missing", () => {
        expect(() => decodeSectionedReport({grandTotal: {}})).toThrow(ApiShapeError);
    });

    it("guards the mantissa safe-integer range", () => {
        const bad = {
            // 2^53: exactly representable as a JS number, but NOT a safe integer (bigger than MAX_SAFE_INTEGER).
            sections: [{title: "X", rows: [{account: "a", depth: 1, own: {}, inclusive: {$: {mantissa: 9007199254740992, places: 2}}}], total: {}}],
            grandTotal: {},
        };
        expect(() => decodeSectionedReport(bad)).toThrow(/safe integer range/);
    });

    it("rejects a negative places value", () => {
        const bad = {sections: [], grandTotal: {$: {mantissa: 100, places: -1}}};
        expect(() => decodeSectionedReport(bad)).toThrow(/invalid places/);
    });
});

describe("UNIT nativeDecode — PeriodReport", () => {
    const raw = {
        buckets: ["2026-05", "2026-06", "2026-07"],
        rows: [
            {account: "assets", depth: 1, values: [{$: dec(138878, 2), EUR: dec(50000, 2)}, {$: dec(630, 0)}, {}]},
            {account: "assets:broker", depth: 2, values: [{$: dec(2500, 2)}, {}, {}]},
        ],
        totals: [{$: dec(138878, 2)}, {$: dec(242512, 2)}, {}],
    };

    it("decodes buckets, per-bucket MixedAmounts, and totals", () => {
        const report = decodePeriodReport(raw);
        expect(report.buckets).toEqual(["2026-05", "2026-06", "2026-07"]);
        expect(report.rows).toHaveLength(2);
        expect(report.rows[0].values[0].get("$")).toEqual({m: 138878n, p: 2});
        expect(report.rows[0].values[0].get("EUR")).toEqual({m: 50000n, p: 2});
        // places 0 stays places 0
        expect(report.rows[0].values[1].get("$")).toEqual({m: 630n, p: 0});
        // empty bucket object → empty Map
        expect(report.rows[0].values[2].size).toBe(0);
        expect(report.totals[2].size).toBe(0);
        expect(report.meta).toBeUndefined();
    });

    it("decodes the net-worth meta.unpriced list", () => {
        const report = decodePeriodReport({buckets: ["2026-07"], rows: [], totals: [{$: dec(1, 0)}], meta: {unpriced: ["GLD", "TSLA"]}});
        expect(report.meta?.unpriced).toEqual(["GLD", "TSLA"]);
    });

    it("throws ApiShapeError when totals is missing", () => {
        expect(() => decodePeriodReport({buckets: [], rows: []})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — HoldingsReport", () => {
    const raw = {
        asOf: "2026-07-08",
        base: "$",
        holdings: [
            {
                symbol: "AAPL",
                name: "Apple Inc.",
                accounts: ["assets:broker:taxable:aapl"],
                shares: dec(195, 1),
                basis: dec(4346100, 3),
                firstBasisDate: "2024-09-16",
                price: {qty: dec(27025, 2), date: "2026-06-30", source: "directive"},
                marketValue: dec(5269875, 3),
                gain: dec(923775, 3),
                gainPct: 21.255263339545795,
            },
            {
                symbol: "GLD",
                name: "GLD",
                accounts: ["assets:broker:taxable:gld"],
                shares: dec(5, 0),
                basis: null,
                firstBasisDate: "2025-08-20",
                price: null,
                marketValue: null,
                gain: null,
                gainPct: null,
            },
        ],
        totals: {marketValue: dec(10552625, 3), basis: null, gain: null, gainPct: null},
        topGainers: [],
        topLosers: [],
        warnings: [
            {symbol: "GLD", kind: "unpriced", message: "GLD: no market price or usable cost annotation — excluded from totals"},
            {symbol: "TSLA", kind: "negative-shares", message: "TSLA: net shares are negative — row hidden"},
        ],
    };

    it("decodes priced and tainted holdings, keeping nulls", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.base).toBe("$");

        const aapl = report.holdings[0];
        expect(aapl.symbol).toBe("AAPL");
        expect(aapl.name).toBe("Apple Inc.");
        expect(aapl.accounts).toEqual(["assets:broker:taxable:aapl"]);
        expect(aapl.shares).toEqual({m: 195n, p: 1});
        expect(aapl.basis).toEqual({m: 4346100n, p: 3});
        expect(aapl.firstBasisDate).toBe("2024-09-16");
        expect(aapl.price).toEqual({qty: {m: 27025n, p: 2}, date: "2026-06-30", source: "directive"});
        expect(aapl.marketValue).toEqual({m: 5269875n, p: 3});
        expect(aapl.gain).toEqual({m: 923775n, p: 3});
        expect(aapl.gainPct).toBeCloseTo(21.2552633, 6);

        const gld = report.holdings[1];
        expect(gld.basis).toBeNull();
        expect(gld.price).toBeNull();
        expect(gld.marketValue).toBeNull();
        expect(gld.gain).toBeNull();
        expect(gld.gainPct).toBeNull();
    });

    it("keeps null portfolio totals (honest-totals rule) with a present market value", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.totals.marketValue).toEqual({m: 10552625n, p: 3});
        expect(report.totals.basis).toBeNull();
        expect(report.totals.gain).toBeNull();
        expect(report.totals.gainPct).toBeNull();
    });

    it("decodes the warning union", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.warnings.map((w) => w.kind)).toEqual(["unpriced", "negative-shares"]);
    });

    it("rejects an unknown warning kind", () => {
        const bad = {...raw, warnings: [{symbol: "X", kind: "surprise", message: "?"}]};
        expect(() => decodeHoldingsReport(bad)).toThrow(/unknown warning kind/);
    });

    it("rejects an unknown price source", () => {
        const bad = {...raw, holdings: [{...raw.holdings[0], price: {qty: dec(1, 0), date: "2026-01-01", source: "guess"}}]};
        expect(() => decodeHoldingsReport(bad)).toThrow(/unknown price source/);
    });

    it("throws ApiShapeError when base is missing", () => {
        expect(() => decodeHoldingsReport({asOf: "2026-07-08", holdings: [], totals: {marketValue: dec(0, 0)}})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — HoldingsSeries", () => {
    const raw = {
        base: "$",
        points: [
            {date: "2026-05-31", bucket: "2026-05", label: "May 2026", marketValue: dec(10045300, 3), basis: null},
            {date: "2026-06-30", bucket: "2026-06", label: "Jun 2026", marketValue: dec(10552625, 3), basis: dec(500000, 2)},
        ],
        hasBasis: true,
    };

    it("decodes points with labels and nullable basis", () => {
        const series = decodeHoldingsSeries(raw);
        expect(series.base).toBe("$");
        expect(series.hasBasis).toBe(true);
        expect(series.points).toHaveLength(2);
        expect(series.points[0]).toMatchObject({date: "2026-05-31", bucket: "2026-05", label: "May 2026"});
        expect(series.points[0].marketValue).toEqual({m: 10045300n, p: 3});
        expect(series.points[0].basis).toBeNull();
        expect(series.points[1].basis).toEqual({m: 500000n, p: 2});
    });

    it("throws ApiShapeError when points is missing", () => {
        expect(() => decodeHoldingsSeries({base: "$"})).toThrow(ApiShapeError);
    });
});
