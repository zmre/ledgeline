// xlsx export (WP-07). exceljs is loaded via `await import("exceljs")` ONLY —
// it must never reach the initial bundle (vite splits it into a lazy chunk).
//
// Numbers cross the exact-money display boundary here, but the ROUNDING never
// does: every cell is rounded on the exact `Dec` (half away from zero, the same
// `displayPlaces`/`roundTo` pair the screen uses) and only then converted with
// `toNumber`, so the workbook shows the number the page showed. Handing Excel
// the unrounded float and letting the number format re-round it made the two
// disagree, because that second rounding operates on a BINARY approximation:
// 1005/1e3 is stored as 1.00499999999999989…, which Excel prints as 1.00 while
// the screen prints 1.01.

import {
    dec,
    displayPlaces,
    formatDec,
    MAX_DISPLAY_DECIMALS,
    MAX_QUANTITY_DECIMALS,
    mul,
    roundTo,
    toNumber,
    type Dec,
    type MixedAmount,
} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {isNative} from "#lib/native/detect.js";
import {share} from "#lib/native/share.js";
import type {HoldingsReport} from "#lib/holdings/types.js";
import {primaryValue} from "#lib/reports/budgetSummary.js";
import type {
    AccountReport,
    Amounts,
    BalanceSheetColumns,
    BalanceSheetReport,
    BsSectionKind,
    BudgetBar,
    BudgetReport,
    CashFlowStatement,
    CfColumns,
    IncomeStatementReport,
    IsSectionKind,
    PeriodReport,
    SectionedReport,
} from "#lib/reports/types.js";
import {asColumns, bsColumnSummaries, columnSectionRows} from "#lib/reports/ui/balanceSheetRows.js";
import {compressIsRows, compressPeriodRows, compressSectionRows} from "#lib/reports/ui/displayRows.js";
import {cfDisplayModel, type CfDisplayRow} from "#lib/reports/ui/cashFlowRows.js";
import {amountsPct, isSummary} from "#lib/reports/ui/incomeStatementRows.js";
import {periodTable} from "#lib/reports/ui/periodTable.js";
import {accountDisplaySections, DELTA, summaryHeading} from "#lib/reports/ui/accountReportRows.js";
import {compareCells, type WindowReading} from "#lib/reports/ui/compareColumns.js";
import type {Bridge, PeriodMetrics, RunwayBasis, SaasMonth, SaasReport} from "#lib/reports/saasTypes.js";
import type {Workbook, Worksheet} from "exceljs"; // type-only: erased at build time

const HEADER_ARGB = "FF1E293B";
const XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/**
 * Commodity characters safe to embed in an Excel number-format code (SEC-13).
 *
 * Deliberately an ALLOWLIST. Commodity strings come from the journal — i.e.
 * user-controlled text — and are interpolated into `styles.xml`, but Excel's
 * number-format grammar has a large, poorly-specified metacharacter set:
 * `;` splits the code into positive/negative/zero/text sections, `\` escapes,
 * `[...]` denotes colours and conditions, `_` and `*` take a padding/repeat
 * argument, and `@` is the text placeholder. Enumerating those (a blocklist)
 * is a losing game; enumerating what a real ticker or currency symbol needs is
 * not. Empty is excluded by `+`, so it falls through to the bare format too.
 *
 * NOTE this is intentionally stricter than strictly necessary. The affix is
 * always emitted inside a quoted literal, and `"` is itself outside the
 * allowlist, so nothing can escape the quotes to reach a metacharacter in the
 * first place. The narrow set is defence in depth against Excel's handling of
 * odd characters *within* a literal, which we cannot test here.
 */
const SAFE_COMMODITY = /^[A-Za-z0-9$€£¥.]+$/u;

/**
 * Excel number format for a quantity: grouping + the Dec's decimal places
 * (capped at `maxDecimals`, the money cap by default) + commodity affix.
 *
 * A commodity outside `SAFE_COMMODITY` (or empty) yields the bare numeric
 * format with NO affix. That loses the label — e.g. a quoted hledger commodity
 * like `"MY FUND"` prints unlabelled — but a wrong-but-readable cell beats a
 * malformed `styles.xml`, which Excel reports as an unrecoverable-content
 * repair on the whole workbook.
 */
export function numberFormat(commodity: string, places: number, maxDecimals: number = MAX_DISPLAY_DECIMALS): string {
    const shown = Math.max(0, Math.min(places, maxDecimals));
    const base = shown > 0 ? `#,##0.${"0".repeat(shown)}` : "#,##0";
    if (!SAFE_COMMODITY.test(commodity)) return base;
    // Redundant given the allowlist rejects `"`, but kept so that widening the
    // allowlist later cannot silently reintroduce a quote break-out.
    const quoted = `"${commodity.replace(/"/g, '""')}"`;
    // Single-symbol commodities ($ € £ ¥) read best as prefixes; codes (USD, AAPL) as suffixes.
    return commodity.length === 1 ? `${quoted}${base}` : `${base} ${quoted}`;
}

type Cell = ReturnType<Worksheet["getCell"]>;

/** Ungrouped fixed-point style for the multi-commodity TEXT fallback; `precision` is filled in per Dec. */
const TEXT_STYLE: Omit<AmountStyle, "precision"> = {side: "L", spaced: false, decimalPoint: ".", digitGroups: null};

/**
 * Value + number format for one exact quantity.
 *
 * The Dec is rounded to its displayed places FIRST (exactly, half away from
 * zero) and the format is then pinned to those same places, so Excel has no
 * rounding left to do and cannot disagree with the screen. `maxDecimals` is the
 * money cap for money and MAX_QUANTITY_DECIMALS for unit counts.
 */
function writeDec(cell: Cell, commodity: string, qty: Dec, maxDecimals: number): void {
    const places = displayPlaces(qty, qty.p, maxDecimals);
    cell.value = toNumber(roundTo(qty, places));
    cell.numFmt = numberFormat(commodity, places, places);
}

/**
 * Commodity-labelled text for a list of exact quantities, e.g. `5 GLD, -2 TSLA`.
 *
 * Formatted from the exact Dec, never `toNumber(...).toFixed(...)` — that rounds
 * the binary double and drifts a cent off the screen's string (FE-6).
 */
function amountsText(entries: readonly [string, Dec][]): string {
    return entries.map(([commodity, qty]) => `${formatDec(qty, {...TEXT_STYLE, precision: qty.p})} ${commodity}`).join(", ");
}

