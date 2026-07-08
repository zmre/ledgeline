// Read-back verification for the xlsx builder (WP-07 DoD): build a workbook,
// serialize it, re-load it with exceljs, and assert title row, headers, cell
// values, and number formats survive the round trip.

import {describe, expect, it} from "vitest";
import {Workbook} from "exceljs";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {PeriodReport, SectionedReport} from "$lib/reports/types";
import {buildWorkbook, numberFormat} from "./xlsx";

const usd = (cents: number): MixedAmount => new Map([["$", dec(cents, 2)]]);

async function roundTrip(report: SectionedReport | PeriodReport, meta: {title: string; params: string}) {
    const built = await buildWorkbook(report, meta);
    const buffer = await built.xlsx.writeBuffer();
    const loaded = new Workbook();
    await loaded.xlsx.load(buffer as never);
    const ws = loaded.getWorksheet(meta.title);
    if (ws === undefined) throw new Error(`worksheet "${meta.title}" missing after round trip`);
    return ws;
}

describe("UNIT export/xlsx", () => {
    describe("numberFormat", () => {
        it("derives decimals from the Dec's places (capped at 2) and affixes the commodity", () => {
            expect(numberFormat("$", 2)).toBe('"$"#,##0.00');
            expect(numberFormat("EUR", 0)).toBe('#,##0 "EUR"');
            expect(numberFormat("", 3)).toBe("#,##0.00"); // display cap: never more than 2 decimals
        });
    });

    it("sectioned report: title, params, headers, section rows, totals, numFmt", async () => {
        const report: SectionedReport = {
            asOf: "2026-07-08",
            sections: [
                {
                    title: "Assets",
                    rows: [
                        {account: "assets", depth: 1, own: new Map(), inclusive: usd(123456)},
                        {account: "assets:bank", depth: 2, own: new Map(), inclusive: usd(123456)},
                        {account: "assets:bank:checking", depth: 3, own: usd(123456), inclusive: usd(123456)},
                    ],
                    total: usd(123456),
                },
                {
                    title: "Liabilities",
                    rows: [{account: "liabilities", depth: 1, own: usd(20000), inclusive: usd(20000)}],
                    total: usd(20000),
                },
            ],
            grandTotal: usd(103456),
        };
        const ws = await roundTrip(report, {title: "Balance Sheet", params: "as of 2026-07-08, depth 3"});

        expect(ws.getCell("A1").value).toBe("Balance Sheet");
        expect(ws.getCell("A1").font.bold).toBe(true);
        expect(ws.getCell("A2").value).toBe("as of 2026-07-08, depth 3");
        expect([ws.getCell("A4").value, ws.getCell("B4").value]).toEqual(["Account", "Amount"]);
        expect(ws.getCell("A4").font.bold).toBe(true);

        // Section content: single-child chain compressed to one row.
        expect(ws.getCell("A5").value).toBe("Assets");
        expect(ws.getCell("A6").value).toBe("assets:bank:checking");
        expect(ws.getCell("B6").value).toBe(1234.56);
        expect(ws.getCell("B6").numFmt).toBe('"$"#,##0.00');
        expect(ws.getCell("A7").value).toBe("Total Assets");
        expect(ws.getCell("B7").value).toBe(1234.56);
        expect(ws.getCell("A8").value).toBe("Liabilities");
        expect(ws.getCell("A9").value).toBe("liabilities");
        expect(ws.getCell("B9").value).toBe(200);
        expect(ws.getCell("A10").value).toBe("Total Liabilities");
        expect(ws.getCell("A11").value).toBe("Net");
        expect(ws.getCell("B11").value).toBe(1034.56);
        expect(ws.getCell("B11").font.bold).toBe(true);
    });

    it("period report: bucket header labels, per-bucket values, Net totals row", async () => {
        const report: PeriodReport = {
            buckets: ["2026-06", "2026-07"],
            rows: [
                {account: "assets", depth: 1, values: [usd(-5000), usd(10050)]},
                {account: "assets:bank", depth: 2, values: [usd(-5000), usd(10050)]},
            ],
            totals: [usd(-5000), usd(10050)],
        };
        const ws = await roundTrip(report, {title: "Cash Flow", params: "last 2 monthly periods ending 2026-07-08, depth 2"});

        expect([ws.getCell("A4").value, ws.getCell("B4").value, ws.getCell("C4").value]).toEqual(["Account", "Jun 2026", "Jul 2026"]);
        // chain compressed to the single leaf row
        expect(ws.getCell("A5").value).toBe("assets:bank");
        expect(ws.getCell("B5").value).toBe(-50);
        expect(ws.getCell("C5").value).toBe(100.5);
        expect(ws.getCell("C5").numFmt).toBe('"$"#,##0.00');
        expect(ws.getCell("A6").value).toBe("Net");
        expect(ws.getCell("B6").value).toBe(-50);
        expect(ws.getCell("C6").value).toBe(100.5);
    });

    it("multi-commodity cells fall back to text; empty cells write 0", async () => {
        const mixed: MixedAmount = new Map([
            ["EUR", dec(1000, 2)],
            ["$", dec(2500, 2)],
        ]);
        const report: PeriodReport = {
            buckets: ["2026"],
            rows: [{account: "assets", depth: 1, values: [mixed]}],
            totals: [new Map()],
        };
        const ws = await roundTrip(report, {title: "Net Worth", params: "last 1 yearly periods ending 2026-07-08"});
        expect(ws.getCell("B5").value).toBe("25.00 $, 10.00 EUR"); // sorted by commodity ("$" < "EUR")
        expect(ws.getCell("B6").value).toBe(0);
    });
});
