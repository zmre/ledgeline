// Display model for the grouped income statement (plans/13).
//
// Presentation, not engine: it lives under `reports/ui/` because collapsing a
// disclosure and turning two exact decimals into "12.3%" are decisions about a
// screen, and the purity rule (`reports/purity.test.ts`) keeps that out of the
// sources that port to Rust.
//
// The whole point of this module is that ONE function (`isDisplayModel`)
// produces the boxes the template iterates AND the flat list the keyboard cursor
// indexes into, so the two cannot drift apart — the same discipline
// `balanceSheetRows.ts` and `ReportTable.svelte` document for their own rows. A
// row that is not in this model is not on screen and is not reachable with `j`.
//
// SIGNS. Figures arrive display-signed: the engine has already flipped the
// sections that are negative internally, so revenue and every cost section read
// positive and nothing here negates anything. `other` is the exception by
// design — a grant and a lawsuit settlement can share it — so it is the one box
// allowed to print a negative total.

import {EM_DASH} from "#lib/format/amounts.js";
import type {MixedAmount} from "#lib/domain/money.js";
import type {Amounts, IncomeStatementReport, IsBucket, IsSection, IsSectionKind, IsSubtotalKind, UnplacedBudget} from "#lib/reports/types.js";
import {pctChange} from "./percent";

export {amountCell, type AmountCell} from "./amountCell";

/**
 * A group heading (always shown), or one of its outline rows (only when the
 * group is expanded): an outline `heading` (label only, no figure), an
 * `account` (an account, a depth-clipped ancestor, or a heading's `(other)`),
 * or a `total` ("Total <heading>", a budgeted heading's ruled subtotal, plan 50
 * D4 exception).
 */
export type IsRowKind = "group" | "heading" | "account" | "total";

export interface IsDisplayRow {
    kind: IsRowKind;
    /**
     * Stable identity, unique across the whole report. Used for three things at
     * once: the `{#each}` key, the collapse set's membership, and the keyboard
     * cursor's anchor. One key rather than three keeps them in agreement when a
     * refetch replaces the report with an equal-but-not-identical one.
     */
    key: string;
    /** The group's name, or the engine's outline label (own `name:` tag, else a humanized segment). */
    label: string;
    /** 0 for a group heading; 1 + the outline `level` for the rows inside it. */
    indent: number;
    /**
     * The full account path this row stands for — the label's hover text,
     * `data-account` and the journal drill-down. Null for a group.
     */
    account: string | null;
    /**
     * The figure(s) on the right: current, plus the prior period's when
     * comparing, or one per bucket in interval mode. Null on an outline
     * heading, which prints no amount (plan 50 D4): the group's Total says it.
     */
    amounts: Amounts | null;
    /** The Budget column's figure (plan 50 D13), or null: no goal here, a heading, or no Budget column. */
    budget: MixedAmount | null;
    /** This line's % change against the prior period, or null when there is none (see `pctChange`). */
    pct: number | null;
    /** Group rows: whether the disclosure is currently open. Always false for an account row. */
    expanded: boolean;
    /** Whether the disclosure can open at all — false for a group the engine sent no rows for. */
    expandable: boolean;
}

/** A rung of the ladder, printed RULED and BETWEEN boxes rather than inside one. */
export interface IsSubtotalLine {
    kind: IsSubtotalKind;
    label: string;
    amounts: Amounts;
    budget: MixedAmount | null;
    pct: number | null;
}

/** One box, plus whatever ladder lines print below it. */
export interface IsBox {
    kind: IsSectionKind;
    title: string;
    /** Visible rows, in visual order. The cursor list is exactly the concatenation of these. */
    rows: IsDisplayRow[];
    total: Amounts;
    totalBudget: MixedAmount | null;
    totalPct: number | null;
    /** Ladder lines below this box. Usually empty. */
    trailing: IsSubtotalLine[];
    /** The D14 footnotes under the box, one per goal shown only in its total (see `budgetFootnote`). */
    footnotes: string[];
}