/** A MixedAmount's commodities, sorted, so a cell's contents never depend on Map insertion order. */
function sortedEntries(ma: MixedAmount): [string, Dec][] {
    return [...ma.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

/** Write a MixedAmount: single-commodity → real number + numFmt; multi → text fallback; empty → 0. */
function setAmount(cell: Cell, ma: MixedAmount): void {
    const entries = sortedEntries(ma);
    if (entries.length === 1) {
        writeDec(cell, entries[0][0], entries[0][1], MAX_DISPLAY_DECIMALS);
    } else if (entries.length === 0) {
        cell.value = 0;
        cell.numFmt = "#,##0";
    } else {
        cell.value = amountsText(entries);
    }
    cell.alignment = {...cell.alignment, horizontal: "right"};
}

/** A single Dec quantity as a real number + numFmt, right-aligned (setAmount's single-commodity case, sans MixedAmount). */
function setDec(cell: Cell, commodity: string, qty: Dec, maxDecimals: number = MAX_DISPLAY_DECIMALS): void {
    writeDec(cell, commodity, qty, maxDecimals);
    cell.alignment = {...cell.alignment, horizontal: "right"};
}

function addTitleRows(ws: Worksheet, meta: {title: string; params: string}, headers: string[]): void {
    ws.getCell(1, 1).value = meta.title;
    ws.getCell(1, 1).font = {bold: true, size: 14};
    ws.getCell(2, 1).value = meta.params;
    ws.getCell(2, 1).font = {italic: true, size: 10, color: {argb: "FF64748B"}};
    // Row 3 stays blank; row 4 is the styled header row.
    headers.forEach((header, i) => {
        const cell = ws.getCell(4, i + 1);
        cell.value = header;
        cell.font = {bold: true, color: {argb: "FFFFFFFF"}};
        cell.fill = {type: "pattern", pattern: "solid", fgColor: {argb: HEADER_ARGB}};
        if (i > 0) cell.alignment = {horizontal: "right"};
    });
    ws.getColumn(1).width = 40;
    for (let i = 2; i <= headers.length; i += 1) ws.getColumn(i).width = 16;
}

function labelCell(ws: Worksheet, rowIx: number, label: string, indent: number, bold = false): void {
    const cell = ws.getCell(rowIx, 1);
    cell.value = label;
    if (indent > 0) cell.alignment = {indent};
    if (bold) cell.font = {bold: true};
}

// --- Grouped balance sheet (plans/12) ---------------------------------------

/**
 * Section fills, matching the on-screen accents (daisyUI success / warning /
 * info) at a darkness that reads against white with the same white bold text
 * `addTitleRows` uses for the header row.
 */
const BS_SECTION_ARGB: Record<BsSectionKind, string> = {
    assets: "FF14532D",
    liabilities: "FF7C2D12",
    equity: "FF1E3A8A",
};

/** An exact zero at cent precision, so a missing base part still formats as `$0.00` and not `0`. */
const ZERO_MONEY: Dec = dec(0n, 2);

/**
 * One balance-sheet line: a figure per column (one, "Amount", on a single-date
 * sheet), then the "Other commodities" column.
 *
 * The whole report is valued into `base`, so the Amount column is finally a REAL
 * NUMBER with a number format rather than the comma-joined text `setAmount`
 * falls back to. What could NOT be valued — a holding with no `P` directive —
 * goes into its own text column instead of being dropped: "unpriced commodities
 * are surfaced, never silently dropped" is the rule the whole redesign rests on,
 * and a workbook that quietly omitted them would be the worst place to break it.
 *
 * `base === null` (a journal with no base commodity) has no figure to promote,
 * so it degrades to `setAmount`'s existing behaviour.
 */
function setBsAmounts(ws: Worksheet, rowIx: number, figures: readonly MixedAmount[], base: string | null, bold: boolean): void {
    figures.forEach((ma, at) => {
        const cell = ws.getCell(rowIx, at + 2);
        if (base === null) setAmount(cell, ma);
        else setDec(cell, base, ma.get(base) ?? ZERO_MONEY);
        if (bold) cell.font = {bold: true};
    });

    // The NEWEST column's leftovers, as the income statement's interval export
    // writes its Total's: a pair of columns per period would double the sheet's
    // width for commodities the as-of column already names. One column, so
    // this is exactly the single-date sheet's extras.
    const last = figures.at(-1) ?? new Map<string, Dec>();
    const others = sortedEntries(last).filter(([commodity, qty]) => commodity !== base && qty.m !== 0n);
    if (others.length === 0 || base === null) return;
    const extras = ws.getCell(rowIx, figures.length + 2);
    extras.value = amountsText(others);
    extras.alignment = {...extras.alignment, horizontal: "right"};
    if (bold) extras.font = {bold: true};
}

/** A full-width rule above a totals row (the workbook's answer to the screen's `<tfoot>` border). */
function ruleAbove(ws: Worksheet, rowIx: number, style: "thin" | "medium", width = 3): void {
    for (let col = 1; col <= width; col += 1) {
        const cell = ws.getCell(rowIx, col);
        cell.border = {...cell.border, top: {style}};
    }
}

/**
 * The grouped balance sheet: a coloured header per box, a bold row per group
 * with its subtotal, the group's indented accounts beneath it, a ruled section
 * total, then the same tie-out the screen shows — the three section totals,
 * `Liabilities + equity` against `Total assets`, the verdict, and net worth as
 * its own figure below.
 *
 * Assets and Liabilities may be banded CURRENT / NON-CURRENT: a subheading row
 * above the band's first group, a ruled subtotal below its last. The rows are
 * the view's own (`columnSectionRows`), so the workbook cannot split its bands at
 * a different group from the page — and the heading and subtotal prose is the engine's, so it
 * cannot word them differently either. A journal that tags nothing produces
 * neither row, exactly as it produces neither on screen.
 *
 * `bsColumnSummaries` is shared with the view precisely so a workbook cannot
 * claim a different `Liabilities + equity` (or a different verdict) from the
 * page it was exported from.
 *
 * One template for both shapes, as on screen: a single-date sheet is a
 * one-column `BalanceSheetColumns` (`asColumns`), and lays out exactly as it
 * always did. A multi-period sheet gets one figure column per period end.
 *
 * Deliberately UNLIKE the screen in exactly one way: every group is written out
 * in full, whatever is collapsed in the UI. A disclosure triangle is a way to
 * read a long statement on a screen; an exported statement missing the accounts
 * the reader happened to have closed is just an incomplete document.
 */
function addBalanceSheet(ws: Worksheet, sheet: BalanceSheetColumns): void {
    // Label, one figure per column, then the extras column.
    const width = sheet.columns.length + 2;
    const put = (rowIx: number, figures: readonly MixedAmount[], bold: boolean): void => setBsAmounts(ws, rowIx, figures, sheet.base, bold);
    let rowIx = 5;
    for (const section of sheet.sections) {
        for (let col = 1; col <= width; col += 1) {
            const cell = ws.getCell(rowIx, col);
            cell.font = {bold: true, color: {argb: "FFFFFFFF"}};
            cell.fill = {type: "pattern", pattern: "solid", fgColor: {argb: BS_SECTION_ARGB[section.kind]}};
        }
        ws.getCell(rowIx, 1).value = section.title;
        rowIx += 1;

        // A banded section pushes its groups one level right, so the workbook's
        // only structural language — indentation — says what typography says on
        // screen: heading over group over account. An unbanded section is
        // untouched, and its groups stay exactly where they have always been.
        const groupIndent = section.subsections.length > 0 ? 2 : 1;

        // The screen's own rows with EVERY group open (see above): the same
        // bands, the same single-child compression, the same engine subtotals.
        for (const row of columnSectionRows(section, () => true)) {
            switch (row.kind) {
                case "subsection":
                    labelCell(ws, rowIx, row.label, 1, true);
                    break;
                case "group":
                    labelCell(ws, rowIx, row.label, groupIndent, true);
                    put(rowIx, row.amount, true);
                    break;
                case "account":
                    // `row.indent` is the compression indent plus one, below its group.
                    labelCell(ws, rowIx, row.label, row.indent + groupIndent);
                    put(rowIx, row.amount, false);
                    break;
                case "subtotal":
                    ruleAbove(ws, rowIx, "thin", width);
                    // The engine's subtotal, at the subheading's own indent so the
                    // band reads as opened and closed by the same level.
                    labelCell(ws, rowIx, row.label, 1, true);
                    put(rowIx, row.amount, true);
                    break;
            }
            rowIx += 1;
        }

        ruleAbove(ws, rowIx, "thin", width);
        labelCell(ws, rowIx, `Total ${section.title}`, 0, true);
        put(rowIx, section.total, true);
        rowIx += 2; // a blank row between boxes, as the screen puts a gap between them
    }

    const summaries = bsColumnSummaries(sheet);
    const across = (pick: (summary: (typeof summaries)[number]) => MixedAmount): MixedAmount[] => summaries.map(pick);
    ruleAbove(ws, rowIx, "medium", width);
    for (const [label, figures] of [
        ["Total Assets", across((summary) => summary.assets)],
        ["Total Liabilities", across((summary) => summary.liabilities)],
        ["Total Equity", across((summary) => summary.equity)],
    ] as const) {
        labelCell(ws, rowIx, label, 0);
        put(rowIx, figures, false);
        rowIx += 1;
    }

    // The tie-out proper. `Liabilities + equity` is summed from the exact Decs
    // by `bsColumnSummaries`, never by re-adding the rounded cells above it.
    ruleAbove(ws, rowIx, "thin", width);
    labelCell(ws, rowIx, "Liabilities + Equity", 0, true);
    put(
        rowIx,
        across((summary) => summary.liabilitiesPlusEquity),
        true
    );
    rowIx += 1;
    labelCell(ws, rowIx, "Total Assets", 0, true);
    put(
        rowIx,
        across((summary) => summary.assets),
        true
    );
    rowIx += 1;

    // The engine's verdict per column, taken as given (the same summaries the
    // screen renders). When any column fails, the row carries every column's
    // exact residue so the reader can go looking for it.
    if (summaries.every((summary) => summary.balanced)) {
        labelCell(ws, rowIx, "Balanced", 0, true);
        ws.getCell(rowIx, 1).font = {bold: true, color: {argb: "FF15803D"}};
    } else {
        labelCell(ws, rowIx, "Out of balance (assets − liabilities − equity)", 0, true);
        put(rowIx, sheet.check, true);
        for (let col = 1; col <= width; col += 1) ws.getCell(rowIx, col).font = {bold: true, color: {argb: "FFB45309"}};
    }
}

// --- Grouped income statement (plans/13) ------------------------------------

/**
 * Section fills, matching the on-screen accents at a darkness that reads against
 * white with the same white bold text `addTitleRows` uses for the header row.
 * Revenue is the one green box; every cost section is in the warm half of the
 * wheel, and the two genuinely different ones (`other`, which is signed, and
 * `depreciation`, which is non-cash) are visibly apart from both.
 */
const IS_SECTION_ARGB: Record<IsSectionKind, string> = {
    revenue: "FF14532D",
    cogs: "FF7C2D12",
    opex: "FF7F1D1D",
    depreciation: "FF581C87",
    interest: "FF134E4A",
    tax: "FF78350F",
    other: "FF1E3A8A",
};

/** Which of a line's figures a column prints. */
type Pick = (amounts: Amounts) => MixedAmount | undefined;

/** One figure column: where it goes, and which figure it prints. */
interface IsColumn {
    col: number;
    pick: Pick;
}

/**
 * Column layout, which depends on the report's mode.
 *
 * Computed ONCE per workbook rather than open-coded per row: the prior column
 * exists only when there is a prior period and interval mode has a column per
 * bucket, so every column after them shifts, and a hand-numbered
 * `getCell(row, 4)` somewhere would silently write the percentage into the
 * extras column on half the reports.
 *
 * - Total mode, comparing: Amount · Prior · % change · extras · prior extras.
 * - Total mode, not comparing: Amount · extras (no prior, so no % change).
 * - Interval mode: one column per bucket · Total · extras — no prior and no
 *   percentage, exactly as on screen. The extras column is the TOTAL's
 *   leftovers: a pair of columns per bucket would double the sheet's width for
 *   commodities the Total already names.
 */
interface IsColumns {
    label: number;
    money: IsColumn[];
    /** The % change column, when the report compares. */
    pct: number | null;
    /** Unconverted leftovers, one column per money column that has its own. */
    extras: IsColumn[];
    count: number;
}

const current: Pick = (amounts) => amounts.current;
const prior: Pick = (amounts) => amounts.prior;

function isColumns(report: IncomeStatementReport): IsColumns {
    if (report.buckets !== null) {
        const total = report.buckets.length + 2;
        return {
            label: 1,
            money: [...report.buckets.map((_, i): IsColumn => ({col: i + 2, pick: (amounts) => amounts.values?.[i]})), {col: total, pick: current}],
            pct: null,
            extras: [{col: total + 1, pick: current}],
            count: total + 1,
        };
    }
    return report.prior !== null
        ? {
              label: 1,
              money: [
                  {col: 2, pick: current},
                  {col: 3, pick: prior},
              ],
              pct: 4,
              extras: [
                  {col: 5, pick: current},
                  {col: 6, pick: prior},
              ],
              count: 6,
          }
        : {label: 1, money: [{col: 2, pick: current}], pct: null, extras: [{col: 3, pick: current}], count: 3};
}

/**
 * One income-statement figure across its money / % change / extras columns.
 *
 * The report is valued into `base`, so the money columns are REAL NUMBERS with a
 * number format rather than the comma-joined text `setAmount` falls back to, and
 * the percentage is a real number with Excel's own percent format (which
 * multiplies by 100 — hence `/100`) rather than a pre-rendered "12.3%" string.
 * A reader can therefore sort, chart and re-total the sheet.
 *
 * `pctChange` is the same function the screen calls, so the workbook cannot
 * round a percentage the other way from the page it came from. `null` leaves the
 * cell EMPTY: there is no percentage change from a zero prior, and `0.0%` would
 * be a claim rather than a blank.
 */
function setIsAmounts(ws: Worksheet, rowIx: number, cols: IsColumns, amounts: Amounts, pct: number | null, base: string | null, bold: boolean): void {
    const write = (col: number, ma: MixedAmount): void => {
        const cell = ws.getCell(rowIx, col);
        if (base === null) setAmount(cell, ma);
        else setDec(cell, base, ma.get(base) ?? ZERO_MONEY);
        if (bold) cell.font = {bold: true};
    };
    for (const {col, pick} of cols.money) {
        const ma = pick(amounts);
        if (ma !== undefined) write(col, ma);
    }

    if (cols.pct !== null && pct !== null) {
        const cell = ws.getCell(rowIx, cols.pct);
        // `round(pct × 10) / 1000`, not `pct / 100` — Excel's % format multiplies
        // by 100, and the naive division compounds the error already in `pct`:
        // 99.9/100 stores 0.9990000000000001, which is a spreadsheet cell a
        // reader can see is wrong the moment they widen the column. Going via the
        // integer tenths the percentage already IS lands on the nearest double to
        // 0.999, which prints as 0.999.
        cell.value = Math.round(pct * 10) / 1000;
        cell.numFmt = "0.0%";
        cell.alignment = {...cell.alignment, horizontal: "right"};
        if (bold) cell.font = {bold: true};
    }

    // What the valuation could NOT convert, in its own column rather than
    // dropped — the rule the whole redesign rests on, and a workbook that quietly
    // omitted them would be the worst place to break it. Each period's leftovers
    // land in that period's OWN column, matching the screen, which hangs a
    // figure's extras under the column the figure is in (current under Amount,
    // prior under the dated prior column). Concatenating both into one cell read
    // "5 GLD, 3 GLD" — neither figure attributed to a window, and the pair
    // indistinguishable from a typo'd duplicate.
    if (base === null) return;
    const writeExtras = (col: number, ma: MixedAmount | undefined): void => {
        const leftovers = sortedEntries(ma ?? new Map()).filter(([commodity, qty]) => commodity !== base && qty.m !== 0n);
        if (leftovers.length === 0) return;
        const extras = ws.getCell(rowIx, col);
        extras.value = amountsText(leftovers);
        extras.alignment = {...extras.alignment, horizontal: "right"};
        if (bold) extras.font = {bold: true};
    };
    for (const {col, pick} of cols.extras) writeExtras(col, pick(amounts));
}

/**
 * The grouped income statement: a coloured header per box, a bold row per group
 * with its subtotal, the group's indented accounts beneath it, a ruled section
 * total, the ladder lines between boxes, then net income.
 *
 * `isSummary` and `amountsPct` are shared with the view precisely so a workbook
 * cannot claim a different bottom line — or a different percentage — from the
 * page it was exported from.
 *
 * Deliberately UNLIKE the screen in exactly one way: every group is written out
 * in full, whatever is collapsed in the UI. A disclosure triangle is a way to
 * read a long statement on a screen; an exported statement missing the accounts
 * the reader happened to have closed is just an incomplete document.
 */
function addIncomeStatement(ws: Worksheet, report: IncomeStatementReport, cols: IsColumns): void {
    const base = report.base;
    const pct = (amounts: Amounts): number | null => amountsPct(amounts, base);
    const columns = Array.from({length: cols.count}, (_, i) => i + 1);
    const rule = (rowIx: number, style: "thin" | "medium"): void => {
        for (const col of columns) {
            const cell = ws.getCell(rowIx, col);
            cell.border = {...cell.border, top: {style}};
        }
    };

    let rowIx = 5;
    for (const section of report.sections) {
        for (const col of columns) {
            const cell = ws.getCell(rowIx, col);
            cell.font = {bold: true, color: {argb: "FFFFFFFF"}};
            cell.fill = {type: "pattern", pattern: "solid", fgColor: {argb: IS_SECTION_ARGB[section.kind]}};
        }
        ws.getCell(rowIx, cols.label).value = section.title;
        rowIx += 1;

        for (const group of section.groups) {
            labelCell(ws, rowIx, group.name, 1, true);
            setIsAmounts(ws, rowIx, cols, group.total, pct(group.total), base, true);
            rowIx += 1;
            // The same compression the screen applies, so a single-child chain is
            // one row in both places.
            for (const {label, indent, row} of compressIsRows(group.rows)) {
                labelCell(ws, rowIx, label, indent + 2);
                setIsAmounts(ws, rowIx, cols, row.amounts, pct(row.amounts), base, false);
                rowIx += 1;
            }
        }

        rule(rowIx, "thin");
        labelCell(ws, rowIx, `Total ${section.title}`, 0, true);
        setIsAmounts(ws, rowIx, cols, section.total, pct(section.total), base, true);
        rowIx += 1;

        // The ladder, between the boxes exactly as on screen — a subtotal spans
        // everything above it, so writing it inside a box would claim otherwise.
        for (const subtotal of section.trailing) {
            rowIx += 1; // a blank row, so the rung reads as separate from the box
            rule(rowIx, "medium");
            labelCell(ws, rowIx, subtotal.label, 0, true);
            setIsAmounts(ws, rowIx, cols, subtotal.total, pct(subtotal.total), base, true);
            rowIx += 1;
        }
        rowIx += 1; // a blank row between boxes, as the screen puts a gap between them
    }

    // The bottom line, and nothing else. A condensed restatement of every
    // section's total stood here and went with the screen's: each of those
    // figures is already a `Total …` row above, so the block was seven duplicated
    // totals in a document whose whole point is that nothing is printed twice.
    const summary = isSummary(report);
    rule(rowIx, "medium");
    labelCell(ws, rowIx, "Net income (revenue − expenses)", 0, true);
    setIsAmounts(ws, rowIx, cols, summary.netIncome, summary.netPct, base, true);
}

/**
 * The FLAT sectioned report: same compressed rows the UI shows, one Amount
 * column. No tab produces one any more (both statements moved to their grouped
 * shapes), but it is still the type the hledger parity goldens decode into.
 */
function addSectioned(ws: Worksheet, report: SectionedReport): void {
    let rowIx = 5;
    for (const section of report.sections) {
        labelCell(ws, rowIx, section.title, 0, true);
        rowIx += 1;
        for (const {label, indent, row} of compressSectionRows(section.rows)) {
            labelCell(ws, rowIx, label, indent + 1);
            setAmount(ws.getCell(rowIx, 2), row.inclusive);
            rowIx += 1;
        }
        labelCell(ws, rowIx, `Total ${section.title}`, 0, true);
        setAmount(ws.getCell(rowIx, 2), section.total);
        ws.getCell(rowIx, 2).font = {bold: true};
        rowIx += 1;
    }
    labelCell(ws, rowIx, "Net", 0, true);
    setAmount(ws.getCell(rowIx, 2), report.grandTotal);
    ws.getCell(rowIx, 2).font = {bold: true};
}

/**
 * Cash flow / net worth: one column per bucket (plus cash flow's Total) and a
 * bold Net totals row — the columns `periodTable` gives the screen.
 */
function addPeriod(ws: Worksheet, report: PeriodReport): void {
    const columns = periodTable(report);
    let rowIx = 5;
    for (const {label, indent, row} of compressPeriodRows(report.rows)) {
        labelCell(ws, rowIx, label, indent);
        columns.cells(row).forEach((value, i) => setAmount(ws.getCell(rowIx, i + 2), value));
        rowIx += 1;
    }
    labelCell(ws, rowIx, "Net", 0, true);
    columns.net.forEach((total, i) => {
        const cell = ws.getCell(rowIx, i + 2);
        setAmount(cell, total);
        cell.font = {bold: true};
    });
}

/** Set a "% of budget" cell (spent/budget as a fraction; Excel's % format multiplies by 100). */
function setPct(cell: Cell, spent: number | null, budget: number | null, bold = false): void {
    if (spent === null || budget === null || budget === 0) return;
    cell.value = spent / budget;
    cell.numFmt = "0%";
    cell.alignment = {...cell.alignment, horizontal: "right"};
    if (bold) cell.font = {bold: true};
}

/** One Spent/Budget/Remaining/% row from an engine bar (all magnitudes already). */
function addBudgetRow(ws: Worksheet, rowIx: number, label: string, bar: BudgetBar, indent: number, bold = false): void {
    labelCell(ws, rowIx, label, indent, bold);
    setAmount(ws.getCell(rowIx, 2), bar.actual);
    setAmount(ws.getCell(rowIx, 3), bar.goal);
    setAmount(ws.getCell(rowIx, 4), bar.remaining);
    setPct(ws.getCell(rowIx, 5), primaryValue(bar.actual), primaryValue(bar.goal), bold);
    if (bold) for (const col of [2, 3, 4]) ws.getCell(rowIx, col).font = {bold: true};
}

/**
 * Budget summary: Spent / Budget / Remaining / % of budget per leaf category,
 * split into the SAME two sections the page renders (the engine's
 * `BudgetReport.summary`: Income then Expenses), each with its own total.
 *
 * There is deliberately no single grand total. Income and expenses are shown in
 * magnitude, so one sum over both columns is |income| + |expenses| — a figure
 * that appears nowhere on screen and means nothing (its "% of budget" even
 * less). Two sections, two totals, exactly as the page shows them.
 */
function addBudget(ws: Worksheet, report: BudgetReport): void {
    let rowIx = 5;
    for (const section of report.summary.sections) {
        const title = section.kind === "revenue" ? "Income" : "Expenses";
        labelCell(ws, rowIx, title, 0, true);
        rowIx += 1;
        for (const bar of section.bars) {
            addBudgetRow(ws, rowIx, bar.account, bar, 1);
            rowIx += 1;
        }
        addBudgetRow(ws, rowIx, `Total ${title}`, section.overall, 0, true);
        rowIx += 1;
    }
}

/** Build the workbook (exported separately so tests can read it back without a DOM). */
export async function buildWorkbook(report: SectionedReport | PeriodReport, meta: {title: string; params: string}): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    const headers = "sections" in report ? ["Account", "Amount"] : ["Account", ...periodTable(report).headers];
    addTitleRows(ws, meta, headers);
    if ("sections" in report) addSectioned(ws, report);
    else addPeriod(ws, report);
    return workbook;
}

/**
 * Grouped balance-sheet workbook: three coloured boxes, the `Liabilities +
 * equity` vs `Total assets` tie-out with its verdict, and net worth.
 *
 * A third column carries the commodities the valuation could not convert, so
 * the Amount column can stay a real number throughout (see `setBsAmount`). The
 * header rows are frozen, because a balance sheet is read by scrolling down
 * through the sections and the column headings are what say which number is
 * which.
 */
export async function buildBalanceSheetWorkbook(report: BalanceSheetReport | BalanceSheetColumns, meta: {title: string; params: string}): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    const sheet = report.kind === "balanceSheet" ? asColumns(report) : report;
    const inBase = sheet.base === null ? "" : ` (${sheet.base})`;
    // A single-date sheet keeps its one "Amount" column. A multi-period one is
    // headed by each period, and its extras column says which column's
    // leftovers it holds (see `setBsAmounts`).
    const last = sheet.columns.at(-1)?.label ?? "";
    const headers =
        report.kind === "balanceSheet"
            ? ["Account", `Amount${inBase}`, "Other commodities"]
            : ["Account", ...sheet.columns.map((column) => `${column.label}${inBase}`), `Other commodities (${last})`];
    addTitleRows(ws, meta, headers);
    ws.getColumn(headers.length).width = 24; // "5 GLD, -2 TSLA" needs more room than a money column
    ws.views = [{state: "frozen", ySplit: 4}];
    addBalanceSheet(ws, sheet);
    return workbook;
}

