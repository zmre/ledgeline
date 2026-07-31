import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {
    decodeBudgetReport,
    decodeHoldingsReport,
    decodeHoldingsSeries,
    decodeInsightsReport,
    decodePeriodReport,
    decodeRulesDoc,
    decodeRulesIndex,
    decodeRulesPreview,
    decodeSectionedReport,
    decodeSubscriptionsReport,
} from "./nativeDecode";

// The native wire has no codegen: these decoders mirror 28 `Wire*` structs in
// crates/ledgeline-server/src/reports_api.rs BY HAND (CLEANUP.md DRY-3).
//
// So the primary samples here are not literals — they are the SAME committed
// response bodies the Rust side asserts against, `fixtures/native/v1/*.json`,
// produced by `just snapshot-native` and byte-checked by
// crates/ledgeline-server/tests/native_wire_golden.rs. Renaming a Rust field
// therefore breaks the Rust golden AND these decode assertions at once, instead
// of quietly rendering $0.00 in the SPA.
//
// Hand-written literals are kept ONLY for the cases fixtures/sample.journal
// cannot produce (a mantissa past Number.MAX_SAFE_INTEGER, a malformed mantissa,
// an unknown enum member, net worth's `meta.unpriced`, refused holdings totals,
// a budget goal of `{}`). Each says which.

const dec = (mantissa: number, places: number) => ({mantissa: String(mantissa), places});

function golden(name: string): unknown {
    return JSON.parse(readFileSync(new URL(`../../../../fixtures/native/v1/${name}.json`, import.meta.url), "utf8"));
}

/**
 * The import-rules goldens, which live in their OWN directory.
 *
 * They cannot join `fixtures/native/v1/`: that manifest is replayed against
 * `fixtures/sample.journal` and guarded by a rule that every pinned URI must fix
 * its own dates, and a rules response has no dates in it at all. These are
 * served over `fixtures/rules/tree/main.journal` and byte-checked by
 * `crates/ledgeline-server/tests/rules_endpoints.rs`, so a renamed Rust field
 * fails there and here at once — the same DRY-3 guard, one directory over.
 */
function rulesGolden(name: string): unknown {
    return JSON.parse(readFileSync(new URL(`../../../../fixtures/rules/golden/${name}.json`, import.meta.url), "utf8"));
}

describe("UNIT nativeDecode — SectionedReport over the balancesheet golden", () => {
    const raw = golden("balancesheet");

    it("decodes sections, rows, and exact Dec/MixedAmount, preserving asOf", () => {
        const report = decodeSectionedReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.from).toBeUndefined();
        expect(report.sections.map((s) => s.title)).toEqual(["Assets", "Liabilities"]);

        const assets = report.sections[0];
        expect(assets.rows[0]).toMatchObject({account: "assets", depth: 1});
        // {mantissa, places} → Dec {m: bigint, p: number}
        expect(assets.rows[0].inclusive.get("$")).toEqual({m: 4840256n, p: 2});
        expect(assets.rows[0].inclusive.get("AAPL")).toEqual({m: 195n, p: 1});
        // places 0 stays places 0, and a negative mantissa survives
        expect(assets.rows[0].inclusive.get("TSLA")).toEqual({m: -2n, p: 0});
        // a parent account's OWN amount is an empty object → an empty Map
        expect(assets.rows[0].own.size).toBe(0);
        expect(assets.rows[1]).toMatchObject({account: "assets:bank", depth: 2});
        expect(assets.total.get("$")).toEqual({m: 4840256n, p: 2});

        expect(report.sections[1].total.get("$")).toEqual({m: 53115n, p: 2});
        expect(report.grandTotal.get("$")).toEqual({m: 4787141n, p: 2});
    });

    it("carries from/to on a range report (the incomestatement golden)", () => {
        const report = decodeSectionedReport(golden("incomestatement"));
        expect(report.from).toBe("2026-01-01");
        expect(report.to).toBe("2026-07-08");
        expect(report.asOf).toBeUndefined();

        const revenues = report.sections[0];
        expect(revenues.title).toBe("Revenues");
        expect(revenues.total.get("$")).toEqual({m: 3401000n, p: 2});
        // a LEAF row carries a non-empty `own` — the field the balance sheet golden
        // only ever shows empty.
        expect(revenues.rows[2]).toMatchObject({account: "income:salary", depth: 2});
        expect(revenues.rows[2].own.get("$")).toEqual({m: 3396000n, p: 2});
        expect(report.grandTotal.get("EUR")).toEqual({m: -22875n, p: 2});
    });

    it("throws ApiShapeError when sections is missing", () => {
        expect(() => decodeSectionedReport({grandTotal: {}})).toThrow(ApiShapeError);
    });

    // Literal: no realistic journal produces this, but a computed marketValue can.
    it("decodes a mantissa far beyond the JS safe-integer range (the marketValue overflow fix)", () => {
        // 9625405560255625000 > Number.MAX_SAFE_INTEGER; string-encoded on the wire
        // so it round-trips exactly via BigInt (a JSON number would have lost it).
        const big = {
            sections: [{title: "X", rows: [{account: "a", depth: 1, own: {}, inclusive: {$: {mantissa: "9625405560255625000", places: 15}}}], total: {}}],
            grandTotal: {},
        };
        const report = decodeSectionedReport(big);
        expect(report.sections[0].rows[0].inclusive.get("$")).toEqual({m: 9625405560255625000n, p: 15});
    });

    it("rejects a non-integer mantissa string", () => {
        const bad = {sections: [], grandTotal: {$: {mantissa: "1.5", places: 2}}};
        expect(() => decodeSectionedReport(bad)).toThrow(/not an integer/);
    });

    it("rejects a negative places value", () => {
        const bad = {sections: [], grandTotal: {$: {mantissa: "100", places: -1}}};
        expect(() => decodeSectionedReport(bad)).toThrow(/invalid places/);
    });
});

