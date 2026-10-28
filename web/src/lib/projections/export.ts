// XLSX downloads for every Projections reading (the user's ask, 2026-10-28):
// the runway, the three statements, Accounts and the generated Journal.
//
// The statements reuse the Reports tab's own workbook builders — they are the
// same report types — with one change: every FORECAST column is labelled
// "(projected)", because a workbook has no divider line and a projected figure
// must never read as an actual one. The Journal reuses the main Journal tab's
// accountant-ready layout (`export/journalXlsx.ts`), so both journals download
// alike and Ledgeline's QuickBooks importer reads either back. The runway has
// a builder of its own in `export/xlsx.ts`.
//
// The Journal's origin label, its "added by rule" test and its search filter
// live here so the screen and the workbook can never disagree about which
// transactions match or what a posting came from.

import type {AmountStyle} from "#lib/domain/types.js";
import {
    buildBalanceSheetWorkbook,
    buildCashFlowWorkbook,
    buildIncomeStatementWorkbook,
    buildAccountReportWorkbook,
    buildRunwayWorkbook,
    downloadWorkbook,
} from "#lib/export/xlsx.js";
import type {ProjectionTab} from "./params";
import type {ForecastEntry, Projection} from "./types";

/** The suffix a forecast column's heading carries in a workbook. */
export const PROJECTED_SUFFIX = " (projected)";

/** `columns` with every one from index `from` on labelled as projected. */
export function markProjected<T extends {label: string}>(columns: readonly T[], from: number): T[] {
    return columns.map((column, at) => (at >= from ? {...column, label: `${column.label}${PROJECTED_SUFFIX}`} : column));
}

/** The rule's period as hledger marks it (`~ monthly`), else what kind of entry this is. */
export function originLabel(entry: ForecastEntry): string {
    if (entry.origin.kind === "event") return "event";
    if (entry.origin.kind === "growth") return "growth";
    return entry.txn.tags.find(([name]) => name === "generated-transaction")?.[1] ?? "rule";
}

/** The `= QUERY` rule that added a posting (hledger's `generated-posting` tag), or `null` for one the scenario wrote. */
export function generatedBy(tags: readonly (readonly [string, string])[]): string | null {
    return tags.find(([name]) => name === "generated-posting")?.[1] ?? null;
}

/**
 * The entries a search matches: one case-insensitive substring of the engine's
 * `haystack`, as the main journal's filter matches — every entry for a blank
 * query.
 */
export function matchingEntries(entries: readonly ForecastEntry[], query: string): readonly ForecastEntry[] {
    const needle = query.trim().toLowerCase();
    return needle === "" ? entries : entries.filter((entry) => entry.haystack.includes(needle));
}

/** What the page knows that a workbook needs and the projection does not carry. */
export interface ExportContext {
    /** The scenario's display name; empty for an unsaved seed. */
    scenarioName: string;
    /** The runway sentence the page shows above the chart. */
    runwaySentence: string;
    /** The commodity the statements are valued in, for the money headings. */
    base: string | null;
    /** The Journal tab's search, so the workbook holds what the screen lists. */
    journalQuery: string;
    /** The display style of a commodity, as the journal workbook writes amounts. */
    styleOf: (commodity: string) => AmountStyle;
}

/** One reading's download: the workbook's title, its subtitle, the filename, and how to build it. */
export interface ProjectionExport {
    title: string;
    params: string;
    filename: string;
    /** Build the workbook (what the tests open). */
    build: () => Promise<import("exceljs").Workbook>;
    /** Build it and hand it to the browser as a download. */
    run: () => Promise<void>;
}

const TAB_TITLES: Record<ProjectionTab, string> = {
    cash: "Projected Runway",
    bs: "Projected Balance Sheet",
    is: "Projected Income Statement",
    cf: "Projected Cash Flow",
    accounts: "Projected Accounts",
    journal: "Projected Journal",
};

const TAB_SLUGS: Record<ProjectionTab, string> = {
    cash: "runway",
    bs: "balance-sheet",
    is: "income-statement",
    cf: "cash-flow",
    accounts: "accounts",
    journal: "journal",
};