/**
 * Grouped income-statement workbook: ladder-ordered coloured boxes, the ruled
 * subtotals between them, and net income — in the screen's column layout.
 *
 * The Prior column exists only when the report compares — a blank column headed
 * "Prior" would read as "the prior period was zero" rather than as "there is no
 * prior period" — and interval mode has a column per bucket plus a Total, so
 * every column after them shifts, which is why the layout is computed once in
 * `isColumns` and threaded through.
 *
 * The header rows are frozen, because an income statement is read by scrolling
 * down through the sections and the column headings are what say which number is
 * which.
 */
export async function buildIncomeStatementWorkbook(report: IncomeStatementReport, meta: {title: string; params: string}): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    const cols = isColumns(report);
    const inBase = report.base === null ? "" : ` (${report.base})`;
    // The prior column is headed with its own DATES, not "Prior": the window is
    // the previous equal-length one, which is not a period a reader can infer
    // from the range in the title — and a mislabelled comparison column is worse
    // than none.
    const priorHeader = report.prior === null ? [] : [`${report.prior.from} … ${report.prior.to}`];
    // Extras split per period exactly as the amounts do (see `setIsAmounts`), so
    // the prior period's column repeats the prior window's dates — two columns
    // both headed "Other commodities" would put the ambiguity the split removes
    // from the cells straight back into the headers.
    const priorExtrasHeader = report.prior === null ? [] : [`Other commodities (${report.prior.from} … ${report.prior.to})`];
    const headers =
        report.buckets !== null
            ? ["Account", ...report.buckets.map((bucket) => bucket.label), `Total${inBase}`, "Other commodities"]
            : report.prior !== null
              ? ["Account", `Amount${inBase}`, ...priorHeader, "% change", "Other commodities", ...priorExtrasHeader]
              : ["Account", `Amount${inBase}`, "Other commodities"];
    addTitleRows(ws, meta, headers);
    for (const {col} of cols.extras) ws.getColumn(col).width = 24; // "5 GLD, -2 TSLA" needs more room than a money column
    ws.views = [{state: "frozen", ySplit: 4}];
    addIncomeStatement(ws, report, cols);
    return workbook;
}