/**
 * The bottom line, and nothing else.
 *
 * This DID restate each section's contribution — "Total Revenue / Less: Cost of
 * revenue / …" — on the argument that it was the income statement's answer to
 * the balance sheet's tie-out. Seeing a multi-step statement rendered settled
 * it: the two are not analogous. The balance sheet's tie-out earns its place
 * because it PROVES something the reader cannot otherwise check, `A = L + E`,
 * and carries the engine's verdict on it. Restating seven section totals proves
 * nothing — every one of them is already in a box footer directly above, and
 * every intermediate figure is already a rung of the ladder. It was seven
 * duplicated totals, which is precisely the complaint this redesign exists to
 * fix, reintroduced one panel lower down.
 *
 * So what remains is the one figure that is NOT already on screen anywhere.
 */
export interface IsSummary {
    netIncome: Amounts;
    netBudget: MixedAmount | null;
    /** Net income's % change against the prior period, or null when there is none. */
    netPct: number | null;
}

/** Everything the view renders, built once so the template and the cursor read the same arrays. */
export interface IsDisplayModel {
    boxes: IsBox[];
    summary: IsSummary;
    /** Total mode with a prior window: the Prior and % change columns exist. */
    comparing: boolean;
    /**
     * Interval mode's columns, or null in Total mode. When set, every figure is
     * printed as its `values` followed by a Total (`current`), and there is no
     * prior and no percentage.
     */
    buckets: IsBucket[] | null;
    /**
     * The Budget column is on (plan 50 D12): an italic Budget figure follows
     * the Total (Total mode: Amount) on every line. One whole-range goal, so it
     * sits after the Total in interval mode, never once per bucket.
     */
    budgeted: boolean;
}

/** The D14 footnote for a goal whose accounts span groups. Shared with the xlsx export. */
export function budgetFootnote(goal: UnplacedBudget): string {
    return `Budget for ${goal.label} spans groups: shown in section total only.`;
}

// There was a `sectionSign()` here, mapping each section kind to +1/−1. It went
// with the per-section summary that used it to write "Less: …" labels. Nothing
// on this side of the wire needs a section's sign any more: figures arrive
// display-signed, so the frontend never negates anything, and the one section
// that is genuinely a net contribution (`other`) simply prints the negative the
// engine sent.

/** The collapse-set key for one group. Exported so a caller can seed or assert the set. */
export function isGroupKey(sectionKind: string, groupName: string): string {
    return `${sectionKind}/${groupName}`;
}

// `pctChange` moved to `./percent` (plan 34) so the comparison columns share
// it; re-exported so every caller here is unchanged.
export {pctChange} from "./percent";

/** `pctChange` of one `Amounts` — the figure the % change column prints for it. */
export function amountsPct(amounts: Amounts, base: string | null): number | null {
    return pctChange(amounts.current, amounts.prior, base);
}

/**
 * A percentage for display: one decimal, or `—` when there is none.
 *
 * An ASCII hyphen, NOT the typographic U+2212 that `fmtSignedPct` uses. This
 * figure shares a ROW with money, and money is rendered by `formatDec`, which
 * writes `$-15,000.00` with a hyphen. `$-15,000.00 | −7.3%` in one line is two
 * different minus characters an inch apart. `fmtSignedPct`'s U+2212 is right
 * where it lives — a standalone change figure — and matching money is right
 * here. (Unifying the two is a change to the domain formatter, and therefore to
 * every money surface in the app; see the note in `format/amounts.ts`.)
 *
 * No forced `+`, matching the QuickBooks "% Change" column this replaced the
 * share-of-revenue one with: a decline carries its minus and growth needs no
 * mark.
 */
export function fmtPct(pct: number | null): string {
    if (pct === null) return EM_DASH;
    return `${pct < 0 ? "-" : ""}${Math.abs(pct).toFixed(1)}%`;
}

