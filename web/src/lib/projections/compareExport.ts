// The rows of a comparison workbook (plans/34), read off the same display
// models the four statement views render, every group open — so a workbook
// lists the lines the screen can show, in its order, with its labels.

import type {CompareSheetRow} from "#lib/export/xlsx.js";
import type {AccountReport, BalanceSheetColumns, CashFlowStatement, IncomeStatementReport} from "#lib/reports/types.js";
import {accountDisplaySections} from "#lib/reports/ui/accountReportRows.js";
import {bsColumnSummaries, columnSectionRows} from "#lib/reports/ui/balanceSheetRows.js";
import {cfDisplayModel, type CfDisplayRow} from "#lib/reports/ui/cashFlowRows.js";
import type {WindowReading} from "#lib/reports/ui/compareColumns.js";
import {isDisplayModel} from "#lib/reports/ui/incomeStatementRows.js";

const heading = (label: string): CompareSheetRow => ({label, indent: 0, bold: true, values: null, reading: "flow"});

/** The balance sheet: every box's lines and total, then the tie-out (its last row's Δ blank, as on screen). */
export function balanceSheetCompareRows(sheet: BalanceSheetColumns): CompareSheetRow[] {
    const rows: CompareSheetRow[] = [];
    const line = (label: string, indent: number, bold: boolean, values: CompareSheetRow["values"], blankDelta = false): CompareSheetRow => ({
        label,
        indent,
        bold,
        values,
        reading: "balance",
        blankDelta,
    });
    for (const section of sheet.sections) {
        rows.push(heading(section.title));
        for (const row of columnSectionRows(section, () => true)) {
            if (row.kind === "subsection") rows.push(line(row.label, 1, true, null));
            else rows.push(line(row.label, row.kind === "account" ? row.indent + 1 : 1, row.kind !== "account", row.amount));
        }
        rows.push(line(`Total ${section.title}`, 0, true, section.total));
    }
    const summaries = bsColumnSummaries(sheet);
    rows.push(
        line(
            "Total Assets",
            0,
            false,
            summaries.map((summary) => summary.assets)
        )
    );
    rows.push(
        line(
            "Total Liabilities",
            0,
            false,
            summaries.map((summary) => summary.liabilities)
        )
    );
    rows.push(
        line(
            "Total Equity",
            0,
            false,
            summaries.map((summary) => summary.equity)
        )
    );
    rows.push(
        line(
            "Liabilities + Equity",
            0,
            true,
            summaries.map((summary) => summary.liabilitiesPlusEquity)
        )
    );
    return rows;
}

/** The income statement: every box, its total and the ladder lines under it, then net income. Flows: window sums. */
export function incomeStatementCompareRows(report: IncomeStatementReport): CompareSheetRow[] {
    const model = isDisplayModel(report, () => true);
    const rows: CompareSheetRow[] = [];
    const line = (label: string, indent: number, bold: boolean, values: CompareSheetRow["values"]): CompareSheetRow => ({
        label,
        indent,
        bold,
        values,
        reading: "flow",
    });
    for (const box of model.boxes) {
        rows.push(heading(box.title));
        for (const row of box.rows) rows.push(line(row.label, row.indent + 1, row.kind === "group", row.amounts.values ?? []));
        rows.push(line(`Total ${box.title}`, 0, true, box.total.values ?? []));
        for (const subtotal of box.trailing) rows.push(line(subtotal.label, 0, true, subtotal.amounts.values ?? []));
    }
    rows.push(line("Net income", 0, true, model.summary.netIncome.values ?? []));
    return rows;
}

/** The cash flow statement: activities (flows), net change, then beginning (first) and ending (last) cash. */
export function cashFlowCompareRows(report: CashFlowStatement): CompareSheetRow[] {
    const model = cfDisplayModel(report, () => true);
    const rows: CompareSheetRow[] = [];
    const cfRow = (row: CfDisplayRow, reading: WindowReading): CompareSheetRow => ({
        label: row.label,
        indent: row.indent + 1,
        bold: row.kind !== "account",
        values: row.amounts.values,
        reading,
    });
    for (const box of model.boxes) {
        rows.push(heading(box.title));
        for (const row of box.rows) rows.push(cfRow(row, "flow"));
        rows.push({label: box.totalLabel, indent: 0, bold: true, values: box.total.values, reading: "flow"});
    }
    if (model.exchange !== null) rows.push({label: "Effect of exchange rates on cash", indent: 0, bold: true, values: model.exchange.values, reading: "flow"});
    rows.push({label: "Net change in cash", indent: 0, bold: true, values: model.netChange.values, reading: "flow"});
    for (const block of [model.beginning, model.ending]) {
        const reading: WindowReading = block.kind === "beginning" ? "opening" : "balance";
        rows.push({label: block.title, indent: 0, bold: true, values: block.total.values, reading});
        for (const row of block.rows) rows.push(cfRow(row, reading));
    }
    return rows;
}

/** The Accounts reading: each section's rows and total, a balance section by its last period, the rest by the sum. */
export function accountsCompareRows(report: AccountReport): CompareSheetRow[] {
    const rows: CompareSheetRow[] = [];
    for (const section of accountDisplaySections(report)) {
        const reading: WindowReading = section.reading === "balance" ? "balance" : "flow";
        rows.push(heading(section.title));
        for (const display of section.rows) rows.push({label: display.label, indent: display.indent + 1, bold: false, values: display.row.values, reading});
        rows.push({label: `Total ${section.title.toLowerCase()}`, indent: 0, bold: true, values: section.totals, reading});
    }
    return rows;
}