/**
 * Statement of cash flows workbook (plans/27): the screen's own rows, from the
 * same display model, so the workbook shows exactly what the page shows. Every
 * line is opened, because a workbook has no disclosure to click; the cash
 * detail is there as on screen.
 *
 * One money column per period plus Total (or a single Amount), in `base`.
 * Anything in another commodity goes to an "Other commodities" column for the
 * Total, rather than being dropped.
 */
export async function buildCashFlowWorkbook(report: CashFlowStatement, meta: {title: string; params: string}, base: string | null): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    const model = cfDisplayModel(report, () => true);
    const inBase = base === null ? "" : ` (${base})`;
    const periods = model.buckets?.map((bucket) => bucket.label) ?? [];
    const headers = ["Account", ...periods, periods.length > 0 ? `Total${inBase}` : `Amount${inBase}`, "Other commodities"];
    addTitleRows(ws, meta, headers);
    const extrasCol = headers.length;
    ws.getColumn(extrasCol).width = 24;
    ws.views = [{state: "frozen", ySplit: 4}];

    let rowIx = 5;
    const write = (label: string, indent: number, columns: CfColumns, bold: boolean): void => {
        labelCell(ws, rowIx, label, indent, bold);
        const figures = periods.length > 0 ? [...columns.values, columns.total] : [columns.total];
        figures.forEach((ma, i) => {
            const cell = ws.getCell(rowIx, i + 2);
            if (base === null) setAmount(cell, ma);
            else setDec(cell, base, ma.get(base) ?? ZERO_MONEY);
            if (bold) cell.font = {bold: true};
        });
        if (base !== null) {
            const leftovers = sortedEntries(columns.total).filter(([commodity, qty]) => commodity !== base && qty.m !== 0n);
            if (leftovers.length > 0) ws.getCell(rowIx, extrasCol).value = amountsText(leftovers);
        }
        rowIx += 1;
    };
    const writeRows = (rows: readonly CfDisplayRow[], indent: number): void => {
        for (const row of rows) write(row.label, indent + row.indent, row.amounts, false);
    };

    for (const box of model.boxes) {
        labelCell(ws, rowIx, box.title, 0, true);
        rowIx += 1;
        writeRows(box.rows, 1);
        write(box.totalLabel, 0, box.total, true);
        rowIx += 1;
    }
    if (model.exchange !== null) write("Effect of exchange rates on cash", 0, model.exchange, true);
    write("Net change in cash", 0, model.netChange, true);
    rowIx += 1;
    for (const block of [model.beginning, model.ending]) {
        write(block.title, 0, block.total, true);
        writeRows(block.rows, 1);
    }
    return workbook;
}

