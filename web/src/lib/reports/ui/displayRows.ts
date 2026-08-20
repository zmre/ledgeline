// Single-child chain compression (WP-07 display concern, per plans/07).
// Engine rows include every ancestor (sorted, parents before children); for
// display, a "boring" parent — exactly one child and nothing of its own —
// collapses into that child, so `assets > bank > checking` renders as one
// `assets:bank:checking` row (hledger tree-mode presentation).

import {maAdd, maIsZero, maNeg, type MixedAmount} from "$lib/domain/money";
import type {IsRow, PeriodReport, ReportRow} from "$lib/reports/types";

export interface DisplayRow<T> {
    /** Account segments relative to the displayed parent, e.g. "bank:checking". */
    label: string;
    /** 0-based visual indent level (after compression). */
    indent: number;
    /** The engine row carrying the amounts (the deepest row of a collapsed chain). */
    row: T;
}

/**
 * Collapse single-child chains. `rows` must be lexically sorted with ancestors
 * present (the engine guarantees both). `boring(parent, onlyChild)` decides
 * whether `parent` may be absorbed into its only child.
 */
export function compressRows<T>(rows: readonly T[], account: (row: T) => string, boring: (parent: T, onlyChild: T) => boolean): DisplayRow<T>[] {
    const byAccount = new Map<string, T>();
    for (const row of rows) byAccount.set(account(row), row);

    const children = new Map<string, string[]>();
    const roots: string[] = [];
    for (const row of rows) {
        const acct = account(row);
        const sep = acct.lastIndexOf(":");
        const parent = sep === -1 ? null : acct.slice(0, sep);
        if (parent !== null && byAccount.has(parent)) {
            const list = children.get(parent);
            if (list === undefined) children.set(parent, [acct]);
            else list.push(acct);
        } else {
            roots.push(acct);
        }
    }

    const out: DisplayRow<T>[] = [];
    const emit = (acct: string, indent: number, prefixLen: number): void => {
        let cur = acct;
        for (;;) {
            const kids = children.get(cur) ?? [];
            if (kids.length !== 1) break;
            const parentRow = byAccount.get(cur);
            const childRow = byAccount.get(kids[0]);
            if (parentRow === undefined || childRow === undefined || !boring(parentRow, childRow)) break;
            cur = kids[0];
        }
        const row = byAccount.get(cur);
        if (row !== undefined) out.push({label: cur.slice(prefixLen), indent, row});
        for (const kid of children.get(cur) ?? []) emit(kid, indent + 1, cur.length + 1);
    };
    for (const root of roots) emit(root, 0, 0);
    return out;
}

/** Sectioned reports (bs/is): a parent is boring when it has no direct postings of its own. */
export function compressSectionRows(rows: readonly ReportRow[]): DisplayRow<ReportRow>[] {
    return compressRows(
        rows,
        (r) => r.account,
        (parent) => maIsZero(parent.own)
    );
}

/**
 * Grouped income statement (plans/13). Its rows carry ONE rolled-up figure —
 * `amounts`, not the `own`/`inclusive` pair — because that is what the wire
 * sends, so "has no postings of its own" is not directly askable. A parent is
 * boring when its figure equals its only child's, which is the same test
 * `compressPeriodRows` makes and for the same reason.
 *
 * BOTH periods have to match. A parent whose current figure equals its child's
 * but whose PRIOR does not has postings of its own in the prior window, and
 * folding it would silently drop them from the comparison column.
 */
export function compressIsRows(rows: readonly IsRow[]): DisplayRow<IsRow>[] {
    const same = (a: MixedAmount | undefined, b: MixedAmount | undefined): boolean => maIsZero(maAdd(a ?? new Map(), maNeg(b ?? new Map())));
    return compressRows(
        rows,
        (r) => r.account,
        (parent, child) => same(parent.amounts.current, child.amounts.current) && same(parent.amounts.prior, child.amounts.prior)
    );
}

type PeriodRow = PeriodReport["rows"][number];

/**
 * Period reports (cf): rows carry only rolled-up values, so a parent is boring
 * when its value equals its only child's in every bucket (⇔ zero own total).
 */
export function compressPeriodRows(rows: readonly PeriodRow[]): DisplayRow<PeriodRow>[] {
    return compressRows(
        rows,
        (r) => r.account,
        (parent, child) => parent.values.every((v, i) => maIsZero(maAdd(v, maNeg(child.values[i]))))
    );
}
