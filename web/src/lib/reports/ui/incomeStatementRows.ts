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

import type {Dec, MixedAmount} from "#lib/domain/money.js";
import {EM_DASH, ZERO} from "#lib/format/amounts.js";
import type {Amounts, IncomeStatementReport, IsBucket, IsSection, IsSectionKind, IsSubtotalKind} from "#lib/reports/types.js";
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
    /** The figure(s) on the right: current, plus the prior period's when comparing, or one per bucket in interval mode. */
    amounts: Amounts;
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
 * `(current − prior) ÷ |prior|`, as a percentage to ONE decimal — the % change
 * column — or null when there is no change to state.
 *
 * Computed from the exact `Dec` values and never from a formatted string
 * ([[journal-refresh-fingerprint]]): the displayed figures are already rounded
 * to cents, and a percentage derived from them would disagree with one derived
 * from the numbers the engine actually sent — visibly so on a large denominator.
 * The arithmetic is BigInt throughout, so nothing here goes through a float
 * until the final tenths-of-a-percent integer, which is exactly representable.
 *
 * The denominator is the prior figure's MAGNITUDE, so the sign always says
 * which way the line moved: an `other` box going from −$100 to −$50 improved,
 * and reads +50.0%, not −50.0%.
 *
 * `Option`-shaped on purpose. A line that was zero (or absent) last period has
 * no percentage change — not 0.0%, not ∞ — and `—` is the honest cell. So does
 * a report with no prior period at all (`prior` undefined: compare off, or
 * interval mode), and `base === null` (a journal with no base commodity), which
 * has no single figure to form a ratio from rather than one picked at random.
 */
export function pctChange(current: MixedAmount, prior: MixedAmount | undefined, base: string | null): number | null {
    if (base === null || prior === undefined) return null;
    const was: Dec | undefined = prior.get(base);
    if (was === undefined || was.m === 0n) return null;
    const now: Dec = current.get(base) ?? ZERO;
    // Both on one scale, then (now − was) / |was| × 1000 = tenths of a percent.
    const scale = Math.max(now.p, was.p);
    const nowM = now.m * 10n ** BigInt(scale - now.p);
    const wasM = was.m * 10n ** BigInt(scale - was.p);
    return Number(divRoundHalfAway((nowM - wasM) * 1000n, wasM < 0n ? -wasM : wasM)) / 10;
}

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
 * followed by its accounts when it is expanded.
 *
 * Account rows go through `compressIsRows`, so a single-child chain
 * (`expenses:housing` → `expenses:housing:rent`) reads as one row here exactly
 * as it does in every other report table.
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
            pct: amountsPct(group.total, base),
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
                pct: amountsPct(display.row.amounts, base),
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
            totalPct: amountsPct(section.total, base),
            trailing: section.trailing.map((subtotal) => ({
                kind: subtotal.kind,
                label: subtotal.label,
                amounts: subtotal.total,
                pct: amountsPct(subtotal.total, base),
            })),
        })),
        summary: isSummary(report),
        comparing: report.prior !== null,
        buckets: report.buckets,
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