describe("UNIT nativeDecode — PeriodReport over the cashflow / networth goldens", () => {
    it("decodes buckets, per-bucket MixedAmounts, and totals", () => {
        const report = decodePeriodReport(golden("cashflow"));
        expect(report.buckets).toEqual(["2026-05", "2026-06", "2026-07"]);
        expect(report.rows.map((r) => r.account)).toEqual(["assets", "assets:bank", "assets:broker"]);
        expect(report.rows[0].values[0].get("$")).toEqual({m: 138878n, p: 2});
        expect(report.rows[0].values[0].get("EUR")).toEqual({m: 50000n, p: 2});
        expect(report.rows[0].values[2].get("$")).toEqual({m: -187500n, p: 2});
        // places 0 stays places 0
        expect(report.rows[2].values[1].get("$")).toEqual({m: 630n, p: 0});
        // a bucket with no activity is an empty object → an empty Map
        expect(report.rows[2].values[2].size).toBe(0);
        expect(report.totals[1].get("$")).toEqual({m: 242512n, p: 2});
        // sample.journal prices every commodity, so the server omits `meta` entirely
        // (reports_golden.rs asserts the same on the Rust side).
        expect(report.meta).toBeUndefined();
    });

    it("decodes market-valued net worth at full computed precision", () => {
        const report = decodePeriodReport(golden("networth"));
        expect(report.rows[0]).toMatchObject({account: "assets", depth: 1});
        // 6 decimal places: a computed value, not a parsed one.
        expect(report.rows[0].values[0].get("$")).toEqual({m: 59729101000n, p: 6});
        expect(report.totals[2].get("$")).toEqual({m: 594514650n, p: 4});
    });

    // Literal: sample.journal leaves nothing unpriced, so `meta` never appears in
    // the goldens — but the field exists and the SPA renders it.
    it("decodes the net-worth meta.unpriced list", () => {
        const report = decodePeriodReport({buckets: ["2026-07"], rows: [], totals: [{$: dec(1, 0)}], meta: {unpriced: ["GLD", "TSLA"]}});
        expect(report.meta?.unpriced).toEqual(["GLD", "TSLA"]);
    });

    it("throws ApiShapeError when totals is missing", () => {
        expect(() => decodePeriodReport({buckets: [], rows: []})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — BudgetReport over the budget golden", () => {
    it("decodes buckets, cells, and the actual/goal pair", () => {
        const report = decodeBudgetReport(golden("budget"));
        expect(report.kind).toBe("budget");
        expect(report.buckets).toEqual(["2026-05", "2026-06", "2026-07"]);

        // sample.journal declares no periodic transactions, so everything lands in
        // the <unbudgeted> catch-all, whose goal is always null.
        const unbudgeted = report.rows[0];
        expect(unbudgeted).toMatchObject({account: "<unbudgeted>", depth: 1});
        expect(unbudgeted.cells[0].actual.get("$")).toEqual({m: -57100n, p: 2});
        expect(unbudgeted.cells[0].actual.get("EUR")).toEqual({m: 50000n, p: 2});
        expect(unbudgeted.cells[0].goal).toBeNull();
        expect(unbudgeted.cells[2].actual.size).toBe(0);
        expect(report.totals[1].actual.get("TSLA")).toEqual({m: -2n, p: 0});
        expect(report.totals[1].goal).toBeNull();
    });

    // Literal: needs a `~` periodic transaction, which fixtures/sample.journal has
    // none of (the budget goldens live in fixtures/budget/ and are exercised by
    // crates/ledgeline-server/tests/report_endpoints.rs).
    it("keeps null goal (unbudgeted) distinct from an empty-object goal (budgeted-but-zero)", () => {
        const report = decodeBudgetReport({
            buckets: ["2026-01", "2026-02"],
            rows: [
                {account: "<unbudgeted>", depth: 1, cells: [{actual: {$: dec(-375, 0)}, goal: null}]},
                // budgeted-but-zero-this-bucket: goal is {} (empty object), NOT null.
                {
                    account: "expenses:gifts",
                    depth: 2,
                    cells: [
                        {actual: {}, goal: {}},
                        {actual: {$: dec(50, 0)}, goal: {$: dec(25, 0)}},
                    ],
                },
            ],
            totals: [{actual: {$: dec(-23, 0)}, goal: {$: dec(400, 0)}}],
        });
        expect(report.rows[0].cells[0].goal).toBeNull();
        const gifts = report.rows[1];
        expect(gifts.cells[0].goal).not.toBeNull();
        expect(gifts.cells[0].goal?.size).toBe(0);
        expect(gifts.cells[1].goal?.get("$")).toEqual({m: 25n, p: 0});
        expect(report.totals[0].goal?.get("$")).toEqual({m: 400n, p: 0});
    });

    it("throws ApiShapeError when totals is missing", () => {
        expect(() => decodeBudgetReport({buckets: [], rows: []})).toThrow(ApiShapeError);
    });

    it("throws ApiShapeError when a row lacks cells", () => {
        expect(() => decodeBudgetReport({buckets: ["2026-01"], rows: [{account: "a", depth: 1}], totals: []})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — HoldingsReport over the holdings golden", () => {
    const raw = golden("holdings");

    it("decodes priced and tainted holdings, keeping nulls", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.base).toBe("$");
        expect(report.holdings.map((h) => h.symbol)).toEqual(["VTI", "AAPL", "TSLA", "GLD"]);

        const aapl = report.holdings[1];
        expect(aapl.name).toBe("Apple Inc.");
        expect(aapl.accounts).toEqual(["assets:broker:taxable:aapl"]);
        expect(aapl.shares).toEqual({m: 195n, p: 1});
        expect(aapl.basis).toEqual({m: 4346100n, p: 3});
        expect(aapl.firstBasisDate).toBe("2024-09-16");
        expect(aapl.price).toEqual({qty: {m: 27025n, p: 2}, date: "2026-06-30", source: "directive"});
        expect(aapl.marketValue).toEqual({m: 5269875n, p: 3});
        expect(aapl.gain).toEqual({m: 923775n, p: 3});
        expect(aapl.gainPct).toBeCloseTo(21.2552633, 6);

        // A short position priced from a cost annotation rather than a P directive.
        const tsla = report.holdings[2];
        expect(tsla.shares).toEqual({m: -2n, p: 0});
        expect(tsla.price?.source).toBe("cost");
        expect(tsla.basis).toBeNull();
        expect(tsla.firstBasisDate).toBeNull();

        // Held but unpriced: every derived field stays null rather than becoming 0.
        const gld = report.holdings[3];
        expect(gld.price).toBeNull();
        expect(gld.marketValue).toBeNull();
        expect(gld.gain).toBeNull();
        expect(gld.gainPct).toBeNull();
    });

    it("decodes portfolio totals and the gainer/loser lists", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.totals.marketValue).toEqual({m: 9922625n, p: 3});
        expect(report.totals.basis).toEqual({m: 9039460n, p: 3});
        expect(report.totals.gain).toEqual({m: 1513165n, p: 3});
        expect(report.totals.gainPct).toBeCloseTo(16.7395508, 6);
        expect(report.topGainers.map((h) => h.symbol)).toEqual(["AAPL", "VTI"]);
        expect(report.topLosers).toEqual([]);
    });

    it("decodes the warning union", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.warnings.map((w) => [w.symbol, w.kind])).toEqual([
            ["GLD", "unpriced"],
            ["GLD", "missing-basis"],
            ["TSLA", "negative-shares"],
        ]);
        expect(report.warnings[0].message).toMatch(/no market price/);
    });

    // Literal: sample.journal's portfolio is priced well enough to produce real
    // totals, so the "refused totals" branch has no golden.
    it("keeps null portfolio totals (honest-totals rule) with a present market value", () => {
        const report = decodeHoldingsReport({
            asOf: "2026-07-08",
            base: "$",
            holdings: [],
            totals: {marketValue: dec(10552625, 3), basis: null, gain: null, gainPct: null},
        });
        expect(report.totals.marketValue).toEqual({m: 10552625n, p: 3});
        expect(report.totals.basis).toBeNull();
        expect(report.totals.gain).toBeNull();
        expect(report.totals.gainPct).toBeNull();
    });

    it("rejects an unknown warning kind", () => {
        expect(() => decodeHoldingsReport({...(raw as object), warnings: [{symbol: "X", kind: "surprise", message: "?"}]})).toThrow(/unknown warning kind/);
    });

    it("rejects an unknown price source", () => {
        const holdings = [{symbol: "X", name: "X", shares: dec(1, 0), price: {qty: dec(1, 0), date: "2026-01-01", source: "guess"}}];
        expect(() => decodeHoldingsReport({...(raw as object), holdings})).toThrow(/unknown price source/);
    });

    it("throws ApiShapeError when base is missing", () => {
        expect(() => decodeHoldingsReport({asOf: "2026-07-08", holdings: [], totals: {marketValue: dec(0, 0)}})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — HoldingsSeries over the holdings-series golden", () => {
    it("decodes points with labels and basis", () => {
        const series = decodeHoldingsSeries(golden("holdings-series"));
        expect(series.base).toBe("$");
        expect(series.hasBasis).toBe(true);
        expect(series.points).toHaveLength(3);
        expect(series.points[0]).toMatchObject({date: "2026-05-31", bucket: "2026-05", label: "May 2026"});
        expect(series.points[0].marketValue).toEqual({m: 10045300n, p: 3});
        expect(series.points[0].basis).toEqual({m: 9039460n, p: 3});
        expect(series.points[2].date).toBe("2026-07-08");
    });

    // Literal: every point in the golden has a basis.
    it("keeps a null basis null", () => {
        const series = decodeHoldingsSeries({
            base: "$",
            points: [{date: "2026-05-31", bucket: "2026-05", label: "May 2026", marketValue: dec(10045300, 3), basis: null}],
            hasBasis: false,
        });
        expect(series.points[0].basis).toBeNull();
        expect(series.hasBasis).toBe(false);
    });

    it("throws ApiShapeError when points is missing", () => {
        expect(() => decodeHoldingsSeries({base: "$"})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — InsightsReport over the insights golden", () => {
    const raw = golden("insights");

    it("decodes the comparison period and the metric deltas", () => {
        const report = decodeInsightsReport(raw);
        expect(report.base).toBe("$");
        expect(report.journalStart).toBe("2024-07-01");
        expect(report.period).toEqual({
            start: "2025-07-01",
            mid: "2026-01-03",
            end: "2026-07-08",
            prevStart: "2025-07-01",
            prevEnd: "2026-01-03",
            currStart: "2026-01-04",
            currEnd: "2026-07-08",
        });

        expect(report.revenue.current.get("$")).toEqual({m: 3401000n, p: 2});
        expect(report.revenue.previous.get("$")).toEqual({m: 3399750n, p: 2});
        expect(report.revenue.delta.get("$")).toEqual({m: 1250n, p: 2});
        expect(report.revenue.pct).toBeCloseTo(0.0367674, 6);
        // a multi-commodity metric keeps every commodity
        expect(report.expenses.delta.get("EUR")).toEqual({m: -47575n, p: 2});
        expect(report.netWorth.current.get("$")).toEqual({m: 594514650n, p: 4});
        expect(report.cashBalance.delta.get("$")).toEqual({m: 1523496n, p: 2});
    });

    it("decodes cost of living and investment performance", () => {
        const report = decodeInsightsReport(raw);
        expect(report.costOfLiving.currentTotal.get("$")).toEqual({m: 1422613n, p: 2});
        expect(report.costOfLiving.previousTotal.get("$")).toEqual({m: 1550676n, p: 2});
        expect(report.costOfLiving.monthsCurrent).toBe(6);
        expect(report.costOfLiving.monthsPrevious).toBe(6);

        expect(report.investment.current.gain).toEqual({m: 562675n, p: 3});
        expect(report.investment.current.gainPct).toBeCloseTo(5.119905, 6);
        expect(report.investment.previous.gain).toEqual({m: 124425n, p: 2});
    });

    it("decodes change rows, movers, and top transactions", () => {
        const report = decodeInsightsReport(raw);
        const rent = report.expenseChanges[0];
        expect(rent.account).toBe("expenses:housing:rent");
        expect(rent.current).toEqual({m: 1125000n, p: 2});
        expect(rent.previous).toEqual({m: 1305000n, p: 2});
        expect(rent.delta).toEqual({m: -180000n, p: 2});
        expect(rent.pct).toBeCloseTo(-13.7931034, 6);
        expect(rent.kind).toBe("changed");
        expect(report.revenueChanges.map((r) => r.account)).toEqual(["income:dividends", "income:salary"]);

        expect(report.movers.map((m) => m.symbol)).toEqual(["AAPL", "VTI", "GLD"]);
        expect(report.movers[0].name).toBe("Apple Inc.");
        expect(report.movers[0].gain).toEqual({m: 327525n, p: 3});
        expect(report.movers[0].startEstimated).toBe(false);
        // GLD's window start fell back to purchase cost — the UI caveats this row.
        expect(report.movers[2].startEstimated).toBe(true);

        const top = report.topTxns[0];
        expect(top).toMatchObject({index: 141, date: "2026-01-27", description: "Acme Corp | January salary"});
        expect(top.amount).toEqual({m: 566000n, p: 2});
    });

    // Literal: sample.journal produces only `changed` rows.
    it("decodes the `ended` change kind and rejects anything else", () => {
        const base = raw as {expenseChanges: unknown[]};
        const row = {account: "expenses:gone", current: dec(0, 2), previous: dec(5000, 2), delta: dec(-5000, 2), pct: -100, kind: "ended"};
        const report = decodeInsightsReport({...(raw as object), expenseChanges: [row]});
        expect(report.expenseChanges[0].kind).toBe("ended");
        expect(base.expenseChanges.length).toBeGreaterThan(0);

        expect(() => decodeInsightsReport({...(raw as object), expenseChanges: [{...row, kind: "new"}]})).toThrow(/unknown change kind/);
    });

    it("throws ApiShapeError when base is missing", () => {
        expect(() => decodeInsightsReport({period: {}})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — SubscriptionsReport over the subscriptions golden", () => {
    const raw = golden("subscriptions");

    it("decodes the detected monthly charges", () => {
        const report = decodeSubscriptionsReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.lookbackStart).toBe("2024-07-08");
        expect(report.annual).toEqual([]);

        const rent = report.monthly[0];
        expect(rent.payee).toBe("Oakview Properties");
        expect(rent.cadence).toBe("monthly");
        expect(rent.typicalAmount).toEqual({m: 187500n, p: 2});
        expect(rent.annualizedCost).toEqual({m: 22500n, p: 0});
        expect(rent.occurrences).toBe(24);
        expect(rent.firstSeen).toBe("2024-08-01");
        expect(rent.lastSeen).toBe("2026-07-01");
        expect(rent.nextExpected).toBe("2026-08-01");
        expect(rent.accounts).toEqual(["expenses:housing:rent"]);
        expect(rent.manual).toBe(false);
    });

    // Literal: sample.journal carries no `subscription:true` tag and no annual
    // cadence, so neither appears in the golden.
    it("decodes an annual, manually-tagged subscription", () => {
        const report = decodeSubscriptionsReport({
            asOf: "2026-07-08",
            lookbackStart: "2024-07-08",
            monthly: [],
            annual: [
                {
                    payee: "Domain Registrar",
                    cadence: "annual",
                    typicalAmount: dec(1800, 2),
                    annualizedCost: dec(1800, 2),
                    occurrences: 2,
                    firstSeen: "2024-09-01",
                    lastSeen: "2025-09-01",
                    nextExpected: "2026-09-01",
                    accounts: ["expenses:online"],
                    manual: true,
                },
            ],
        });
        expect(report.annual[0].cadence).toBe("annual");
        expect(report.annual[0].manual).toBe(true);
    });

    it("rejects an unknown cadence", () => {
        const bad = {asOf: "2026-07-08", lookbackStart: "2024-07-08", monthly: [{...(raw as {monthly: object[]}).monthly[0], cadence: "weekly"}]};
        expect(() => decodeSubscriptionsReport(bad)).toThrow(/unknown cadence/);
    });

    it("throws ApiShapeError when lookbackStart is missing", () => {
        expect(() => decodeSubscriptionsReport({asOf: "2026-07-08"})).toThrow(ApiShapeError);
    });
});

// ---------------------------------------------------------------------------
// DRY-3: a missing amount is an error, never a zero
// ---------------------------------------------------------------------------

describe("UNIT nativeDecode — a missing MixedAmount throws instead of rendering zero", () => {
    // decodeMixed used to return an empty Map for undefined, so a renamed or
    // dropped Rust field became `$0.00` (format.ts fmtBase) / `0`
    // (ReportTable.svelte) with nothing raising on either side of the wire.
    const row = (extra: object) => ({sections: [{title: "Assets", rows: [{account: "assets", depth: 1, ...extra}], total: {}}], grandTotal: {}});

    it("a report missing `inclusive` throws rather than rendering zero", () => {
        expect(() => decodeSectionedReport(row({own: {}}))).toThrow(ApiShapeError);
        expect(() => decodeSectionedReport(row({own: {}}))).toThrow(/inclusive: missing amount/);
    });

    it("a report missing `own` throws", () => {
        expect(() => decodeSectionedReport(row({inclusive: {}}))).toThrow(/own: missing amount/);
    });

    it("a section missing `total` throws", () => {
        expect(() => decodeSectionedReport({sections: [{title: "Assets", rows: []}], grandTotal: {}})).toThrow(/total: missing amount/);
    });

    it("a report missing `grandTotal` throws", () => {
        expect(() => decodeSectionedReport({sections: []})).toThrow(/grandTotal: missing amount/);
    });

    it("a period report missing a bucket value throws", () => {
        const bad = {buckets: ["2026-07"], rows: [{account: "a", depth: 1, values: [undefined]}], totals: [{}]};
        expect(() => decodePeriodReport(bad)).toThrow(/values\[0\]: missing amount/);
    });

    it("a budget cell missing `actual` throws", () => {
        const bad = {buckets: ["2026-07"], rows: [{account: "a", depth: 1, cells: [{goal: null}]}], totals: []};
        expect(() => decodeBudgetReport(bad)).toThrow(/actual: missing amount/);
    });

    it("an insights metric missing `delta` throws", () => {
        const bad = {...(golden("insights") as object), revenue: {current: {}, previous: {}, pct: 0}};
        expect(() => decodeInsightsReport(bad)).toThrow(/delta: missing amount/);
    });

    it("still decodes an EMPTY amount `{}` as an empty Map — absent and empty are different facts", () => {
        const report = decodeSectionedReport(row({own: {}, inclusive: {}}));
        expect(report.sections[0].rows[0].inclusive.size).toBe(0);
        expect(report.grandTotal.size).toBe(0);
    });
});

// ---------------------------------------------------------------------------
// DRY-3: the cross-language rename guard
// ---------------------------------------------------------------------------

/** Deep-clone `node`, renaming every key whose collapsed path equals `target`. */
function renameKeyAt(node: unknown, path: string, target: string): unknown {
    if (Array.isArray(node)) return node.map((child) => renameKeyAt(child, `${path}[]`, target));
    if (node !== null && typeof node === "object") {
        const out: Record<string, unknown> = {};
        for (const [key, value] of Object.entries(node)) {
            const child = `${path}.${key}`;
            out[child === target ? `${key}Renamed` : key] = renameKeyAt(value, child, target);
        }
        return out;
    }
    return node;
}

/** Every key path in `node`, with array indices collapsed to `[]`. */
function keyPaths(node: unknown, path: string, out: Set<string>): void {
    if (Array.isArray(node)) {
        for (const child of node) keyPaths(child, `${path}[]`, out);
    } else if (node !== null && typeof node === "object") {
        for (const [key, value] of Object.entries(node)) {
            out.add(`${path}.${key}`);
            keyPaths(value, `${path}.${key}`, out);
        }
    }
}

/** Structural snapshot of a decoded report — Maps and BigInts made comparable. */
function shape(value: unknown): string {
    return JSON.stringify(value, (_key, item) => (typeof item === "bigint" ? `${item}n` : item instanceof Map ? [...item.entries()] : (item as unknown)));
}

describe("UNIT nativeDecode — RulesIndex over the rules-index golden", () => {
    it("decodes the discovery listing, including the fields a summary needs", () => {
        const index = decodeRulesIndex(rulesGolden("rules-index"));
        expect(index.rootLabel).toBe("tree");
        expect(index.editable).toBe(true);
        expect(index.truncated).toBe(false);
        expect(index.warnings).toEqual([]);
        expect(index.files).toHaveLength(1);
        expect(index.files[0]).toEqual({
            id: "import/2026/bank.csv.rules",
            label: "bank",
            revision: "665-f7dafa70ae699ef1",
            sizeBytes: 1637,
            parsed: true,
            account1: "assets:bank:checking",
            account2: "expenses:unknown",
            ifBlockCount: 4,
            editableBlockCount: 3,
            opaqueItemCount: 1,
            warnings: [],
        });
    });

    // `account1`/`account2` are `skip_serializing_if = "Option::is_none"`, so an
    // absent key means "this file declares neither" — a different fact from an
    // empty string, and one the file list renders differently.
    it("reads an absent account1/account2 as null rather than as an empty string", () => {
        const index = decodeRulesIndex({
            rootLabel: "fixtures",
            editable: false,
            truncated: false,
            files: [{id: "a.rules", label: "a", revision: "0-0", sizeBytes: 0, parsed: false, ifBlockCount: 0, editableBlockCount: 0, opaqueItemCount: 0}],
            warnings: ["one file was skipped"],
        });
        expect(index.files[0]).toMatchObject({account1: null, account2: null, parsed: false, warnings: []});
        expect(index.warnings).toEqual(["one file was skipped"]);
    });

    it("throws ApiShapeError when the files array is missing", () => {
        expect(() => decodeRulesIndex({rootLabel: "x", editable: true})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — RulesDocument over the rules-doc golden", () => {
    const raw = rulesGolden("rules-doc");

    it("decodes the document header and every settings entry with its item id", () => {
        const doc = decodeRulesDoc(raw);
        expect(doc.id).toBe("import/2026/bank.csv.rules");
        expect(doc.label).toBe("bank");
        expect(doc.revision).toBe("665-f7dafa70ae699ef1");
        expect(doc.editable).toBe(true);
        expect(doc.newline).toBe("lf");
        expect(doc.warnings).toEqual([]);

        expect(doc.settings.dateFormat).toEqual({value: "%Y-%m-%d", itemId: 3});
        expect(doc.settings.skip).toEqual({value: 1, itemId: 1});
        expect(doc.settings.account1).toEqual({value: "assets:bank:checking", itemId: 5});
        expect(doc.settings.account2).toEqual({value: "expenses:unknown", itemId: 6});
        expect(doc.settings.currency).toEqual({value: "$", itemId: 4});
        expect(doc.settings.fields).toEqual({names: ["date", "description", "amount"], itemId: 2});
        // Absent settings are "the file does not say", which is NOT hledger's
        // default for them — choosing a default is a rendering decision.
        expect(doc.settings.separator).toBeNull();
        expect(doc.settings.source).toBeNull();
        expect(doc.settings.newestFirst).toBeNull();
    });

    it("decodes every item kind the wire can carry, in order", () => {
        const doc = decodeRulesDoc(raw);
        expect(doc.items.map((item) => item.kind)).toEqual([
            "trivia",
            "directive",
            "fields",
            "directive",
            "assignment",
            "assignment",
            "assignment",
            "ifBlock",
            "ifBlock",
            "ifBlock",
            "opaque",
        ]);
        expect(doc.items.map((item) => item.id)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        const trivia = doc.items[0];
        expect(trivia).toMatchObject({kind: "trivia", line: 1, lines: 13, truncated: false});
        expect(trivia?.kind === "trivia" && trivia.text).toContain("discovery MUST find");

        expect(doc.items[1]).toMatchObject({kind: "directive", name: "skip", value: "1"});
        expect(doc.items[2]).toMatchObject({kind: "fields", names: ["date", "description", "amount"]});
        expect(doc.items[5]).toMatchObject({kind: "assignment", field: "account1", value: "assets:bank:checking"});
    });

    it("reads a whole-record matcher's absent `field` as null, and a scoped one by name", () => {
        const doc = decodeRulesDoc(raw);
        expect(doc.items[7]).toMatchObject({
            kind: "ifBlock",
            layout: "inline",
            matchers: [{field: null, pattern: "COFFEE"}],
            assignments: [{field: "account2", value: "expenses:food:coffee"}],
        });
        expect(doc.items[9]).toMatchObject({
            kind: "ifBlock",
            layout: "stacked",
            matchers: [
                {field: "description", pattern: "SUPERMARKET"},
                {field: "description", pattern: "GROCER"},
            ],
            assignments: [
                {field: "account2", value: "expenses:food:groceries"},
                {field: "comment", value: "weekly shop"},
            ],
        });
    });

    it("carries an opaque construct's reason, label and raw text", () => {
        const opaque = decodeRulesDoc(raw).items[10];
        expect(opaque).toMatchObject({kind: "opaque", reason: "ifTable", label: "if,account2,comment", truncated: false});
        expect(opaque?.kind === "opaque" && opaque.text).toContain("ATM WITHDRAWAL,assets:cash,cash out");
    });

    // An item whose shape the decoder invented would be echoed back to the
    // engine under an id it has no right to claim, so refusing to open the
    // document is the honest failure.
    it("throws on an unknown item kind rather than inventing a carry-through", () => {
        const doc = raw as {items: unknown[]};
        const mutated = {...doc, items: [...doc.items, {id: 11, line: 50, lines: 1, kind: "somethingNew"}]};
        expect(() => decodeRulesDoc(mutated)).toThrow(ApiShapeError);
    });

    it("throws on an unknown opaque reason, layout or newline", () => {
        expect(() => decodeRulesDoc({...(raw as object), newline: "cr"})).toThrow(ApiShapeError);
        const items = (raw as {items: Record<string, unknown>[]}).items;
        expect(() => decodeRulesDoc({...(raw as object), items: [{...items[10], reason: "somethingNew"}]})).toThrow(ApiShapeError);
        expect(() => decodeRulesDoc({...(raw as object), items: [{...items[7], layout: "table"}]})).toThrow(ApiShapeError);
    });

    it("decodes an anchored warning, and one about the file as a whole", () => {
        const doc = decodeRulesDoc({
            ...(raw as object),
            warnings: [
                {itemId: 2, line: 15, message: "hledger requires at least two comma-separated field names"},
                {line: 0, message: "this file has no date field"},
            ],
        });
        expect(doc.warnings[0]).toEqual({itemId: 2, line: 15, message: "hledger requires at least two comma-separated field names"});
        expect(doc.warnings[1]).toEqual({itemId: null, line: 0, message: "this file has no date field"});
    });
});

describe("UNIT nativeDecode — RulesPreview", () => {
    // Literal, not a golden: the preview route has no committed body (its rows
    // come from a CSV, not from the journal). These are the exact bytes the
    // engine answered for fixtures/rules/tree/import/2026/bank.csv.
    it("decodes an available preview, header and sample rows included", () => {
        const preview = decodeRulesPreview({
            available: true,
            dataLabel: "bank.csv",
            separator: ",",
            header: ["Date", "Description", "Amount"],
            rows: [["2026-01-03", "COFFEE HOUSE", "-6.45"]],
            columns: 3,
            truncated: false,
        });
        expect(preview.available).toBe(true);
        expect(preview.reason).toBeNull();
        expect(preview.dataLabel).toBe("bank.csv");
        expect(preview.header).toEqual(["Date", "Description", "Amount"]);
        expect(preview.rows).toEqual([["2026-01-03", "COFFEE HOUSE", "-6.45"]]);
        expect(preview.columns).toBe(3);
    });

    // A refusal is a VALUE, not an error: "your `source` is a shell command we
    // will not run" is information the mapping panel shows.
    it("decodes a refusal with its typed reason and no header", () => {
        const preview = decodeRulesPreview({
            available: false,
            reason: "sourceIsCommand",
            dataLabel: "piped",
            separator: ",",
            rows: [],
            columns: 0,
            truncated: false,
        });
        expect(preview).toMatchObject({available: false, reason: "sourceIsCommand", header: null, rows: [], columns: 0});
    });

    it("throws on a reason it does not know rather than reporting `no reason`", () => {
        expect(() => decodeRulesPreview({available: false, reason: "somethingNew", separator: ",", rows: [], columns: 0})).toThrow(ApiShapeError);
    });

    it("throws when rows is missing", () => {
        expect(() => decodeRulesPreview({available: true, separator: ","})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — renaming any wire key is detected, not absorbed", () => {
    const DECODERS: [string, (raw: unknown) => unknown][] = [
        ["balancesheet", decodeSectionedReport],
        ["incomestatement", decodeSectionedReport],
        ["cashflow", decodePeriodReport],
        ["networth", decodePeriodReport],
        ["budget", decodeBudgetReport],
        ["insights", decodeInsightsReport],
        ["subscriptions", decodeSubscriptionsReport],
        ["holdings", decodeHoldingsReport],
        ["holdings-series", decodeHoldingsSeries],
    ];

    // Renames these decoders CANNOT currently notice. Every one is a gap in the
    // FIXTURE, not in the decoder: the field's value in fixtures/sample.journal
    // happens to equal the value the decoder falls back to, so dropping the key
    // is indistinguishable from keeping it. Closing them needs journal data
    // sample.journal does not have (a `~` periodic transaction, a
    // `subscription:true` tag, an annual charge, a losing position).
    //
    // This list must only ever SHRINK. A new entry means a newly-added field can
    // be renamed in Rust and silently absorbed here — the DRY-3 bug, returning.
    const TOLERATED = new Set([
        "budget $.rows[].cells[].goal", // every goal in the golden is already null
        "budget $.totals[].goal", // ditto
        "subscriptions $.monthly[].manual", // no `subscription:true` tag in sample.journal
        "subscriptions $.annual", // no annual cadence in sample.journal → [] either way
        "holdings $.topLosers", // nothing is down as of 2026-07-08 → [] either way
    ]);

    /** Rename every key of every golden in turn; report the ones nothing noticed. */
    function sweep(): {tolerated: string[]; checked: number} {
        const tolerated: string[] = [];
        let checked = 0;
        for (const [name, decode] of DECODERS) {
            const raw = golden(name);
            const baseline = shape(decode(raw));
            const paths = new Set<string>();
            keyPaths(raw, "$", paths);

            for (const path of paths) {
                checked += 1;
                let absorbed: boolean;
                try {
                    absorbed = shape(decode(renameKeyAt(raw, "$", path))) === baseline;
                } catch {
                    absorbed = false; // threw — the rename was caught, which is the point
                }
                if (absorbed) tolerated.push(`${name} ${path}`);
            }
        }
        return {tolerated, checked};
    }

    it("every key in every golden is load-bearing, except the documented fixture gaps", () => {
        expect(new Set(sweep().tolerated)).toEqual(TOLERATED);
    });

    it("checks a meaningful number of keys (the sweep is actually running)", () => {
        expect(sweep().checked).toBeGreaterThan(300);
    });
});
