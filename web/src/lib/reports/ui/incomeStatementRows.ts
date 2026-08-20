// Display model for the grouped income statement (plans/13).
//
// Presentation, not engine: it lives under `reports/ui/` because collapsing a
// disclosure and turning two exact decimals into "73.9%" are decisions about a
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

import type {Dec, MixedAmount} from "$lib/domain/money";
import {EM_DASH, ZERO} from "$lib/format/amounts";
import type {Amounts, IncomeStatementReport, IsSection, IsSectionKind, IsSubtotalKind} from "$lib/reports/types";
import {compressIsRows} from "./displayRows";

export {amountCell, type AmountCell} from "./amountCell";

/** A group heading (always shown) or one of its accounts (only when expanded). */
export type IsRowKind = "group" | "account";

export interface IsDisplayRow {
    kind: IsRowKind;
    /**
     * Stable identity, unique across the whole report. Used for three things at
     * once: the `{#each}` key, the collapse set's membership, and the keyboard
     * cursor's anchor. One key rather than three keeps them in agreement when a
     * refetch replaces the report with an equal-but-not-identical one.
     */
    key: string;
    /** The group's name, or the compressed account label relative to its displayed parent. */
    label: string;
    /** 0 for a group heading; 1 + the compression indent for its accounts. */
    indent: number;
    /** The account this row stands for — `data-account` and the journal drill-down. Null for a group. */
    account: string | null;
    /** The figure(s) on the right: current, plus the prior period's when comparing. */
    amounts: Amounts;
    /** This line as a share of revenue, or null when there is no revenue to divide by. */
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
    pct: number | null;
}

/** One box, plus whatever ladder lines print below it. */
export interface IsBox {
    kind: IsSectionKind;
    title: string;
    /** Visible rows, in visual order. The cursor list is exactly the concatenation of these. */
    rows: IsDisplayRow[];
    total: Amounts;
    totalPct: number | null;
    /** Ladder lines below this box. Usually empty. */
    trailing: IsSubtotalLine[];
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
    /** Net income as a share of revenue, or null when there is no revenue to divide by. */
    netPct: number | null;
}

/** Everything the view renders, built once so the template and the cursor read the same arrays. */
export interface IsDisplayModel {
    boxes: IsBox[];
    summary: IsSummary;
    comparing: boolean;
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

/**
 * Integer division rounded HALF AWAY FROM ZERO — the same convention
 * `roundTo`/`formatDec` apply to every money figure in this app, applied here to
 * a ratio so the percentage column cannot round the other way from the amounts
 * beside it.
 */
function divRoundHalfAway(numerator: bigint, denominator: bigint): bigint {
    const negative = numerator < 0n !== denominator < 0n;
    const a = numerator < 0n ? -numerator : numerator;
    const b = denominator < 0n ? -denominator : denominator;
    // floor(a/b + 1/2), done in integers.
    const quotient = (2n * a + b) / (2n * b);
    return negative ? -quotient : quotient;
}

/**
 * `amount ÷ revenue`, as a percentage to ONE decimal, or null when there is no
 * revenue to divide by.
 *
 * Computed from the exact `Dec` values and never from a formatted string
 * ([[journal-refresh-fingerprint]]): the displayed figures are already rounded
 * to cents, and a percentage derived from them would disagree with one derived
 * from the numbers the engine actually sent — visibly so on a large denominator.
 * The arithmetic is BigInt throughout, so nothing here goes through a float
 * until the final tenths-of-a-percent integer, which is exactly representable.
 *
 * `Option`-shaped on purpose. A journal with no revenue in the window has no
 * percentage — not 0.0%, not ∞ — and `—` is the honest cell. The denominator is
 * the REVENUE section's total, not net income: "% of revenue" is the only
 * reading of a common-size income statement, and net income is one of the lines
 * being measured.
 *
 * `base === null` (a journal with no base commodity) has no single figure to
 * form a ratio from, so it yields null rather than picking a commodity.
 */
export function pctOfRevenue(amount: MixedAmount, revenue: MixedAmount, base: string | null): number | null {
    if (base === null) return null;
    const denominator = revenue.get(base);
    if (denominator === undefined || denominator.m === 0n) return null;
    const numerator: Dec = amount.get(base) ?? ZERO;
    // value(n)/value(d) × 1000 = tenths of a percent, exactly:
    //   (n.m / 10^n.p) / (d.m / 10^d.p) × 1000 = n.m × 10^d.p × 1000 / (d.m × 10^n.p)
    const num = numerator.m * 10n ** BigInt(denominator.p) * 1000n;
    const den = denominator.m * 10n ** BigInt(numerator.p);
    return Number(divRoundHalfAway(num, den)) / 10;
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
 * No forced `+`: "73.9% of revenue" is a share, not a change, and "+73.9%"
 * would read as growth.
 */
export function fmtPct(pct: number | null): string {
    if (pct === null) return EM_DASH;
    return `${pct < 0 ? "-" : ""}${Math.abs(pct).toFixed(1)}%`;
}

/** The percentage denominator: the Revenue section's current total, or empty when there is none. */
export function revenueTotal(report: IncomeStatementReport): MixedAmount {
    // By kind, not by index: sections are ladder-ordered and a journal with no
    // revenue omits the box entirely, so position says nothing.
    return report.sections.find((section) => section.kind === "revenue")?.total.current ?? new Map();
}

/**
 * One section's visible rows, in visual order: every group heading, each
 * followed by its accounts when it is expanded.
 *
 * Account rows go through `compressIsRows`, so a single-child chain
 * (`expenses:housing` → `expenses:housing:rent`) reads as one row here exactly
 * as it does in every other report table.
 */
export function sectionDisplayRows(section: IsSection, isExpanded: (key: string) => boolean, revenue: MixedAmount, base: string | null): IsDisplayRow[] {
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
            pct: pctOfRevenue(group.total.current, revenue, base),
            expanded,
            expandable,
        });
        if (!expanded) continue;
        for (const display of compressIsRows(group.rows)) {
            out.push({
                kind: "account",
                key: `${key}/${display.row.account}`,
                label: display.label,
                indent: display.indent + 1,
                account: display.row.account,
                amounts: display.row.amounts,
                pct: pctOfRevenue(display.row.amounts.current, revenue, base),
                expanded: false,
                expandable: false,
            });
        }
    }
    return out;
}

/**
 * The bottom line. Shared with the xlsx export so the workbook cannot disagree
 * with the screen about net income — or, just as easily, about which
 * denominator its margin was taken against.
 *
 * Small enough to look inlineable, and deliberately not: the choice of
 * denominator (the Revenue SECTION total, not net income) and the rounding are
 * the two things the two surfaces have to agree on, and they live here.
 */
export function isSummary(report: IncomeStatementReport): IsSummary {
    return {
        netIncome: report.netIncome,
        netPct: pctOfRevenue(report.netIncome.current, revenueTotal(report), report.base),
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
    const revenue = revenueTotal(report);
    const base = report.base;
    return {
        boxes: report.sections.map((section) => ({
            kind: section.kind,
            title: section.title,
            rows: sectionDisplayRows(section, isExpanded, revenue, base),
            total: section.total,
            totalPct: pctOfRevenue(section.total.current, revenue, base),
            trailing: section.trailing.map((subtotal) => ({
                kind: subtotal.kind,
                label: subtotal.label,
                amounts: subtotal.total,
                pct: pctOfRevenue(subtotal.total.current, revenue, base),
            })),
        })),
        summary: isSummary(report),
        comparing: report.prior !== null,
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
