// INTEGRATION check against a live ledgeline-server. Skipped unless
// LEDGELINE_API_URL is set, e.g.:
//   target/debug/ledgeline-server fixtures/sample.journal --port 5000
//   LEDGELINE_API_URL=http://127.0.0.1:5000 vitest run nativeLive
//
// Drives the SAME fetch → decode → format pipeline the /reports and /holdings
// pages use (LedgelineApi + nativeDecode + the display helpers reportStyles/
// formatTotals/styleFor/formatAmount), asserting the golden display strings —
// so a green run here is proof the pages render the correct numbers against the
// real engine, short of a DOM (the UI components are unchanged and already
// tested over these exact domain types).

import {describe, expect, it} from "vitest";
import {formatAmount} from "$lib/domain/money";
import type {MixedAmount} from "$lib/domain/money";
import {partitionShortPositions, shortPositionNote} from "$lib/holdings/ui/view";
import {styleFor} from "$lib/insights/series";
import {formatTotals} from "$lib/journal/rowModel";
import {reportStyles} from "$lib/reports/ui/styles";
import {HledgerApi} from "./client";
import {LedgelineApi} from "./native";
import {
    decodeBalanceSheetReport,
    decodeHoldingsReport,
    decodeHoldingsSeries,
    decodeIncomeStatementReport,
    decodePeriodReport,
    decodeSectionedReport,
} from "./nativeDecode";
import {normalizeTransactions} from "./normalize";

const apiUrl = process.env.LEDGELINE_API_URL;
const AS_OF = "2026-07-08"; // pinned like the e2e clock; the journal ends 2026-07-04 so this == "today"

