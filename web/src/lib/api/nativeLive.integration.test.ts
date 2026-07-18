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
import {styleFor} from "$lib/insights/series";
import {formatTotals} from "$lib/journal/rowModel";
import {reportStyles} from "$lib/reports/ui/styles";
import {HledgerApi} from "./client";
import {LedgelineApi} from "./native";
import {decodeHoldingsReport, decodeHoldingsSeries, decodePeriodReport, decodeSectionedReport} from "./nativeDecode";
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

    it("net worth reports GLD and TSLA as unpriced", async () => {
        const report = decodePeriodReport(await new LedgelineApi(url).netWorth({end: AS_OF, interval: "monthly", count: 3}));
        expect(report.buckets[report.buckets.length - 1]).toBe("2026-07");
        expect(report.meta?.unpriced).toEqual(["GLD", "TSLA"]);
    });

    it("cash flow buckets end at the as-of month", async () => {
        const report = decodePeriodReport(await new LedgelineApi(url).cashFlow({end: AS_OF, interval: "monthly", count: 3, depth: 2}));
        expect(report.buckets).toEqual(["2026-05", "2026-06", "2026-07"]);
        expect(report.rows.length).toBeGreaterThan(0);
    });

    it("holdings render AAPL/VTI values, GLD tainted, NVDA/TSLA absent, honest null totals", async () => {
        const txns = normalizeTransactions(await new HledgerApi(url).transactions());
        const report = decodeHoldingsReport(await new LedgelineApi(url).holdings({asOf: AS_OF, accounts: "", mode: "include"}));
        expect(report.base).toBe("$");
        const style = styleFor(txns, report.base);
        const fmt = (h: {marketValue: import("$lib/domain/money").Dec | null}) =>
            h.marketValue === null ? "—" : formatAmount({commodity: "$", qty: h.marketValue, style});

        const bySymbol = new Map(report.holdings.map((h) => [h.symbol, h]));
        // Priced holdings, sorted market value desc → VTI before AAPL.
        expect(report.holdings.map((h) => h.symbol)).toEqual(["VTI", "AAPL", "GLD"]);

        const aapl = bySymbol.get("AAPL")!;
        expect(aapl.name).toBe("Apple Inc.");
        expect(aapl.shares).toEqual({m: 195n, p: 1}); // 19.5 shares
        expect(fmt(aapl)).toBe("$5,269.88");
        expect(aapl.price?.date).toBe("2026-06-30");
        expect(formatAmount({commodity: "$", qty: aapl.price!.qty, style})).toBe("$270.25");

        const vti = bySymbol.get("VTI")!;
        expect(fmt(vti)).toBe("$5,282.75");
        expect(vti.basis === null ? "—" : formatAmount({commodity: "$", qty: vti.basis, style})).toBe("$4,693.36");

        // GLD present but tainted (null basis, unpriced); NVDA fully sold, TSLA net-negative → both hidden.
        expect(bySymbol.get("GLD")!.basis).toBeNull();
        expect(bySymbol.has("NVDA")).toBe(false);
        expect(bySymbol.has("TSLA")).toBe(false);

        // Honest totals: GLD in scope ⇒ market value real, basis/gain null.
        expect(formatAmount({commodity: "$", qty: report.totals.marketValue, style})).toBe("$10,552.63");
        expect(report.totals.basis).toBeNull();
        expect(report.totals.gain).toBeNull();

        // Warnings explain GLD (twice) + TSLA; both priced holdings are gainers.
        expect(report.warnings.map((w) => `${w.symbol}:${w.kind}`)).toEqual(["GLD:unpriced", "GLD:missing-basis", "TSLA:negative-shares"]);
        expect(report.topGainers.map((h) => h.symbol)).toEqual(["AAPL", "VTI"]);
        expect(report.topLosers).toEqual([]);
    });

    it("holdings series returns a trailing window with no basis line (GLD taints every point)", async () => {
        const series = decodeHoldingsSeries(await new LedgelineApi(url).holdingsSeries({asOf: AS_OF, mode: "include", interval: "monthly", count: 12}));
        expect(series.base).toBe("$");
        expect(series.points).toHaveLength(12);
        expect(series.hasBasis).toBe(false);
        expect(series.points[series.points.length - 1].label).toBe("Jul 2026");
    });
});