/**
 * Holdings workbook: one row per holding mirroring the UI table (Name …
 * Gain %), then a bold totals row with values ONLY in Basis and Market value
 * — the engine's honest totals, never recomputed (basis blank when any
 * holding is tainted or unpriced). Nulls are empty cells; gain % is stored
 * as gainPct/100 with a real Excel percent format (which multiplies by 100).
 */
export async function buildHoldingsWorkbook(report: HoldingsReport, meta: {title: string; params: string}): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    addTitleRows(ws, meta, ["Name", "Symbol", "Shares", "Basis", "First basis", "Price", "Price date", "Market value", "Gain", "Gain %"]);

    let rowIx = 5;
    for (const h of report.holdings) {
        ws.getCell(rowIx, 1).value = h.name;
        ws.getCell(rowIx, 2).value = h.symbol;
        // Shares are a unit count, not money — same non-money cap the table uses.
        setDec(ws.getCell(rowIx, 3), "", h.shares, MAX_QUANTITY_DECIMALS);
        if (h.basis !== null) setDec(ws.getCell(rowIx, 4), report.base, h.basis);
        if (h.firstBasisDate !== null) ws.getCell(rowIx, 5).value = h.firstBasisDate;
        if (h.price !== null) {
            setDec(ws.getCell(rowIx, 6), report.base, h.price.qty);
            ws.getCell(rowIx, 7).value = h.price.date;
        }
        if (h.marketValue !== null) setDec(ws.getCell(rowIx, 8), report.base, h.marketValue);
        if (h.gain !== null) setDec(ws.getCell(rowIx, 9), report.base, h.gain);
        if (h.gainPct !== null) {
            const cell = ws.getCell(rowIx, 10);
            cell.value = h.gainPct / 100; // Excel's % format multiplies by 100
            cell.numFmt = "+0.0%;-0.0%";
            cell.alignment = {...cell.alignment, horizontal: "right"};
        }
        rowIx += 1;
    }

    labelCell(ws, rowIx, `Total (${report.holdings.length} holdings)`, 0, true);
    if (report.totals.basis !== null) {
        setDec(ws.getCell(rowIx, 4), report.base, report.totals.basis);
        ws.getCell(rowIx, 4).font = {bold: true};
    }
    setDec(ws.getCell(rowIx, 8), report.base, report.totals.marketValue);
    ws.getCell(rowIx, 8).font = {bold: true};
    return workbook;
}

/** Budget workbook: one row per revenue/expense leaf (Spent/Budget/Remaining/% of budget) and a bold total. */
export async function buildBudgetWorkbook(report: BudgetReport, meta: {title: string; params: string}): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    addTitleRows(ws, meta, ["Account", "Spent", "Budget", "Remaining", "% of budget"]);
    addBudget(ws, report);
    return workbook;
}
/**
 * The Projections runway as a workbook (plans/30 follow-up): the cash the
 * scenario leaves at the end of each FORECAST period, after the real opening
 * balance it starts from. The runway sentence the page shows above the chart
 * travels in the title row's params, where a reader of the file looks first.
 */
export async function buildRunwayWorkbook(
    runway: {buckets: readonly string[]; opening: MixedAmount; closing: readonly MixedAmount[]; openingLabel: string},
    meta: {title: string; params: string},
    base: string | null
): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    addTitleRows(ws, meta, ["Period", base === null ? "Cash" : `Cash (${base})`]);
    ws.views = [{state: "frozen", ySplit: 4}];
    labelCell(ws, 5, runway.openingLabel, 0, true);
    setAmount(ws.getCell(5, 2), runway.opening);
    runway.buckets.forEach((label, i) => {
        labelCell(ws, 6 + i, label, 0);
        setAmount(ws.getCell(6 + i, 2), runway.closing[i] ?? new Map());
    });
    return workbook;
}

/**
 * The Accounts report as a workbook (2026-10-28): the screen's sections, rows
 * and folded chains (`accountDisplaySections`), one money column per period
 * and a last column per section — the Δ over the window for assets and
 * liabilities, every figure in it prefixed "Δ" in its number format (so it
 * stays a real number), and the Total for the rest. Each section's heading
 * row says how its columns read.
 */
export async function buildAccountReportWorkbook(
    report: AccountReport,
    meta: {title: string; params: string},
    columnLabels: readonly string[] = report.periods.map((period) => period.label)
): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    addTitleRows(ws, meta, ["Account", ...columnLabels, `${DELTA} or Total`]);
    ws.views = [{state: "frozen", ySplit: 4}];
    const last = columnLabels.length + 2;
    const delta = (cell: Cell, ma: MixedAmount): void => {
        setAmount(cell, ma);
        if (typeof cell.value === "number") cell.numFmt = `"${DELTA} "${cell.numFmt}`;
        else cell.value = `${DELTA} ${String(cell.value)}`;
    };
    let rowIx = 5;
    for (const section of accountDisplaySections(report)) {
        labelCell(ws, rowIx, `${section.title} — ${section.reading === "balance" ? "balance at period end" : "change in period"}`, 0, true);
        const heading = ws.getCell(rowIx, last);
        heading.value = summaryHeading(section.reading);
        heading.font = {bold: true};
        heading.alignment = {horizontal: "right"};
        rowIx += 1;
        const write = (label: string, indent: number, values: readonly MixedAmount[], summary: MixedAmount, bold: boolean): void => {
            labelCell(ws, rowIx, label, indent, bold);
            values.forEach((value, i) => setAmount(ws.getCell(rowIx, i + 2), value));
            if (section.reading === "balance") delta(ws.getCell(rowIx, last), summary);
            else setAmount(ws.getCell(rowIx, last), summary);
            if (bold) for (let col = 2; col <= last; col += 1) ws.getCell(rowIx, col).font = {bold: true};
            rowIx += 1;
        };
        for (const {label, indent, row} of section.rows) write(label, indent + 1, row.values, row.summary, false);
        write(`Total ${section.title.toLowerCase()}`, 0, section.totals, section.summary, true);
        rowIx += 1;
    }
    return workbook;
}

export async function exportAccountReportXlsx(report: AccountReport, meta: {title: string; params: string}, filename: string): Promise<void> {
    await downloadWorkbook(await buildAccountReportWorkbook(report, meta), filename);
}

// ---------------------------------------------------------------------------
// Compare vs actuals (plans/34)
// ---------------------------------------------------------------------------