/** The "$" line as the report table renders it (formatTotals → the exact cell string). */
function dollarLine(ma: MixedAmount, styles: ReadonlyMap<string, import("$lib/domain/types").AmountStyle>): string | undefined {
    return formatTotals(ma, styles).find((line) => line.text.startsWith("$"))?.text;
}

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live ledgeline-server native reports", () => {
    const url = apiUrl ?? "";

    it("balance sheet renders Total Assets $48,402.56 and Net $47,871.41", async () => {
        const styles = reportStyles(normalizeTransactions(await new HledgerApi(url).transactions()));
        const report = decodeSectionedReport(await new LedgelineApi(url).balanceSheet({asOf: AS_OF, depth: 2}));
        expect(report.asOf).toBe(AS_OF);

        const assets = report.sections.find((s) => s.title === "Assets");
        expect(assets).toBeDefined();
        expect(dollarLine(assets!.total, styles)).toBe("$48,402.56");
        expect(dollarLine(report.grandTotal, styles)).toBe("$47,871.41");
    });

    it("income statement totals revenues $34,010.00 over the year to date", async () => {
        const styles = reportStyles(normalizeTransactions(await new HledgerApi(url).transactions()));
        const report = decodeSectionedReport(await new LedgelineApi(url).incomeStatement({from: "2026-01-01", to: AS_OF, depth: 2}));
        const revenues = report.sections.find((s) => s.title === "Revenues");
        expect(revenues).toBeDefined();
        expect(dollarLine(revenues!.total, styles)).toBe("$34,010.00");
    });

    // The two GROUPED statements. These matter more than the flat ones above: the
    // flat reports are pinned byte-for-byte by a golden, whereas these are the
    // reports the /reports page actually renders, and their decoders are the ones
    // carrying the `Amounts`/`prior` present-vs-absent convention that no type
    // system checks across the language boundary.
    it("grouped balance sheet ties out and names its unpriced commodities", async () => {
        const report = decodeBalanceSheetReport(await new LedgelineApi(url).balanceSheetGrouped({asOf: AS_OF}));
        expect(report.asOf).toBe(AS_OF);
        expect(report.sections.map((s) => s.kind)).toEqual(["assets", "liabilities", "equity"]);
        expect(report.balanced).toBe(true);
        // GLD and TSLA have no `P` directive at any date — the genuinely unpriced path.
        expect(report.meta?.unpriced).toEqual(["GLD", "TSLA"]);
    });

    it("grouped P&L groups by segment, valued, against the prior equal-length window", async () => {
        const styles = reportStyles(normalizeTransactions(await new HledgerApi(url).transactions()));
        const report = decodeIncomeStatementReport(await new LedgelineApi(url).incomeStatementGrouped({from: "2026-01-01", to: AS_OF}));

        // Untagged journal ⇒ the simple two-box shape, no GAAP ladder.
        expect(report.multiStep).toBe(false);
        expect(report.sections.map((s) => s.kind)).toEqual(["revenue", "opex"]);
        expect(report.sections.map((s) => s.title)).toEqual(["Revenue", "Expenses"]);
        expect(report.sections.flatMap((s) => s.trailing)).toEqual([]);

        // hledger 1.52: `is -V -b 2026-01-01 -e 2026-07-09 --depth 2`.
        const [revenue, expenses] = report.sections;
        expect(dollarLine(revenue.total.current, styles)).toBe("$34,010.00");
        expect(dollarLine(expenses.total.current, styles)).toBe("$25,126.48");
        expect(dollarLine(report.netIncome.current, styles)).toBe("$8,883.52");
        expect(revenue.groups.map((g) => g.name)).toEqual(["Dividends", "Salary"]);
        expect(revenue.groups.every((g) => g.source === "segment")).toBe(true);

        // The prior window is equal-LENGTH, not the prior calendar year:
        // `is -V -b 2025-06-26 -e 2026-01-01 --depth 2`.
        expect(report.prior).toEqual({from: "2025-06-26", to: "2025-12-31"});
        expect(dollarLine(revenue.total.prior!, styles)).toBe("$39,397.50");
        expect(dollarLine(report.netIncome.prior!, styles)).toBe("$14,880.79");
    });

    it("omits every prior figure, not just the header, when compare=none", async () => {
        const report = decodeIncomeStatementReport(await new LedgelineApi(url).incomeStatementGrouped({from: "2026-01-01", to: AS_OF, compare: "none"}));
        expect(report.prior).toBeNull();
        expect(report.netIncome.prior).toBeUndefined();
        expect(
            report.sections
                .flatMap((s) => [s.total, ...s.groups.flatMap((g) => [g.total, ...g.rows.map((r) => r.amounts)])])
                .every((a) => a.prior === undefined)
        ).toBe(true);
    });

    // GLD and TSLA used to land here as unpriced. Since net worth started inferring
    // market prices from @/@@ purchase costs, the sample journal prices everything and
    // the server omits `meta` entirely. `reports_golden.rs` asserts the same thing on
    // the Rust side (`report.meta.is_none()`); keep the two in step.
    it("net worth leaves nothing unpriced", async () => {
        const report = decodePeriodReport(await new LedgelineApi(url).netWorth({end: AS_OF, interval: "monthly", count: 3}));
        expect(report.buckets[report.buckets.length - 1]).toBe("2026-07");
        expect(report.meta).toBeUndefined();
    });

    it("cash flow buckets end at the as-of month", async () => {
        const report = decodePeriodReport(await new LedgelineApi(url).cashFlow({end: AS_OF, interval: "monthly", count: 3, depth: 2}));
        expect(report.buckets).toEqual(["2026-05", "2026-06", "2026-07"]);
        expect(report.rows.length).toBeGreaterThan(0);
    });

    it("holdings render AAPL/VTI values, GLD tainted, NVDA absent, TSLA short-and-hidden, partial totals", async () => {
        const txns = normalizeTransactions(await new HledgerApi(url).transactions());
        const report = decodeHoldingsReport(await new LedgelineApi(url).holdings({asOf: AS_OF, accounts: "", mode: "include"}));
        expect(report.base).toBe("$");
        const style = styleFor(txns, report.base);
        const fmt = (h: {marketValue: import("$lib/domain/money").Dec | null}) =>
            h.marketValue === null ? "—" : formatAmount({commodity: "$", qty: h.marketValue, style});

        const bySymbol = new Map(report.holdings.map((h) => [h.symbol, h]));
        // Sorted market value desc → VTI before AAPL, then the net-short TSLA
        // (−$630.00), then unpriced GLD.
        expect(report.holdings.map((h) => h.symbol)).toEqual(["VTI", "AAPL", "TSLA", "GLD"]);

        const aapl = bySymbol.get("AAPL")!;
        expect(aapl.name).toBe("Apple Inc.");
        expect(aapl.shares).toEqual({m: 195n, p: 1}); // 19.5 shares
        expect(fmt(aapl)).toBe("$5,269.88");
        expect(aapl.price?.date).toBe("2026-06-30");
        expect(formatAmount({commodity: "$", qty: aapl.price!.qty, style})).toBe("$270.25");

        const vti = bySymbol.get("VTI")!;
        expect(fmt(vti)).toBe("$5,282.75");
        expect(vti.basis === null ? "—" : formatAmount({commodity: "$", qty: vti.basis, style})).toBe("$4,693.36");

        // GLD present but tainted (null basis, unpriced). NVDA was fully sold → 0
        // shares → genuinely gone. TSLA is net −2 sh (sold, never bought): the
        // engine reports it, because the balance sheet values those shares.
        expect(bySymbol.get("GLD")!.basis).toBeNull();
        expect(bySymbol.has("NVDA")).toBe(false);
        const tsla = bySymbol.get("TSLA")!;
        expect(tsla.shares).toEqual({m: -2n, p: 0});
        expect(fmt(tsla)).toBe("$-630.00");
        expect(tsla.basis).toBeNull(); // the opening lot was never entered
        expect(tsla.gain).toBeNull();

        // Partial totals: GLD (tainted+unpriced) and TSLA (short) are out of
        // basis/gain; market value carries the short, so it reads $630.00 below
        // the $10,552.63 it showed while the row was withheld. That $9,922.63 is
        // what makes the portfolio agree with the valued balance sheet
        // (hledger `bal assets:broker … --value=end,'$'` → $17,532.38 with cash).
        expect(formatAmount({commodity: "$", qty: report.totals.marketValue, style})).toBe("$9,922.63");
        expect(report.totals.basis === null ? "—" : formatAmount({commodity: "$", qty: report.totals.basis, style})).toBe("$9,039.46");
        expect(report.totals.gain === null ? "—" : formatAmount({commodity: "$", qty: report.totals.gain, style})).toBe("$1,513.17");

        // Warnings explain GLD (twice) + TSLA; both priced holdings are gainers,
        // and the short is in NEITHER ranking (a null gain has nothing to rank).
        expect(report.warnings.map((w) => `${w.symbol}:${w.kind}`)).toEqual(["GLD:unpriced", "GLD:missing-basis", "TSLA:negative-shares"]);
        expect(report.topGainers.map((h) => h.symbol)).toEqual(["AAPL", "VTI"]);
        expect(report.topLosers).toEqual([]);

        // What the /holdings page then does with that report: the short is kept
        // out of the table and accounted for by the note under it. This is the
        // closest check to the rendered page available without a DOM.
        const {shown, hidden} = partitionShortPositions(report.holdings);
        expect(shown.map((h) => h.symbol)).toEqual(["VTI", "AAPL", "GLD"]);
        expect(hidden.map((h) => h.symbol)).toEqual(["TSLA"]);
        expect(shortPositionNote(hidden, (v) => formatAmount({commodity: "$", qty: v, style}))).toBe(
            "1 short position is hidden (TSLA): net shares are negative, so the opening purchase was likely never recorded. " +
                "Its market value ($-630.00) is still counted in the totals above."
        );
    });

    it("holdings series returns a trailing window with a partial basis line (GLD excluded)", async () => {
        const series = decodeHoldingsSeries(await new LedgelineApi(url).holdingsSeries({asOf: AS_OF, mode: "include", interval: "monthly", count: 12}));
        expect(series.base).toBe("$");
        expect(series.points).toHaveLength(12);
        expect(series.hasBasis).toBe(true);
        expect(series.points[series.points.length - 1].label).toBe("Jul 2026");
    });
});
