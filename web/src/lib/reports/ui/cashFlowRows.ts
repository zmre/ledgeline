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
//
// OUTLINE (plan 50). The engine lays each line's accounts out as an outline and
// labels every row; this file only folds, indents by `level` and keeps
// headings' amounts off the page. Every label carries its full account path for
// hover (`path`).

import type {
    CashFlowStatement,
    CfCashLine,
    CfColumns,
    CfLine,
    CfSectionKind,
    IsBucket,
    OutlineFields,
    OutlineKind,
    PeriodReport,
    PeriodRow,
} from "#lib/reports/types.js";
import {CF_EXCHANGE_PIN, CF_PIN} from "./cashFlowColors";

/** `line`: a statement line; the outline kinds: the rows inside an open line. */
export type CfRowKind = "netIncome" | "line" | OutlineKind;

export interface CfDisplayRow {
    kind: CfRowKind;
    /** Stable across refetches: the `{#each}` key, the fold state's key and the cursor's anchor. */
    key: string;
    label: string;
    /** 0 for a line; 1 + the outline level for a row inside an open line. */
    indent: number;
    /** The account Enter opens in the journal: an account or `(other)` row, or a single-account line; null otherwise. */
    account: string | null;
    /** The full account path shown on hover, or null when the row has none (net income, a group line). */
    path: string | null;
    /** Null for a heading: it groups rows, the line's total carries the money (plan 50 D4). */
    amounts: CfColumns | null;
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

/** Whether a row sits inside an open line (an outline row), rather than being a line itself. */
export function isOutlineRow(row: CfDisplayRow): boolean {
    return row.kind !== "line" && row.kind !== "netIncome";
}

/** What `lineRows` needs of a section line or a cash line. */
interface FoldLine<R> {
    name: string;
    source: CfLine["source"];
    label: string;
    account: string | null;
    rows: readonly R[];
}

/**
 * Whether a line can open.
 *
 * An account line (rolled up to the report's depth) that turns out to stand
 * for exactly ONE account has nothing to open: `assets:fidelity:ira` folding
 * over a lone `cash` was a disclosure that hid one row and then showed it. It
 * keeps its own label and shows that account's path on hover. A `cfgroup:`
 * line keeps its arrow even with one account, because there the name is the
 * user's own and the account behind it is information the line does not show.
 */
function canOpen(line: FoldLine<{account: string}>): boolean {
    if (line.source === "account" && line.rows.length === 1) return false;
    return line.rows.length > 1 || line.rows[0]?.account !== line.name;
}

/**
 * A line's head row, then (when open) its outline rows, all keyed under
 * `key` (the cash blocks prefix their edge, so the two lists' keys differ
 * while `cfFoldKey` still finds one fold for both).
 */
function lineRows<R extends OutlineFields & {account: string}>(
    line: FoldLine<R>,
    key: string,
    figures: {line: CfColumns; row: (row: R) => CfColumns},
    open: (openable: boolean) => boolean
): CfDisplayRow[] {
    const openable = canOpen(line);
    const expanded = open(openable);
    const lone = openable ? null : (line.rows[0]?.account ?? null);
    const head: CfDisplayRow = {
        kind: "line",
        key,
        label: line.label,
        indent: 0,
        account: lone,
        path: lone ?? line.account,
        amounts: figures.line,
        expanded,
        expandable: openable,
    };
    if (!expanded) return [head];
    return [
        head,
        ...line.rows.map((row): CfDisplayRow => ({
            kind: row.kind,
            // A heading and its `(other)` row share an account; the kind tells them apart.
            key: `${key}/${row.account}${row.kind === "account" ? "" : `#${row.kind}`}`,
            label: row.label,
            indent: 1 + row.level,
            account: row.kind === "heading" ? null : row.account,
            path: row.account,
            amounts: row.kind === "heading" ? null : figures.row(row),
            expanded: false,
            expandable: false,
        })),
    ];
}

function sectionLineRows(prefix: string, line: CfLine, isOpen: IsOpen): CfDisplayRow[] {
    const key = `${prefix}/${line.source}/${line.name}`;
    return lineRows(line, key, {line: line.total, row: (row) => row.amounts}, (openable) => openable && isOpen(key, false));
}

function cashRows(edge: "beginning" | "ending", line: CfCashLine, isOpen: IsOpen): CfDisplayRow[] {
    const key = `cash/${line.source}/${line.name}`;
    // One fold for both blocks: opening "Mercury" under Beginning cash opens it
    // under Ending cash too, so the two lists always line up.
    return lineRows(line, `${edge}/${key}`, {line: line[edge], row: (row) => row[edge]}, (openable) => openable && isOpen(key, true));
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
        const lines = section.lines.flatMap((line) => sectionLineRows(section.kind, line, isOpen));
        const rows: CfDisplayRow[] =
            section.kind === "operating"
                ? [
                      {
                          kind: "netIncome",
                          key: "operating/net-income",
                          label: "Net income",
                          indent: 0,
                          account: null,
                          path: null,
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
 * Each row is PINNED to its heading's colour in the statement (`CF_PIN`), so
 * the chart does not hand colours out by size the way Net Worth does.
 */
export function cashFlowChartReport(report: CashFlowStatement): PeriodReport {
    const series: {account: string; figures: CfColumns; pin: NonNullable<PeriodRow["pin"]>}[] = report.sections.map((section) => ({
        account: section.title.replace(/ activities$/, ""),
        figures: section.total,
        pin: CF_PIN[section.kind],
    }));
    if (!isZero(report.exchange)) series.push({account: "Exchange rates", figures: report.exchange, pin: CF_EXCHANGE_PIN});
    return {
        buckets: report.buckets.map((bucket) => bucket.key),
        periods: report.buckets.map((bucket) => ({key: bucket.key, label: bucket.label, start: bucket.from, end: bucket.to})),
        rows: series.map((row): PeriodRow => ({account: row.account, depth: 1, values: row.figures.values, total: row.figures.total, pin: row.pin})),
        totals: report.netChange.values,
        grandTotal: report.netChange.total,
    };
}
