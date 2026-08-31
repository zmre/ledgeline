import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {defaultReportParams} from "$lib/reports/ui/params";
import {GROUPED_BALANCE_SHEET} from "$lib/testing/balanceSheetFixture";
import {GROUPED_INCOME_STATEMENT} from "$lib/testing/incomeStatementFixture";
import {buildReportQuery, reports, sameReportQuery, type ReportQuery} from "./reports.svelte";

/** A minimal engine response for a period (cf/nw) report. */
const period = () => ({buckets: ["2026-01"], rows: [], totals: [{}]});

/**
 * Stub the network so `reports.load` decodes a real payload without an engine.
 *
 * The handler is routed on the URL rather than answering one canned body,
 * because the tabs no longer share a wire shape: `bs` goes to
 * `/balancesheet/grouped` and `is` to `/incomestatement/grouped`, and each
 * decoder REFUSES the other's body — as it should, and as it did the moment
 * this file was left serving the old flat report to both.
 */
function serve(handler: (url: string) => unknown): void {
    vi.stubGlobal("fetch", (input: string) =>
        Promise.resolve(new Response(JSON.stringify(handler(String(input))), {status: 200, headers: {"Content-Type": "application/json"}}))
    );
}

/** The body each report route expects, keyed off the requested path. */
const engine = (url: string): unknown =>
    url.includes("/balancesheet/grouped") ? GROUPED_BALANCE_SHEET : url.includes("/incomestatement/grouped") ? GROUPED_INCOME_STATEMENT : period();

const BS: ReportQuery = {tab: "bs", asOf: "2025-12-31"};
const IS: ReportQuery = {tab: "is", from: "2026-01-01", to: "2026-12-31"};

describe("UNIT reports store tags each report with the query it came from (FE-1)", () => {
    beforeEach(() => {
        serve(engine);
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
        expect(sameReportQuery(BS, {tab: "bs", asOf: "2025-12-31"})).toBe(true);
    });

    it("rejects a different tab", () => {
        expect(sameReportQuery(BS, IS)).toBe(false);
    });

    it("rejects the same tab at a different as-of", () => {
        // The export names the file and titles the sheet from the CURRENT
        // controls: `balance-sheet-2026-06-30.xlsx` holding December's figures
        // is exactly the failure this prevents.
        expect(sameReportQuery(BS, {tab: "bs", asOf: "2026-06-30"})).toBe(false);
    });

    it("rejects a different depth", () => {
        // Neither statement HAS a depth any more (both ask for an unclamped
        // report), so this is pinned on cash flow, which still does.
        const CF: ReportQuery = {tab: "cf", end: "2026-12-31", interval: "monthly", count: 12, depth: 2};
        expect(sameReportQuery(CF, {...CF, depth: 3})).toBe(false);
    });

    it("rejects the same P&L tab over a different range", () => {
        // The export names the file and titles the sheet from the CURRENT
        // controls: `income-statement-2026-01-01-to-2026-12-31.xlsx` holding last
        // year's figures is exactly the failure this prevents.
        expect(sameReportQuery(IS, {tab: "is", from: "2025-01-01", to: "2025-12-31"})).toBe(false);
    });

    it.each([
        ["bs", (params: ReturnType<typeof defaultReportParams>) => buildReportQuery({...params, tab: "bs" as const, depth: 3})],
        ["is", (params: ReturnType<typeof defaultReportParams>) => buildReportQuery({...params, tab: "is" as const, depth: 3})],
    ])("builds the %s query without a depth, whatever the shared field holds", (_tab, build) => {
        // Both statements ask the engine for an UNCLAMPED report — expanding a
        // group has to show all of it — and absent is how those endpoints spell
        // "no clamp" (`depth=0` is already totals-only). A stale `?depth=` from a
        // bookmark still lands in the shared param for cf/nw, and must not leak
        // back into either request.
        const params = defaultReportParams();
        expect(build(params)).not.toHaveProperty("depth");
        expect(build({...params, depth: 1})).toEqual(build({...params, depth: 9}));
    });

    it("builds the bs and is queries from the params each tab actually shows", () => {
        const params = defaultReportParams();
        expect(buildReportQuery({...params, tab: "bs"})).toEqual({tab: "bs", asOf: params.asOf});
        expect(buildReportQuery({...params, tab: "is"})).toEqual({tab: "is", from: params.from, to: params.to});
    });

    it("fetches the P&L from the GROUPED endpoint, whose decoder refuses the flat body", async () => {
        // The two decoders are not interchangeable, and that is the safety net:
        // pointing this tab at the old `/api/reports/incomestatement` again would
        // fail loudly here rather than render a flat report under the new view.
        const seen: string[] = [];
        serve((url) => {
            seen.push(url);
            return engine(url);
        });
        await reports.load("http://engine", IS);

        expect(seen.some((url) => url.includes("/api/reports/incomestatement/grouped"))).toBe(true);
        // No depth, and no `compare`: the engine's default is the previous
        // equal-length window, which is what the comparison column shows.
        expect(seen.every((url) => !url.includes("depth="))).toBe(true);
        expect(reports.report).toMatchObject({kind: "incomeStatement"});
        vi.unstubAllGlobals();
    });
});
