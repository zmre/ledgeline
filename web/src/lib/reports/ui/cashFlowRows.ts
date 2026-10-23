// Display model for the statement of cash flows (plans/27).
//
// Presentation, not engine, for the reason `incomeStatementRows.ts` gives: it
// decides what is folded and what reads as a heading, which is about a screen.
// ONE function builds the rows the template iterates AND the flat list the
// keyboard cursor walks, so a row cannot be on screen and unreachable by `j`,
// or the reverse.
//
// SIGNS. Every figure arrives as its contribution to cash: an asset that grew
// reads negative (cash went into it), a liability that grew reads positive.
// Nothing here negates anything.
//
// FOLDING. A section line with more than one account behind it, or one rolled
// up from deeper accounts, can open to show them; it starts closed. A cash line
// starts OPEN: the cash balances by account are what this tab used to show, and
// the detail is kept on screen (plans/27, "Answers").

import type {CashFlowStatement, CfCashLine, CfColumns, CfLine, CfSectionKind, IsBucket, PeriodReport} from "#lib/reports/types.js";

export type CfRowKind = "netIncome" | "line" | "account";

export interface CfDisplayRow {
    kind: CfRowKind;
    /** Stable across refetches: the `{#each}` key, the fold state's key and the cursor's anchor. */
    key: string;
    label: string;
    /** 0 for a line, 1 for an account inside an open line. */
    indent: number;
    /** The real account behind an account row, or a single-account line; null otherwise. */
    account: string | null;
    amounts: CfColumns;
    expanded: boolean;
    expandable: boolean;
}

export interface CfBox {
    kind: CfSectionKind;
    title: string;
    rows: CfDisplayRow[];
    /** "Net cash from operating activities" and so on. */
    totalLabel: string;
    total: CfColumns;
}

export interface CfCashBlock {
    kind: "beginning" | "ending";
    title: string;
    rows: CfDisplayRow[];
    total: CfColumns;
}

export interface CfDisplayModel {
    boxes: CfBox[];
    /** The effect of exchange rates, or null when it is zero in every column. */
    exchange: CfColumns | null;
    netChange: CfColumns;
    beginning: CfCashBlock;
    ending: CfCashBlock;
    /** Interval columns, or null for a single Total column (one bucket). */
    buckets: IsBucket[] | null;
}

/** Whether a fold is open, given its key and whether it starts open. */
export type IsOpen = (key: string, startsOpen: boolean) => boolean;

const isZero = (columns: CfColumns): boolean => columns.values.every((value) => [...value.values()].every((dec) => dec.m === 0n));

/** `account` relative to the line it sits in: the remainder under an account line, else the whole name. */
function rowLabel(account: string, line: string): string {
    return account.startsWith(`${line}:`) ? account.slice(line.length + 1) : account;
}

/**
 * Whether a line can open, and what it is called.
 *
 * An account line (rolled up to the report's depth) that turns out to stand
 * for exactly ONE account is printed as that account, in full, with nothing to
 * open: `assets:fidelity:ira` folding over a lone `cash` was a disclosure that
 * hid one row and then showed it. A `cfgroup:` line keeps its name and its
 * arrow even with one account, because there the name is the user's own and
 * the account behind it is information the line does not show.
 */
function folding(line: {name: string; source: CfLine["source"]; rows: readonly {account: string}[]}): {label: string; canOpen: boolean} {
    if (line.source === "account" && line.rows.length === 1) return {label: line.rows[0].account, canOpen: false};
    return {label: line.name, canOpen: line.rows.length > 1 || line.rows[0]?.account !== line.name};
}

function lineRows(prefix: string, line: CfLine, isOpen: IsOpen): CfDisplayRow[] {
    const key = `${prefix}/${line.source}/${line.name}`;
    const {label, canOpen} = folding(line);
    const open = canOpen && isOpen(key, false);
    const head: CfDisplayRow = {
        kind: "line",
        key,
        label,
        indent: 0,
        account: canOpen ? null : (line.rows[0]?.account ?? null),
        amounts: line.total,
        expanded: open,
        expandable: canOpen,
    };
    if (!open) return [head];
    return [
        head,
        ...line.rows.map((row): CfDisplayRow => ({
            kind: "account",
            key: `${key}/${row.account}`,
            label: rowLabel(row.account, line.name),
            indent: 1,
            account: row.account,
            amounts: row.amounts,
            expanded: false,
            expandable: false,
        })),
    ];
}

