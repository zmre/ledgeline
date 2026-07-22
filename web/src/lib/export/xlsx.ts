// xlsx export (WP-07). exceljs is loaded via `await import("exceljs")` ONLY —
// it must never reach the initial bundle (vite splits it into a lazy chunk).
// Numbers cross the exact-money display boundary here: `toNumber()` at the
// export boundary is acceptable per plans/07; the Excel number format keeps
// the Dec's decimal places.

import {resolveAccountType, type AccountType} from "$lib/domain/accountTypes";
import {maAdd, maNeg, MAX_DISPLAY_DECIMALS, toNumber, type Dec, type MixedAmount} from "$lib/domain/money";
import type {HoldingsReport} from "$lib/holdings/types";
import {budgetLeaves, magnitudeAmount, primaryValue, summarizeBudget} from "$lib/reports/budgetSummary";
import {bucketLabel} from "$lib/reports/periods";
import type {BudgetReport, PeriodReport, SectionedReport} from "$lib/reports/types";
import {compressPeriodRows, compressSectionRows} from "$lib/reports/ui/displayRows";
import type {Workbook, Worksheet} from "exceljs"; // type-only: erased at build time

const HEADER_ARGB = "FF1E293B";
const XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/** Excel number format for a quantity: grouping + the Dec's decimal places (display-capped) + commodity affix. */
export function numberFormat(commodity: string, places: number): string {
    const shown = Math.min(places, MAX_DISPLAY_DECIMALS);
    const base = shown > 0 ? `#,##0.${"0".repeat(shown)}` : "#,##0";
    if (commodity === "") return base;
    const quoted = `"${commodity.replace(/"/g, '""')}"`;
    // Single-symbol commodities ($ € £ ¥) read best as prefixes; codes (USD, AAPL) as suffixes.
    return commodity.length === 1 ? `${quoted}${base}` : `${base} ${quoted}`;
}

type Cell = ReturnType<Worksheet["getCell"]>;

/** Write a MixedAmount: single-commodity → real number + numFmt; multi → text fallback; empty → 0. */
function setAmount(cell: Cell, ma: MixedAmount): void {
    const entries = [...ma.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    if (entries.length === 1) {
        const [commodity, qty] = entries[0];
        cell.value = toNumber(qty);
        cell.numFmt = numberFormat(commodity, qty.p);
    } else if (entries.length === 0) {
        cell.value = 0;
        cell.numFmt = "#,##0";
    } else {
        cell.value = entries
            .map(([commodity, qty]: [string, Dec]) => `${toNumber(qty).toFixed(Math.min(qty.p, MAX_DISPLAY_DECIMALS))} ${commodity}`)
            .join(", ");
    }
    cell.alignment = {...cell.alignment, horizontal: "right"};
}

/** A single Dec quantity as a real number + numFmt, right-aligned (setAmount's single-commodity case, sans MixedAmount). */
function setDec(cell: Cell, commodity: string, qty: Dec): void {
    cell.value = toNumber(qty);
    cell.numFmt = numberFormat(commodity, qty.p);
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

/**
 * Budget summary: Spent / Budget / Remaining / % of budget per leaf category,
 * then a bold total. Matches the on-screen view — only revenue & expense
 * accounts, amounts in magnitude (income budgets are negative on the wire).
 */
function addBudget(ws: Worksheet, report: BudgetReport, declared: ReadonlyMap<string, AccountType>): void {
    const shown = budgetLeaves(summarizeBudget(report)).filter((l) => {
        const t = resolveAccountType(l.account, declared);
        return t === "revenue" || t === "expense";
    });
    let rowIx = 5;
    let totActual: MixedAmount = new Map();
    let totGoal: MixedAmount = new Map();
    for (const line of shown) {
        const goal = magnitudeAmount(line.goal ?? new Map());
        const actual = magnitudeAmount(line.actual);
        labelCell(ws, rowIx, line.account, 0);
        setAmount(ws.getCell(rowIx, 2), actual);
        setAmount(ws.getCell(rowIx, 3), goal);
        setAmount(ws.getCell(rowIx, 4), maAdd(goal, maNeg(actual)));
        setPct(ws.getCell(rowIx, 5), primaryValue(actual), primaryValue(goal));
        totActual = maAdd(totActual, actual);
        totGoal = maAdd(totGoal, goal);
        rowIx += 1;
    }
    labelCell(ws, rowIx, "Total", 0, true);
    setAmount(ws.getCell(rowIx, 2), totActual);
    setAmount(ws.getCell(rowIx, 3), totGoal);
    setAmount(ws.getCell(rowIx, 4), maAdd(totGoal, maNeg(totActual)));
    for (const col of [2, 3, 4]) ws.getCell(rowIx, col).font = {bold: true};
    setPct(ws.getCell(rowIx, 5), primaryValue(totActual), primaryValue(totGoal), true);
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
        setDec(ws.getCell(rowIx, 3), "", h.shares);
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
