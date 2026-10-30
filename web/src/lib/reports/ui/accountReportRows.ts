// The Accounts report's display model (2026-10-28): what each section is
// called, what its last column is headed, and its rows with single-child
// chains folded — one function, so the screen and the workbook agree.

import {accountMatches} from "#lib/domain/accounts.js";
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

/** Every account the report has a row for, sorted: the account filter's tree. */
export function reportAccounts(report: AccountReport): string[] {
    return [...new Set(report.sections.flatMap((section) => section.rows.map((row) => row.account)))].sort();
}

const sumColumns = (rows: readonly AccountReportRow[], width: number): MixedAmount[] =>
    Array.from({length: width}, (_, i) => rows.reduce<MixedAmount>((total, row) => maAdd(total, row.values[i] ?? new Map()), new Map()));

/**
 * The report narrowed to the selected subtrees (subtree roots, as
 * `AccountTreeSelect` keeps them). An empty selection is the whole report.
 *
 * Only selected accounts and their descendants stay — never their ancestors,
 * whose rolled-up figures would carry the siblings the user left out. Each
 * section's totals are re-summed over its topmost remaining rows (rows are
 * rolled up, so summing deeper ones would count them twice), and a section
 * with nothing left is dropped. The screen and the workbook both read this,
 * so a download holds what the table shows.
 */
export function filterAccountReport(report: AccountReport, selected: ReadonlySet<string>): AccountReport {
    if (selected.size === 0) return report;
    const covered = (account: string): boolean => [...selected].some((root) => accountMatches(root, account));
    const sections = report.sections.flatMap((section) => {
        const rows = section.rows.filter((row) => covered(row.account));
        if (rows.length === 0) return [];
        const present = new Set(rows.map((row) => row.account));
        const isTop = (account: string): boolean =>
            account
                .split(":")
                .slice(0, -1)
                .every((_, i, segments) => !present.has(segments.slice(0, i + 1).join(":")));
        const tops = rows.filter((row) => isTop(row.account));
        return [
            {
                ...section,
                rows,
                totals: sumColumns(tops, section.totals.length),
                summary: tops.reduce<MixedAmount>((total, row) => maAdd(total, row.summary), new Map()),
            },
        ];
    });
    return {...report, sections};
}

/** One displayed row of the Accounts table, with its disclosure state. */
export interface AccountTreeRow {
    display: DisplayRow<AccountReportRow>;
    /** Whether rows sit beneath it, so it gets a disclosure. */
    parent: boolean;
    /** Whether those rows are showing. */
    expanded: boolean;
}

/**
 * The rows of one section that are on screen: every row whose displayed
 * ancestors are all expanded. `collapsed` holds account names (a folded
 * chain's deepest account, the one its row carries), so a refetch — which
 * replaces every row object — keeps what the user closed.
 */
export function visibleAccountRows(rows: readonly DisplayRow<AccountReportRow>[], collapsed: ReadonlySet<string>): AccountTreeRow[] {
    const out: AccountTreeRow[] = [];
    let hiddenBelow: number | null = null;
    rows.forEach((display, i) => {
        if (hiddenBelow !== null && display.indent > hiddenBelow) return;
        hiddenBelow = null;
        const parent = (rows[i + 1]?.indent ?? -1) > display.indent;
        const expanded = !(parent && collapsed.has(display.row.account));
        if (!expanded) hiddenBelow = display.indent;
        out.push({display, parent, expanded});
    });
    return out;
}

/** Every row with rows beneath it, across sections: what "Collapse all" closes. */
export function parentAccounts(sections: readonly AccountDisplaySection[]): Set<string> {
    return new Set(
        sections.flatMap((section) =>
            section.rows.filter((display, i) => (section.rows[i + 1]?.indent ?? -1) > display.indent).map((display) => display.row.account)
        )
    );
}