/** One line of a comparison sheet: a heading (`values` null) or 2N figures read one way. */
export interface CompareSheetRow {
    label: string;
    indent: number;
    bold: boolean;
    values: readonly MixedAmount[] | null;
    reading: WindowReading;
    /** Leave Δ and Δ% empty (the balance sheet's tie-out row). */
    blankDelta?: boolean;
}

/** The suffix an as-projected column's heading carries in a comparison workbook. */
export const AS_PROJECTED_SUFFIX = " (as projected)";

/** A figure in `base` as a real number when that is all it holds, else as text. */
function setCompareMoney(cell: Cell, ma: MixedAmount, base: string | null): void {
    const others = [...ma].some(([commodity, qty]) => commodity !== base && qty.m !== 0n);
    if (base !== null && !others) setDec(cell, base, ma.get(base) ?? ZERO_MONEY);
    else setAmount(cell, ma);
}

/** Δ% as a numeric fraction under `0.0%`, uncapped (the screen caps at ±999%), or an empty cell. */
function setComparePct(cell: Cell, pct: number | null): void {
    if (pct === null) return;
    cell.value = pct / 100;
    cell.numFmt = "0.0%";
    cell.alignment = {...cell.alignment, horizontal: "right"};
}

/**
 * A statement compared with the projection made on its first day: one row per
 * line, the N actual columns, the same N "(as projected)", then Δ and Δ %
 * (`compareCells`, the screen's own figures) in place of a Total.
 */
export async function buildComparisonWorkbook(
    rows: readonly CompareSheetRow[],
    columnLabels: readonly string[],
    split: number,
    meta: {title: string; params: string},
    base: string | null
): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    const labels = columnLabels.map((label, at) => (at >= split ? `${label}${AS_PROJECTED_SUFFIX}` : label));
    const inBase = base === null ? "" : ` (${base})`;
    addTitleRows(ws, meta, ["Line", ...labels, `Δ${inBase}`, "Δ %"]);
    ws.views = [{state: "frozen", ySplit: 4, xSplit: 1}];
    rows.forEach((row, i) => {
        const rowIx = 5 + i;
        labelCell(ws, rowIx, row.label, row.indent, row.bold);
        if (row.values === null) return;
        row.values.forEach((value, at) => setCompareMoney(ws.getCell(rowIx, at + 2), value, base));
        if (row.blankDelta !== true) {
            const cells = compareCells(row.values, split, row.reading, base);
            setCompareMoney(ws.getCell(rowIx, row.values.length + 2), cells.delta, base);
            setComparePct(ws.getCell(rowIx, row.values.length + 3), cells.pct);
        }
        if (row.bold) for (let col = 2; col <= row.values.length + 3; col += 1) ws.getCell(rowIx, col).font = {bold: true};
    });
    return workbook;
}

/** The runway compared: Period | Actual | As projected | Δ | Δ %, one row per actual period. */
export async function buildRunwayCompareWorkbook(
    runway: {labels: readonly string[]; opening: MixedAmount; actual: readonly MixedAmount[]; projected: readonly MixedAmount[]},
    meta: {title: string; params: string},
    base: string | null
): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    const inBase = base === null ? "" : ` (${base})`;
    addTitleRows(ws, meta, ["Period", `Actual${inBase}`, `As projected${inBase}`, `Δ${inBase}`, "Δ %"]);
    ws.views = [{state: "frozen", ySplit: 4}];
    labelCell(ws, 5, "Opening balance (actual)", 0, true);
    setCompareMoney(ws.getCell(5, 2), runway.opening, base);
    setCompareMoney(ws.getCell(5, 3), runway.opening, base);
    runway.labels.forEach((label, i) => {
        const rowIx = 6 + i;
        const actual = runway.actual[i] ?? new Map<string, Dec>();
        const projected = runway.projected[i] ?? new Map<string, Dec>();
        const cells = compareCells([actual, projected], 1, "balance", base);
        labelCell(ws, rowIx, label, 0);
        setCompareMoney(ws.getCell(rowIx, 2), actual, base);
        setCompareMoney(ws.getCell(rowIx, 3), projected, base);
        setCompareMoney(ws.getCell(rowIx, 4), cells.delta, base);
        setComparePct(ws.getCell(rowIx, 5), cells.pct);
    });
    return workbook;
}

// --- Startup metrics (plans/37) ---------------------------------------------

/** The sheet names, in workbook order. Customer MRR and By segment are left out rather than written empty. */
export const SAAS_SHEETS = {metrics: "Metrics", periods: "Periods", segments: "By segment", customers: "Customer MRR", definitions: "Definitions"} as const;

/** A percent-units number (62.5) as a real fraction under `0.0%`; null leaves the cell empty. */
function setPctUnits(cell: Cell, pct: number | null): void {
    if (pct === null) return;
    cell.value = pct / 100;
    cell.numFmt = "0.0%";
    cell.alignment = {...cell.alignment, horizontal: "right"};
}

/** A plain number with `places` decimals (counts, ratios, months, days); null leaves the cell empty. */
function setNumber(cell: Cell, value: number | null, places: number): void {
    if (value === null) return;
    cell.value = value;
    cell.numFmt = places > 0 ? `#,##0.${"0".repeat(places)}` : "#,##0";
    cell.alignment = {...cell.alignment, horizontal: "right"};
}

/** One kind of cell a metric row writes. */
type SaasValue =
    | {kind: "money"; value: Dec | null}
    | {kind: "pct"; value: number | null}
    | {kind: "number"; value: number | null; places: number}
    | {kind: "text"; value: string | null};

function setSaasValue(cell: Cell, value: SaasValue, base: string): void {
    switch (value.kind) {
        case "money":
            if (value.value !== null) setDec(cell, base, value.value);
            return;
        case "pct":
            return setPctUnits(cell, value.value);
        case "number":
            return setNumber(cell, value.value, value.places);
        case "text":
            if (value.value !== null) cell.value = value.value;
    }
}

const money = (value: Dec | null): SaasValue => ({kind: "money", value});
const pctUnits = (value: number | null): SaasValue => ({kind: "pct", value});
const count = (value: number | null): SaasValue => ({kind: "number", value, places: 0});
const decimal = (value: number | null, places = 1): SaasValue => ({kind: "number", value, places});

const TWELVE: Dec = dec(12, 0);

/** Gross margin % of one month, or null without COGS or revenue. */
function monthMargin(month: SaasMonth): number | null {
    if (month.cogs === null || month.revenue.m === 0n) return null;
    const revenue = toNumber(month.revenue);
    return ((revenue - toNumber(month.cogs)) / revenue) * 100;
}

/** The Metrics sheet's rows: one per figure, a cell per month. A row nothing in the span supports is left out. */
function saasMonthRows(report: SaasReport): [string, (month: SaasMonth) => SaasValue][] {
    const bridge = (pick: (b: Bridge) => Dec) => (month: SaasMonth) => money(month.bridge === null ? null : pick(month.bridge));
    // A role row only when the engine resolved that role for either half: the
    // months carry these as $0 regardless, and a journal tagging only
    // `kpi: recurring` must not export four rows of zeros.
    const resolved = (pick: (p: PeriodMetrics) => Dec | null): boolean => pick(report.current) !== null || pick(report.previous) !== null;
    const rows: [string, (month: SaasMonth) => SaasValue, boolean][] = [
        ["MRR", (m) => money(m.mrr), true],
        // Blank for a partial month: its MRR is shown (marked partial in the
        // header), but annualising part of a month is not a run-rate.
        ["ARR (MRR × 12)", (m) => money(m.partial ? null : mul(m.mrr, TWELVE)), true],
        ["Usage", (m) => money(m.usage), resolved((p) => p.usage)],
        ["Customers", (m) => count(m.customers), report.available.customers],
        ["New", bridge((b) => b.new), report.available.customers],
        ["Expansion", bridge((b) => b.expansion), report.available.customers],
        ["Contraction", bridge((b) => b.contraction), report.available.customers],
        ["Churn", bridge((b) => b.churn), report.available.customers],
        ["Reactivation", bridge((b) => b.reactivation), report.available.customers],
        ["Other", bridge((b) => b.other), report.available.customers],
        ["Revenue", (m) => money(m.revenue), true],
        ["Gross margin %", (m) => pctUnits(monthMargin(m)), report.available.cogs],
        ["Net burn", (m) => money(m.netBurn), true],
        ["Net burn, adjusted for prepayments", (m) => money(m.netBurnAdjusted), report.months.some((m) => m.netBurnAdjusted !== null)],
        ["Cash", (m) => money(m.cashEnd), true],
        ["Deferred revenue", (m) => money(m.deferredEnd), report.months.some((m) => m.deferredEnd !== null)],
        ["S&M", (m) => money(m.sm), resolved((p) => p.sm)],
        ["R&D", (m) => money(m.rnd), resolved((p) => p.rnd)],
        ["G&A", (m) => money(m.ga), resolved((p) => p.ga)],
    ];
    return rows.filter(([, , shown]) => shown).map(([label, read]) => [label, read]);
}

