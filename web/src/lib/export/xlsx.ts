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

import {resolveAccountType, type AccountType} from "$lib/domain/accountTypes";
import {
    displayPlaces,
    formatDec,
    maAdd,
    maNeg,
    MAX_DISPLAY_DECIMALS,
    MAX_QUANTITY_DECIMALS,
    roundTo,
    toNumber,
    type Dec,
    type MixedAmount,
} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {HoldingsReport} from "$lib/holdings/types";
import {budgetLeaves, budgetTotals, magnitudeAmount, primaryValue, summarizeBudget, type BudgetLine} from "$lib/reports/budgetSummary";
import {bucketLabel} from "$lib/reports/periods";
import type {BudgetReport, PeriodReport, SectionedReport} from "$lib/reports/types";
import {compressPeriodRows, compressSectionRows} from "$lib/reports/ui/displayRows";
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

/** Write a MixedAmount: single-commodity → real number + numFmt; multi → text fallback; empty → 0. */
function setAmount(cell: Cell, ma: MixedAmount): void {
    const entries = [...ma.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    if (entries.length === 1) {
        writeDec(cell, entries[0][0], entries[0][1], MAX_DISPLAY_DECIMALS);
    } else if (entries.length === 0) {
        cell.value = 0;
        cell.numFmt = "#,##0";
    } else {
        // Formatted from the exact Dec, never `toNumber(...).toFixed(...)` — that
        // rounds the binary double and drifts a cent off the screen's string.
        cell.value = entries.map(([commodity, qty]: [string, Dec]) => `${formatDec(qty, {...TEXT_STYLE, precision: qty.p})} ${commodity}`).join(", ");
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

/** Balance sheet / income statement: same compressed rows the UI shows, one Amount column. */
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

/** Cash flow / net worth: one column per bucket plus a bold Net totals row. */
function addPeriod(ws: Worksheet, report: PeriodReport): void {
    let rowIx = 5;
    for (const {label, indent, row} of compressPeriodRows(report.rows)) {
        labelCell(ws, rowIx, label, indent);
        row.values.forEach((value, i) => setAmount(ws.getCell(rowIx, i + 2), value));
        rowIx += 1;
    }
    labelCell(ws, rowIx, "Net", 0, true);
    report.totals.forEach((total, i) => {
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

/** One Spent/Budget/Remaining/% row from a line's magnitudes (income budgets are negative on the wire). */
function addBudgetRow(ws: Worksheet, rowIx: number, label: string, actual: MixedAmount, goal: MixedAmount, indent: number, bold = false): void {
    labelCell(ws, rowIx, label, indent, bold);
    setAmount(ws.getCell(rowIx, 2), actual);
    setAmount(ws.getCell(rowIx, 3), goal);
    setAmount(ws.getCell(rowIx, 4), maAdd(goal, maNeg(actual)));
    setPct(ws.getCell(rowIx, 5), primaryValue(actual), primaryValue(goal), bold);
    if (bold) for (const col of [2, 3, 4]) ws.getCell(rowIx, col).font = {bold: true};
}

/**
 * Budget summary: Spent / Budget / Remaining / % of budget per leaf category,
 * split into the SAME two sections the page renders (BudgetSummary.svelte:
 * Income then Expenses), each with its own total.
 *
 * There is deliberately no single grand total. Income and expenses are opposite
 * signs on the wire and are shown here in magnitude, so one sum over both
 * columns is |income| + |expenses| — a figure that appears nowhere on screen and
 * means nothing (its "% of budget" even less). Two sections, two totals, exactly
 * as the page shows them.
 *
 * Section totals come from `budgetTotals` and take their magnitude AFTER
 * summing, matching the page's overall bar rather than re-deriving it.
 */
function addBudget(ws: Worksheet, report: BudgetReport, declared: ReadonlyMap<string, AccountType>): void {
    const leaves = budgetLeaves(summarizeBudget(report));
    const ofType = (type: AccountType): BudgetLine[] => leaves.filter((l) => resolveAccountType(l.account, declared) === type);
    const sections = [
        {title: "Income", lines: ofType("revenue")},
        {title: "Expenses", lines: ofType("expense")},
    ].filter((s) => s.lines.length > 0);

    let rowIx = 5;
    for (const section of sections) {
        labelCell(ws, rowIx, section.title, 0, true);
        rowIx += 1;
        for (const line of section.lines) {
            addBudgetRow(ws, rowIx, line.account, magnitudeAmount(line.actual), magnitudeAmount(line.goal ?? new Map()), 1);
            rowIx += 1;
        }
        const totals = budgetTotals(section.lines);
        addBudgetRow(ws, rowIx, `Total ${section.title}`, magnitudeAmount(totals.actual), magnitudeAmount(totals.goal), 0, true);
        rowIx += 1;
    }
}

/** Build the workbook (exported separately so tests can read it back without a DOM). */
export async function buildWorkbook(report: SectionedReport | PeriodReport, meta: {title: string; params: string}): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    const headers = "sections" in report ? ["Account", "Amount"] : ["Account", ...report.buckets.map(bucketLabel)];
    addTitleRows(ws, meta, headers);
    if ("sections" in report) addSectioned(ws, report);
    else addPeriod(ws, report);
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
export async function buildBudgetWorkbook(
    report: BudgetReport,
    meta: {title: string; params: string},
    declared: ReadonlyMap<string, AccountType>
): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet(meta.title);
    addTitleRows(ws, meta, ["Account", "Spent", "Budget", "Remaining", "% of budget"]);
    addBudget(ws, report, declared);
    return workbook;
}

/** Serialize the workbook and trigger a browser download (Blob + anchor). */
async function downloadWorkbook(workbook: Workbook, filename: string): Promise<void> {
    const buffer = await workbook.xlsx.writeBuffer();
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

/** Build the holdings .xlsx and trigger a browser download. */
export async function exportHoldingsXlsx(report: HoldingsReport, meta: {title: string; params: string}, filename: string): Promise<void> {
    await downloadWorkbook(await buildHoldingsWorkbook(report, meta), filename);
}

/** Build the budget .xlsx and trigger a browser download. */
export async function exportBudgetXlsx(
    report: BudgetReport,
    meta: {title: string; params: string},
    filename: string,
    declared: ReadonlyMap<string, AccountType>
): Promise<void> {
    await downloadWorkbook(await buildBudgetWorkbook(report, meta, declared), filename);
}
