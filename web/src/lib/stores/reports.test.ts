import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {defaultReportParams} from "$lib/reports/ui/params";
import {budgetSpan, buildReportQuery, reports, sameReportQuery, type ReportQuery} from "./reports.svelte";

/** A minimal engine response for a sectioned (bs/is) report. */
const sectioned = (title: string) => ({
    sections: [{title, rows: [], total: {}}],
    total: {},
});

/** Stub the network so `reports.load` decodes a real payload without an engine. */
function serve(handler: (url: string) => unknown): void {
    vi.stubGlobal("fetch", (input: string) =>
        Promise.resolve(new Response(JSON.stringify(handler(String(input))), {status: 200, headers: {"Content-Type": "application/json"}}))
    );
}

const BS: ReportQuery = {tab: "bs", asOf: "2025-12-31", depth: 2};
const IS: ReportQuery = {tab: "is", from: "2026-01-01", to: "2026-12-31", depth: 2};

describe("UNIT reports store tags each report with the query it came from (FE-1)", () => {
    beforeEach(() => {
        serve(() => sectioned("Assets"));
    });
    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("remembers which query produced the held report", async () => {
        await reports.load("http://engine", BS);
        // Without this the page had only the report's SHAPE to go on — and
        // `bs`/`is` are both SectionedReport, `cf`/`nw` both PeriodReport — so a
        // loaded balance sheet rendered, and exported, under the P&L's label.
        expect(reports.query).toEqual(BS);
        expect(reports.report).not.toBeNull();
    });

    it("keeps the FAILED query's tag pointing at the report actually held", async () => {
        await reports.load("http://engine", BS);
        vi.stubGlobal("fetch", () => Promise.reject(new TypeError("connection refused")));
        await reports.load("http://engine", IS);

        expect(reports.status).toBe("error");
        // The balance sheet is still in hand — that is what made the old
        // `report === null` guard unsatisfiable — but it is still tagged `bs`,
        // so the page can see it does not answer the P&L tab.
        expect(reports.report).not.toBeNull();
        expect(reports.query?.tab).toBe("bs");
        expect(reports.query?.tab).not.toBe(IS.tab);
    });
});

describe("UNIT sameReportQuery gates the export on an exact match", () => {
    it("accepts an identical query", () => {
        expect(sameReportQuery(BS, {tab: "bs", asOf: "2025-12-31", depth: 2})).toBe(true);
    });

    it("rejects a different tab", () => {
        expect(sameReportQuery(BS, IS)).toBe(false);
    });

    it("rejects the same tab at a different as-of", () => {
        // The export names the file and titles the sheet from the CURRENT
        // controls: `balance-sheet-2026-06-30.xlsx` holding December's figures
        // is exactly the failure this prevents.
        expect(sameReportQuery(BS, {tab: "bs", asOf: "2026-06-30", depth: 2})).toBe(false);
    });

    it("rejects a different depth", () => {
        expect(sameReportQuery(BS, {tab: "bs", asOf: "2025-12-31", depth: 3})).toBe(false);
    });

    it("distinguishes budget spans that share an end and a month count", () => {
        // `buildReportQuery` reduces the budget span to end + count, so without
        // carrying `from` two different spans could compare equal and the
        // workbook would be named after a range it does not contain.
        const params = defaultReportParams();
        const a = buildReportQuery({...params, tab: "budget", from: "2026-01-01", to: "2026-12-31"});
        const b = buildReportQuery({...params, tab: "budget", from: "2026-01-15", to: "2026-12-31"});
        expect(sameReportQuery(a, b)).toBe(false);
    });
});

describe("UNIT budgetSpan — the bars' real span", () => {
    // The engine walks whole months back from `end`, so a mid-month `from` makes
    // the first bucket start on the 1st. Measured against a live engine: a bar
    // for from=2026-01-15 reported $720.00 while the journal link, filtered to
    // 2026-01-15, showed $20.00. The link now uses this span instead.
    it("snaps `from` to the first of its month, because the first bucket does", () => {
        expect(budgetSpan("2026-01-15", "2026-01-31")).toEqual({from: "2026-01-01", to: "2026-01-31", count: 1});
    });

    it("leaves an already-aligned from untouched", () => {
        expect(budgetSpan("2026-01-01", "2026-07-25")).toEqual({from: "2026-01-01", to: "2026-07-25", count: 7});
    });

    it("keeps `to` exactly as asked — the engine truncates the last bucket at `end`", () => {
        // Verified against the engine: end=2026-07-25 with a 2026-07-28 txn in the
        // journal reported $30.00, not $530.00. Only the START drifts.
        expect(budgetSpan("2026-07-01", "2026-07-25").to).toBe("2026-07-25");
    });

    it("feeds buildReportQuery, so the fetch and the link agree", () => {
        const q = buildReportQuery({...defaultReportParams(), tab: "budget", from: "2026-01-15", to: "2026-01-31"});
        expect(q).toMatchObject({tab: "budget", end: "2026-01-31", count: 1});
    });
});