/** Which burn a figure used, in words. */
function basisText(basis: RunwayBasis | null): SaasValue {
    return {kind: "text", value: basis === null ? null : basis === "adjusted" ? "adjusted burn" : "cash burn"};
}

/** The Periods sheet's rows: every PeriodMetrics figure, labelled. */
const PERIOD_ROWS: [string, (p: PeriodMetrics) => SaasValue][] = [
    ["Latest complete month", (p) => ({kind: "text", value: p.month})],
    ["Complete months", (p) => count(p.months)],
    ["MRR", (p) => money(p.mrr)],
    ["ARR", (p) => money(p.arr)],
    ["Usage", (p) => money(p.usage)],
    ["MRR growth", (p) => pctUnits(p.mrrGrowthPct)],
    ["CMGR", (p) => pctUnits(p.cmgrPct)],
    ["Net new ARR", (p) => money(p.netNewArr)],
    ["New MRR", (p) => money(p.bridge?.new ?? null)],
    ["New logos", (p) => count(p.bridge?.newLogos ?? null)],
    ["Expansion MRR", (p) => money(p.bridge?.expansion ?? null)],
    ["Contraction MRR", (p) => money(p.bridge?.contraction ?? null)],
    ["Churned MRR", (p) => money(p.bridge?.churn ?? null)],
    ["Churned logos", (p) => count(p.bridge?.churnedLogos ?? null)],
    ["Reactivation MRR", (p) => money(p.bridge?.reactivation ?? null)],
    ["Reactivated logos", (p) => count(p.bridge?.reactivatedLogos ?? null)],
    ["Other", (p) => money(p.bridge?.other ?? null)],
    ["Customers", (p) => count(p.customers)],
    ["Customer growth", (p) => pctUnits(p.customerGrowthPct)],
    ["Logo churn", (p) => pctUnits(p.logoChurnPct)],
    ["ARPA", (p) => money(p.arpa)],
    ["ACV", (p) => money(p.acv)],
    ["NRR", (p) => pctUnits(p.nrrPct)],
    ["GRR", (p) => pctUnits(p.grrPct)],
    ["Quick ratio", (p) => decimal(p.quickRatio, 2)],
    ["Top customer share", (p) => pctUnits(p.top1Pct)],
    ["Top 5 customers' share", (p) => pctUnits(p.top5Pct)],
    ["Top 10 customers' share", (p) => pctUnits(p.top10Pct)],
    ["Revenue", (p) => money(p.revenue)],
    ["Gross profit", (p) => money(p.grossProfit)],
    ["Gross margin", (p) => pctUnits(p.grossMarginPct)],
    ["Net burn", (p) => money(p.netBurn)],
    ["Net burn, adjusted for prepayments", (p) => money(p.netBurnAdjusted)],
    ["Net burn, 3-month average", (p) => money(p.netBurnAvg3)],
    ["Net burn adjusted for prepayments, 3-month average", (p) => money(p.netBurnAdjustedAvg3)],
    ["Cash", (p) => money(p.cash)],
    ["Runway basis", (p) => basisText(p.runwayBasis)],
    ["Runway (months)", (p) => decimal(p.runwayMonths)],
    ["Cash-out date", (p) => ({kind: "text", value: p.cashOutDate})],
    ["Burn multiple", (p) => decimal(p.burnMultiple, 2)],
    ["Burn multiple basis", (p) => basisText(p.burnMultipleBasis)],
    ["Rule of 40", (p) => decimal(p.ruleOf40)],
    ["S&M", (p) => money(p.sm)],
    ["R&D", (p) => money(p.rnd)],
    ["G&A", (p) => money(p.ga)],
    ["S&M % of revenue", (p) => pctUnits(p.smPct)],
    ["R&D % of revenue", (p) => pctUnits(p.rndPct)],
    ["G&A % of revenue", (p) => pctUnits(p.gaPct)],
    ["CAC", (p) => money(p.cac)],
    ["CAC payback (months)", (p) => decimal(p.cacPaybackMonths)],
    ["Deferred revenue", (p) => money(p.deferredRevenue)],
    ["Billings", (p) => money(p.billings)],
    ["DSO (days)", (p) => decimal(p.dsoDays)],
    ["DPO (days)", (p) => decimal(p.dpoDays)],
];

/** The Periods rows for customer counts from billings (Amendment A1.2), written only when either half has them. */
const BILLING_PERIOD_ROWS: [string, (p: PeriodMetrics) => SaasValue][] = [
    ["Customers (from billings)", (p) => count(p.billingCustomers?.customers ?? null)],
    ["Customer growth (from billings)", (p) => pctUnits(p.billingCustomers?.customerGrowthPct ?? null)],
    ["New customers (from billings)", (p) => count(p.billingCustomers?.newLogos ?? null)],
    ["Lapsed customers (from billings)", (p) => count(p.billingCustomers?.lapsedLogos ?? null)],
    ["Logo churn (from billings)", (p) => pctUnits(p.billingCustomers?.logoChurnPct ?? null)],
    ["ACV (from billings)", (p) => money(p.billingCustomers?.acv ?? null)],
    ["Under 12 months of history", (p) => ({kind: "text", value: p.billingCustomers === null ? null : p.billingCustomers.windowShort ? "yes" : "no"})],
];

/** Definitions: metric, formula in words, what feeds it, and which tier it needs. */
const DEFINITIONS: [string, string, string, "recurring" | "customers" | "business"][] = [
    ["MRR", "Recurring revenue recognised in the latest complete month", "kpi: recurring", "recurring"],
    ["ARR", "MRR × 12 (run-rate)", "kpi: recurring", "recurring"],
    ["Usage", "Usage revenue in the same month; never part of ARR", "kpi: usage", "recurring"],
    ["Net new ARR", "12 × (MRR at the period's end − MRR in the month before it)", "kpi: recurring", "recurring"],
    ["CMGR", "Compound monthly growth rate of MRR over the period", "kpi: recurring", "recurring"],
    ["Customers", "Customers with MRR above zero in the latest complete month", "kpi: recurring + customer: tags", "customers"],
    [
        "MRR bridge",
        "Per customer, month by month: new, expansion, contraction, churn, reactivation; Other is the rest",
        "kpi: recurring + customer: tags",
        "customers",
    ],
    ["NRR", "MRR now of the customers paying 12 months earlier ÷ their MRR then", "kpi: recurring + customer: tags", "customers"],
    ["GRR", "NRR with each customer capped at their MRR 12 months earlier", "kpi: recurring + customer: tags", "customers"],
    ["ARPA", "Attributed MRR ÷ customers, latest complete month", "kpi: recurring + customer: tags", "customers"],
    ["ACV", "Average of 12 × first-month MRR over the period's new customers", "kpi: recurring + customer: tags", "customers"],
    ["Logo churn", "Customers lost during the period ÷ customers in the month before it", "kpi: recurring + customer: tags", "customers"],
    ["Quick ratio", "(new + expansion + reactivation) ÷ (contraction + churn)", "kpi: recurring + customer: tags", "customers"],
    ["CAC", "Sales and marketing spend ÷ new customers, over the period", "kpi: sales-marketing + customer: tags", "customers"],
    ["CAC payback", "Months of gross profit from new and expansion MRR to repay S&M spend", "kpi: sales-marketing + issection: cogs", "customers"],
    ["Revenue", "All revenue over the period", "type: R", "business"],
    ["Gross margin", "(revenue − cost of goods sold) ÷ revenue", "issection: cogs", "business"],
    ["Net burn", "−(change in cash − financing cash flow)", "type: C, cfsection: financing", "business"],
    ["Runway", "Cash ÷ average burn over the last three complete months: adjusted burn when it exists, else net burn", "type: C", "business"],
    [
        "Net burn, adjusted for prepayments",
        "Net burn + change in deferred revenue − change in receivables: burn as if customers paid when served",
        "type: C + kpi: deferred-revenue / kpi: receivable",
        "business",
    ],
    [
        "Customers (from billings)",
        "Customers billed in the 12 months to the latest complete month, when per-customer MRR is hidden",
        "kpi: recurring / deferred-revenue + customer: on invoices",
        "business",
    ],
    [
        "ACV (from billings)",
        "Billings over the last 12 months ÷ customers billed in them",
        "kpi: recurring / deferred-revenue + customer: on invoices",
        "business",
    ],
    [
        "Logo churn (from billings)",
        "Customers lapsing (12 months without a billing) during the period ÷ billed customers before it",
        "kpi: recurring / deferred-revenue + customer: on invoices",
        "business",
    ],
    [
        "Burn multiple",
        "Net burn ÷ net new ARR, over the period; net burn adjusted for prepayments when the books have deferred revenue or receivables, else cash net burn",
        "type: C + kpi: recurring (+ kpi: deferred-revenue / kpi: receivable)",
        "recurring",
    ],
    [
        "Closed through",
        "The last month with any recurring revenue recorded on or before the span's end; later months are marked (open) and no metric uses them",
        "kpi: recurring",
        "recurring",
    ],
    ["Rule of 40", "Revenue growth % + free cash flow margin %", "type: R, type: C", "business"],
    ["S&M / R&D / G&A %", "Each kind of spend ÷ revenue, over the period", "kpi: sales-marketing / rnd / ga", "business"],
    ["DSO", "Receivables at month end ÷ revenue over the period × days", "kpi: receivable (or a *receivable* asset)", "business"],
    ["DPO", "Payables at month end ÷ expenses over the period × days", "kpi: payable (or a *payable* liability)", "business"],
    ["Deferred revenue", "Deferred revenue balance at the end of the latest complete month", "kpi: deferred-revenue", "business"],
    ["Billings", "Revenue over the period + the change in deferred revenue", "kpi: deferred-revenue", "business"],
];

