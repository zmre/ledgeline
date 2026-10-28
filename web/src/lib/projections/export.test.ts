// Every Projections reading downloads as XLSX (2026-10-28). What only a built
// workbook can answer: which columns say "(projected)", what the runway rows
// hold, and that the Journal's download is exactly the list on screen.

import type {Workbook, Worksheet} from "exceljs";
import {describe, expect, it} from "vitest";
import {DEFAULT_AMOUNT_STYLE} from "#lib/format/amounts.js";
import {accountsFixture, journalFixture, projectionFixture} from "#lib/testing/projectionFixture.js";
import {markProjected, matchingEntries, PROJECTED_SUFFIX, projectionExport, scenarioSlug, type ExportContext} from "./export";
import {PROJECTION_TABS, type ProjectionTab} from "./params";

const CONTEXT: ExportContext = {
    scenarioName: "Series A, hiring ramp",
    runwaySentence: "Cash never turns negative over the 2 months projected.",
    base: "$",
    journalQuery: "",
    styleOf: () => DEFAULT_AMOUNT_STYLE,
};

/** Every non-empty cell value of the first sheet, as text. */
function texts(workbook: Workbook): string[] {
    const ws = workbook.worksheets[0]!;
    const out: string[] = [];
    ws.eachRow((row) => row.eachCell((cell) => out.push(String(cell.value))));
    return out;
}

/** Row 4, the header row every report workbook writes. */
function headers(ws: Worksheet): string[] {
    return (ws.getRow(4).values as unknown[]).slice(1).map(String);
}

async function built(tab: ProjectionTab, context: ExportContext = CONTEXT, overrides = {}) {
    const download = projectionExport(tab, projectionFixture(overrides), context);
    expect(download, tab).not.toBeNull();
    return {download: download!, workbook: await download!.build()};
}

describe("UNIT projections/export", () => {
    it("labels every column from the first forecast one on as projected", () => {
        const columns = [{label: "Jun"}, {label: "Jul"}, {label: "Aug"}];
        expect(markProjected(columns, 1).map((c) => c.label)).toEqual(["Jun", `Jul${PROJECTED_SUFFIX}`, `Aug${PROJECTED_SUFFIX}`]);
        expect(markProjected(columns, 3).map((c) => c.label)).toEqual(["Jun", "Jul", "Aug"]);
    });

    it("names the file after the scenario, the reading and the window", () => {
        expect(scenarioSlug("Series A, hiring ramp")).toBe("series-a-hiring-ramp");
        expect(scenarioSlug("  ")).toBe("scenario");
        const download = projectionExport("is", projectionFixture(), CONTEXT)!;
        expect(download.filename).toBe("projection-series-a-hiring-ramp-income-statement-2026-06-01-to-2026-09-30.xlsx");
        expect(download.title).toBe("Projected Income Statement — Series A, hiring ramp");
    });

    it("offers a download on every tab once its reading is on hand", () => {
        const ready = projectionFixture({accounts: accountsFixture(), journal: journalFixture()});
        for (const tab of PROJECTION_TABS) expect(projectionExport(tab, ready, CONTEXT), tab).not.toBeNull();
        // Accounts and Journal ride on the run only while their tab is open.
        expect(projectionExport("accounts", projectionFixture(), CONTEXT)).toBeNull();
        expect(projectionExport("journal", projectionFixture(), CONTEXT)).toBeNull();
    });

    it("writes the balance sheet with its forecast columns marked, actuals not", async () => {
        const {workbook} = await built("bs");
        // The money columns: the extras column's heading quotes the last one.
        const heads = headers(workbook.worksheets[0]!).filter((h) => !h.startsWith("Other commodities"));
        expect(heads.filter((h) => h.includes(PROJECTED_SUFFIX))).toHaveLength(2);
        expect(heads.find((h) => h.startsWith("Jun 2026"))).not.toContain(PROJECTED_SUFFIX);
        expect(heads.find((h) => h.startsWith("Aug 2026"))).toContain(PROJECTED_SUFFIX);
    });

    it("writes the income statement and the cash flow with their forecast columns marked", async () => {
        for (const tab of ["is", "cf"] as const) {
            const {workbook} = await built(tab);
            const heads = headers(workbook.worksheets[0]!).filter((h) => !h.startsWith("Other commodities"));
            const projection = projectionFixture();
            const buckets = tab === "is" ? projection.incomeStatement.buckets! : projection.cashFlow.buckets;
            expect(
                heads.filter((h) => h.includes(PROJECTED_SUFFIX)),
                tab
            ).toHaveLength(Math.max(0, buckets.length - projection.projectedFrom));
        }
    });

    it("writes the runway: the real opening balance, then each forecast period's closing cash", async () => {
        const {workbook, download} = await built("cash");
        const ws = workbook.worksheets[0]!;
        expect(ws.getCell("A2").value).toBe(CONTEXT.runwaySentence);
        expect(download.params).toBe(CONTEXT.runwaySentence);
        expect(headers(ws)).toEqual(["Period", "Cash ($)"]);
        expect([ws.getCell("A5").value, ws.getCell("B5").value]).toEqual(["Opening balance (actual)", 10000]);
        expect([ws.getCell("A6").value, ws.getCell("B6").value]).toEqual(["2026-08", 14800]);
        expect([ws.getCell("A7").value, ws.getCell("B7").value]).toEqual(["2026-09", 19600]);
    });

    it("writes the accounts, each section read its own way, the Δ a number", async () => {
        const {workbook, download} = await built("accounts", CONTEXT, {accounts: accountsFixture()});
        expect(download.params).toMatch(/^Assets and liabilities: balance at each period's end, then Δ/);
        const ws = workbook.worksheets[0]!;
        expect(headers(ws).some((h) => h.includes(PROJECTED_SUFFIX))).toBe(true);
        expect(headers(ws).at(-1)).toBe("Δ or Total");
        // The Δ stays a real number; the "Δ" is in its number format.
        const cells: {value: unknown; numFmt: string}[] = [];
        ws.eachRow((row) => row.eachCell((cell) => cells.push({value: cell.value, numFmt: cell.numFmt})));
        const delta = cells.find((cell) => cell.value === 3200 && cell.numFmt.startsWith('"Δ "'));
        expect(delta, "the checking Δ").toBeDefined();
        expect(cells.some((cell) => cell.value === "Assets — balance at period end")).toBe(true);
    });

    it("writes the journal exactly as the search lists it", async () => {
        const journal = journalFixture(2);
        expect(matchingEntries(journal.entries, "  LAPTOP ").map((e) => e.txn.description)).toEqual(["new laptop"]);
        const all = await built("journal", CONTEXT, {journal});
        expect(texts(all.workbook).some((t) => t.includes("new laptop"))).toBe(true);
        expect(texts(all.workbook).some((t) => t.includes("expenses:rent"))).toBe(true);

        const searched = await built("journal", {...CONTEXT, journalQuery: "laptop"}, {journal});
        expect(searched.download.params).toBe('1 generated transaction matching "laptop"');
        const cells = texts(searched.workbook);
        expect(cells.some((t) => t.includes("new laptop"))).toBe(true);
        // The rent entries did not match, so they are not in the file.
        expect(cells.some((t) => t === "projection")).toBe(false);
    });
});