/**
 * One section's visible rows, in visual order: every group heading, each
 * followed by its outline rows when it is expanded.
 *
 * The outline is the engine's (plan 50): it decides headings, labels and
 * levels, so a lone child already reads as one row and nothing here
 * compresses or relabels anything. This only indents and keys.
 */
export function sectionDisplayRows(section: IsSection, isExpanded: (key: string) => boolean, base: string | null): IsDisplayRow[] {
    const out: IsDisplayRow[] = [];
    for (const group of section.groups) {
        const key = isGroupKey(section.kind, group.name);
        const expandable = group.rows.length > 0;
        const expanded = expandable && isExpanded(key);
        out.push({
            kind: "group",
            key,
            label: group.name,
            indent: 0,
            account: null,
            amounts: group.total,
            budget: group.budget,
            pct: amountsPct(group.total, base),
            expanded,
            expandable,
        });
        if (!expanded) continue;
        for (const row of group.rows) {
            const heading = row.kind === "heading";
            out.push({
                kind: heading ? "heading" : row.kind === "total" ? "total" : "account",
                // A heading with its own postings shares its account with its
                // `(other)` row, and a budgeted heading with its `total` row, so
                // those are told apart by a suffix.
                key: `${key}/${row.account}${row.kind === "other" || row.kind === "total" ? `#${row.kind}` : ""}`,
                label: row.label,
                indent: row.level + 1,
                account: row.account,
                amounts: heading ? null : row.amounts,
                // A heading prints nothing; its `total` row carries the goal.
                budget: heading ? null : row.budget,
                pct: heading ? null : amountsPct(row.amounts, base),
                expanded: false,
                expandable: false,
            });
        }
    }
    return out;
}

/**
 * The bottom line. Shared with the xlsx export so the workbook cannot disagree
 * with the screen about net income — or about its % change, which goes through
 * the same `pctChange` and the same rounding on both surfaces.
 */
export function isSummary(report: IncomeStatementReport): IsSummary {
    return {
        netIncome: report.netIncome,
        netBudget: report.netIncomeBudget,
        netPct: amountsPct(report.netIncome, report.base),
    };
}

/**
 * THE display model. One call builds every box, every ladder line and the
 * bottom line, from one `report` and one collapse predicate.
 *
 * The view holds this in a `$derived` and iterates `boxes[i].rows`; the cursor
 * indexes into `isCursorRows` of the SAME object, so a row can never be
 * reachable by `j` and absent from the screen (or the reverse).
 */
export function isDisplayModel(report: IncomeStatementReport, isExpanded: (key: string) => boolean): IsDisplayModel {
    const base = report.base;
    return {
        boxes: report.sections.map((section) => ({
            kind: section.kind,
            title: section.title,
            rows: sectionDisplayRows(section, isExpanded, base),
            total: section.total,
            totalBudget: section.budget,
            totalPct: amountsPct(section.total, base),
            trailing: section.trailing.map((subtotal) => ({
                kind: subtotal.kind,
                label: subtotal.label,
                amounts: subtotal.total,
                budget: subtotal.budget,
                pct: amountsPct(subtotal.total, base),
            })),
            footnotes: section.budgetUnplaced.map(budgetFootnote),
        })),
        summary: isSummary(report),
        comparing: report.prior !== null,
        buckets: report.buckets,
        budgeted: report.budgeted,
    };
}

/**
 * Every cursorable row, in visual order — the flattening of the very arrays the
 * template iterates.
 *
 * Ladder lines are deliberately NOT in it. A subtotal is not a thing you can
 * expand or drill into, so landing on one with `j` would be a stop that does
 * nothing on Enter.
 */
export function isCursorRows(model: IsDisplayModel): IsDisplayRow[] {
    return model.boxes.flatMap((box) => box.rows);
}