function cashRows(edge: "beginning" | "ending", line: CfCashLine, isOpen: IsOpen): CfDisplayRow[] {
    const key = `cash/${line.source}/${line.name}`;
    const {label, canOpen} = folding(line);
    // One fold for both blocks: opening "Mercury" under Beginning cash opens it
    // under Ending cash too, so the two lists always line up.
    const open = canOpen && isOpen(key, true);
    const head: CfDisplayRow = {
        kind: "line",
        key: `${edge}/${key}`,
        label,
        indent: 0,
        account: canOpen ? null : (line.rows[0]?.account ?? null),
        amounts: line[edge],
        expanded: open,
        expandable: canOpen,
    };
    if (!open) return [head];
    return [
        head,
        ...line.rows.map((row): CfDisplayRow => ({
            kind: "account",
            key: `${edge}/${key}/${row.account}`,
            label: rowLabel(row.account, line.name),
            indent: 1,
            account: row.account,
            amounts: row[edge],
            expanded: false,
            expandable: false,
        })),
    ];
}

/** The fold key a row toggles: cash rows share one key across the Beginning and Ending blocks. */
export function cfFoldKey(row: CfDisplayRow): string {
    return row.key.replace(/^(beginning|ending)\//, "");
}

const TOTAL_LABELS: Record<CfSectionKind, string> = {
    operating: "Net cash from operating activities",
    investing: "Net cash from investing activities",
    financing: "Net cash from financing activities",
};

export function cfDisplayModel(report: CashFlowStatement, isOpen: IsOpen): CfDisplayModel {
    const boxes = report.sections.map((section): CfBox => {
        const lines = section.lines.flatMap((line) => lineRows(section.kind, line, isOpen));
        const rows: CfDisplayRow[] =
            section.kind === "operating"
                ? [
                      {
                          kind: "netIncome",
                          key: "operating/net-income",
                          label: "Net income",
                          indent: 0,
                          account: null,
                          amounts: report.netIncome,
                          expanded: false,
                          expandable: false,
                      },
                      ...lines,
                  ]
                : lines;
        return {kind: section.kind, title: section.title, rows, totalLabel: TOTAL_LABELS[section.kind], total: section.total};
    });
    return {
        boxes,
        exchange: isZero(report.exchange) ? null : report.exchange,
        netChange: report.netChange,
        beginning: {
            kind: "beginning",
            title: "Beginning cash",
            rows: report.cash.flatMap((line) => cashRows("beginning", line, isOpen)),
            total: report.beginningCash,
        },
        ending: {
            kind: "ending",
            title: "Ending cash",
            rows: report.cash.flatMap((line) => cashRows("ending", line, isOpen)),
            total: report.endingCash,
        },
        buckets: report.buckets.length > 1 ? report.buckets : null,
    };
}

/** Every row the cursor can land on, in screen order. */
export function cfCursorRows(model: CfDisplayModel): CfDisplayRow[] {
    return [...model.boxes.flatMap((box) => box.rows), ...model.beginning.rows, ...model.ending.rows];
}

/**
 * The chart's input: one stacked series per activity (and the effect of
 * exchange rates when there is one), with the net change in cash as the line.
 *
 * Shaped as a `PeriodReport` so the stacked-bar chart the other period tabs
 * use draws it unchanged, with its tooltips, legend and one-currency note.
 */
export function cashFlowChartReport(report: CashFlowStatement): PeriodReport {
    const series = report.sections.map((section) => ({account: section.title.replace(/ activities$/, ""), figures: section.total}));
    if (!isZero(report.exchange)) series.push({account: "Exchange rates", figures: report.exchange});
    return {
        buckets: report.buckets.map((bucket) => bucket.key),
        periods: report.buckets.map((bucket) => ({key: bucket.key, label: bucket.label, start: bucket.from, end: bucket.to})),
        rows: series.map((row) => ({account: row.account, depth: 1, values: row.figures.values, total: row.figures.total})),
        totals: report.netChange.values,
        grandTotal: report.netChange.total,
    };
}
