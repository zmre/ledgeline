// The Accounts report's display model (2026-10-28): what each section is
// called, what its last column is headed, and its rows with single-child
// chains folded — one function, so the screen and the workbook agree.

import {maAdd, maIsZero, maNeg, type MixedAmount} from "#lib/domain/money.js";
import type {AccountReading, AccountReport, AccountReportRow, AccountSectionKind} from "#lib/reports/types.js";
import {compressRows, type DisplayRow} from "./displayRows";

export const ACCOUNT_SECTION_TITLES: Record<AccountSectionKind, string> = {
    assets: "Assets",
    liabilities: "Liabilities",
    equity: "Equity",
    revenue: "Revenue",
    expenses: "Expenses",
    other: "Other accounts",
};

/** The symbol a balance's last column carries, so a skimming reader never takes a change for a balance. */
export const DELTA = "Δ";

/** The heading of a section's last column. */
export function summaryHeading(reading: AccountReading): string {
    return reading === "balance" ? `${DELTA} over the period` : "Total";
}

/** One sentence per reading, for the help text above the table. */
export const READING_HELP: Record<AccountReading, string> = {
    balance: `Assets and liabilities show each account's balance at the end of each period. Their last column, ${DELTA}, is how much that balance changed from the start of the first period to the end of the last.`,
    change: "Equity, revenue and expenses show what moved in each period. Their last column is the Total of the periods.",
};

export interface AccountDisplaySection {
    kind: AccountSectionKind;
    title: string;
    reading: AccountReading;
    rows: DisplayRow<AccountReportRow>[];
    totals: MixedAmount[];
    summary: MixedAmount;
}

const same = (a: MixedAmount, b: MixedAmount): boolean => maIsZero(maAdd(a, maNeg(b)));

/**
 * The report's sections, each with its rows' single-child chains folded: a
 * parent equal to its only child in every column AND the last one has
 * nothing of its own, so `assets:mercury` → `assets:mercury:checking` reads as
 * one row, as on every other report table.
 */
export function accountDisplaySections(report: AccountReport): AccountDisplaySection[] {
    return report.sections.map((section) => ({
        kind: section.section,
        title: ACCOUNT_SECTION_TITLES[section.section],
        reading: section.reading,
        rows: compressRows(
            section.rows,
            (row) => row.account,
            (parent, child) => same(parent.summary, child.summary) && parent.values.every((value, i) => same(value, child.values[i] ?? new Map()))
        ),
        totals: section.totals,
        summary: section.summary,
    }));
}