/**
 * The startup metrics as a workbook (plans/37): Metrics (one column per month
 * across the whole span), Periods (current vs previous), By segment, Customer
 * MRR (needs `detail=customers`), Definitions. Every number is a real number
 * in the base currency, like the other exports.
 */
export async function buildSaasWorkbook(report: SaasReport, meta: {title: string; params: string}): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const base = report.base;
    // An open month (A2: after the last month with recurring revenue booked) is
    // also partial; "(open)" says why no metric uses it.
    const monthHeaders = report.months.map((month) => (month.open ? `${month.month} (open)` : month.partial ? `${month.month} (partial)` : month.month));

    const metrics = workbook.addWorksheet(SAAS_SHEETS.metrics);
    addTitleRows(metrics, meta, ["Metric", ...monthHeaders]);
    metrics.views = [{state: "frozen", ySplit: 4, xSplit: 1}];
    saasMonthRows(report).forEach(([label, read], i) => {
        labelCell(metrics, 5 + i, label, 0);
        report.months.forEach((month, at) => setSaasValue(metrics.getCell(5 + i, at + 2), read(month), base));
    });

    const periods = workbook.addWorksheet(SAAS_SHEETS.periods);
    const {period} = report;
    addTitleRows(periods, meta, ["Metric", `Current (${period.currStart} to ${period.currEnd})`, `Previous (${period.prevStart} to ${period.prevEnd})`]);
    periods.getColumn(2).width = 28;
    periods.getColumn(3).width = 28;
    const hasBilling = report.current.billingCustomers !== null || report.previous.billingCustomers !== null;
    [...PERIOD_ROWS, ...(hasBilling ? BILLING_PERIOD_ROWS : [])].forEach(([label, read], i) => {
        labelCell(periods, 5 + i, label, 0);
        setSaasValue(periods.getCell(5 + i, 2), read(report.current), base);
        setSaasValue(periods.getCell(5 + i, 3), read(report.previous), base);
    });

    if (report.segments.length > 0) {
        const segments = workbook.addWorksheet(SAAS_SHEETS.segments);
        addTitleRows(segments, meta, ["Value", "MRR (current)", "MRR (previous)", "Customers", "ARPA", "NRR"]);
        let rowIx = 5;
        for (const breakdown of report.segments) {
            labelCell(segments, rowIx, breakdown.key, 0, true);
            rowIx += 1;
            for (const row of breakdown.rows) {
                labelCell(segments, rowIx, row.value, 1);
                setDec(segments.getCell(rowIx, 2), base, row.mrrCurr);
                setDec(segments.getCell(rowIx, 3), base, row.mrrPrev);
                setNumber(segments.getCell(rowIx, 4), row.customersCurr, 0);
                if (row.arpa !== null) setDec(segments.getCell(rowIx, 5), base, row.arpa);
                setPctUnits(segments.getCell(rowIx, 6), row.nrrPct);
                rowIx += 1;
            }
        }
    }

    if (report.customerMatrix !== null && report.customerMatrix.length > 0) {
        const customers = workbook.addWorksheet(SAAS_SHEETS.customers);
        addTitleRows(customers, meta, ["Customer", "Segment", ...monthHeaders]);
        customers.views = [{state: "frozen", ySplit: 4, xSplit: 2}];
        report.customerMatrix.forEach((row, i) => {
            labelCell(customers, 5 + i, row.name, 0);
            if (row.segment !== null) customers.getCell(5 + i, 2).value = row.segment;
            row.values.forEach((value, at) => setDec(customers.getCell(5 + i, at + 3), base, value));
        });
    }

    const definitions = workbook.addWorksheet(SAAS_SHEETS.definitions);
    addTitleRows(definitions, meta, ["Metric", "Definition", "Fed by", "Customer coverage", "Span"]);
    definitions.getColumn(2).width = 70;
    definitions.getColumn(3).width = 36;
    const span = `${period.start} to ${period.end}`;
    DEFINITIONS.forEach(([label, formula, roles, tier], i) => {
        const rowIx = 5 + i;
        labelCell(definitions, rowIx, label, 0);
        definitions.getCell(rowIx, 2).value = formula;
        definitions.getCell(rowIx, 3).value = roles;
        if (tier === "customers") setPctUnits(definitions.getCell(rowIx, 4), report.available.coveragePct);
        definitions.getCell(rowIx, 5).value = span;
    });

    return workbook;
}

/** Build the startup-metrics .xlsx and trigger a browser download. */
export async function exportSaasXlsx(report: SaasReport, meta: {title: string; params: string}, filename: string): Promise<void> {
    await downloadWorkbook(await buildSaasWorkbook(report, meta), filename);
}

/**
 * Serialize the workbook and trigger a browser download (Blob + anchor). In
 * the iOS app (plan 42) a web view cannot complete an `<a download>`, so the
 * bytes go to the native share sheet instead.
 */
export async function downloadWorkbook(workbook: Workbook, filename: string): Promise<void> {
    const buffer = await workbook.xlsx.writeBuffer();
    if (isNative()) {
        share(filename, XLSX_MIME, buffer as ArrayBuffer);
        return;
    }
    const blob = new Blob([buffer as ArrayBuffer], {type: XLSX_MIME});
    const url = URL.createObjectURL(blob);
    try {
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = filename;
        document.body.appendChild(anchor);
        anchor.click();
        anchor.remove();
    } finally {
        URL.revokeObjectURL(url);
    }
}

/** Build the .xlsx and trigger a browser download (Blob + anchor). */
export async function exportXlsx(report: SectionedReport | PeriodReport, meta: {title: string; params: string}, filename: string): Promise<void> {
    await downloadWorkbook(await buildWorkbook(report, meta), filename);
}

/** Build the grouped balance-sheet .xlsx and trigger a browser download. */
export async function exportBalanceSheetXlsx(
    report: BalanceSheetReport | BalanceSheetColumns,
    meta: {title: string; params: string},
    filename: string
): Promise<void> {
    await downloadWorkbook(await buildBalanceSheetWorkbook(report, meta), filename);
}

/** Build the grouped income-statement .xlsx and trigger a browser download. */
export async function exportIncomeStatementXlsx(report: IncomeStatementReport, meta: {title: string; params: string}, filename: string): Promise<void> {
    await downloadWorkbook(await buildIncomeStatementWorkbook(report, meta), filename);
}

/** Build the statement-of-cash-flows .xlsx and trigger a browser download. */
export async function exportCashFlowXlsx(
    report: CashFlowStatement,
    meta: {title: string; params: string},
    filename: string,
    base: string | null
): Promise<void> {
    await downloadWorkbook(await buildCashFlowWorkbook(report, meta, base), filename);
}

/** Build the holdings .xlsx and trigger a browser download. */
export async function exportHoldingsXlsx(report: HoldingsReport, meta: {title: string; params: string}, filename: string): Promise<void> {
    await downloadWorkbook(await buildHoldingsWorkbook(report, meta), filename);
}

/** Build the budget .xlsx and trigger a browser download. */
export async function exportBudgetXlsx(report: BudgetReport, meta: {title: string; params: string}, filename: string): Promise<void> {
    await downloadWorkbook(await buildBudgetWorkbook(report, meta), filename);
}