/** A filename-safe slug of the scenario's name, or `scenario`. */
export function scenarioSlug(name: string): string {
    const slug = name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 48);
    return slug === "" ? "scenario" : slug;
}

/**
 * The download for `tab`, or `null` when that reading is not on hand (Accounts
 * and Journal ride on the run only while their tab is open, so a projection
 * held from another tab has neither yet).
 */
export function projectionExport(tab: ProjectionTab, projection: Projection, context: ExportContext): ProjectionExport | null {
    const columns = projection.balanceSheet.columns;
    const span = columns.length === 0 ? "" : `-${columns[0]!.from}-to-${columns.at(-1)!.to}`;
    const filename = `projection-${scenarioSlug(context.scenarioName)}-${TAB_SLUGS[tab]}${span}.xlsx`;
    const named = context.scenarioName.trim() === "" ? "" : ` — ${context.scenarioName.trim()}`;
    const title = `${TAB_TITLES[tab]}${named}`;
    const actuals = projection.projectedFrom;
    const window =
        columns.length === 0
            ? ""
            : `${columns[0]!.from} to ${columns.at(-1)!.to}; ${actuals} actual period${actuals === 1 ? "" : "s"}, then projected (columns marked "${PROJECTED_SUFFIX.trim()}")`;
    const meta = (params: string) => ({title, params});
    const finish = (params: string, build: () => Promise<import("exceljs").Workbook>): ProjectionExport => ({
        title,
        params,
        filename,
        build,
        run: async () => downloadWorkbook(await build(), filename),
    });

    switch (tab) {
        case "cash":
            return finish(context.runwaySentence, () =>
                buildRunwayWorkbook(
                    {
                        buckets: projection.buckets,
                        opening: projection.cash.opening,
                        closing: projection.cash.values,
                        openingLabel: "Opening balance (actual)",
                    },
                    meta(context.runwaySentence),
                    context.base
                )
            );
        case "bs":
            return finish(window, () =>
                buildBalanceSheetWorkbook({...projection.balanceSheet, columns: markProjected(projection.balanceSheet.columns, actuals)}, meta(window))
            );
        case "is": {
            const report = projection.incomeStatement;
            const buckets = report.buckets === null ? null : markProjected(report.buckets, actuals);
            return finish(window, () => buildIncomeStatementWorkbook({...report, buckets}, meta(window)));
        }
        case "cf":
            return finish(window, () =>
                buildCashFlowWorkbook({...projection.cashFlow, buckets: markProjected(projection.cashFlow.buckets, actuals)}, meta(window), context.base)
            );
        case "accounts": {
            const accounts = projection.accounts;
            if (accounts === null) return null;
            const params = `Assets and liabilities: balance at each period's end, then Δ; the rest: change in each period, then Total. ${window}`;
            const labels = markProjected(accounts.periods, actuals).map((period) => period.label);
            return finish(params, () => buildAccountReportWorkbook(accounts, meta(params), labels));
        }
        case "journal": {
            const journal = projection.journal;
            if (journal === null) return null;
            const listed = matchingEntries(journal.entries, context.journalQuery);
            const query = context.journalQuery.trim();
            const params = [
                `${listed.length} generated transaction${listed.length === 1 ? "" : "s"}`,
                query === "" ? "" : `matching "${query}"`,
                journal.truncated ? `from the first ${journal.entries.length} of ${journal.total}` : "",
            ]
                .filter((part) => part !== "")
                .join(" ");
            const dates = listed.map((entry) => entry.txn.date);
            return finish(params, async () => {
                // Loaded on demand, as the Journal route loads it, so it never
                // reaches the page's initial bundle.
                const {buildJournalWorkbook} = await import("#lib/export/journalXlsx.js");
                return buildJournalWorkbook(
                    listed.map((entry) => entry.txn),
                    context.styleOf,
                    {
                        title: title,
                        period: dates.length === 0 ? "no transactions" : `${dates[0]} – ${dates.at(-1)} · ${params}`,
                    }
                );
            });
        }
    }
}
