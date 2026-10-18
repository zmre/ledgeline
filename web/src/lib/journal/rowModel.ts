// Pure row-model helpers for the journal view (WP-03): the source→destination
// arrow heuristic, display amounts, totals formatting, and the fixed-pitch
// windowing math for the virtualized table. No runtime Svelte imports, so
// vitest runs these in node.
//
// Filtering and display ORDER are the engine's now (plan 26 B5:
// crates/ledgeline-core/src/view/filter.rs): the client holds only the rows
// near the viewport.

import {add, formatAmount, isZero, neg, type Dec, type MixedAmount} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Posting, Transaction, TxnStatus} from "#lib/domain/types.js";

type Sign = -1 | 0 | 1;

/** Net sign of a posting across its commodities; null when commodities disagree in sign. */
function postingNetSign(posting: Posting): Sign | null {
    const net: MixedAmount = new Map();
    for (const amount of posting.amounts) {
        const prev = net.get(amount.commodity);
        net.set(amount.commodity, prev === undefined ? amount.qty : add(prev, amount.qty));
    }
    let sign: Sign = 0;
    for (const qty of net.values()) {
        if (isZero(qty)) continue;
        const s: Sign = qty.m < 0n ? -1 : 1;
        if (sign === 0) sign = s;
        else if (sign !== s) return null;
    }
    return sign;
}

export type AccountFlow = {kind: "flow"; source: string; dest: string} | {kind: "list"; accounts: string[]};

/**
 * From→to arrow heuristic: postings with net-negative MixedAmount are sources,
 * net-positive are destinations. Renders as `source → dest` only for the simple
 * case of exactly one source and one destination account; N-way splits (>2
 * distinct sides) and sign-mixed postings degrade to a plain account list.
 */
export function accountFlow(txn: Transaction): AccountFlow {
    const sources: string[] = [];
    const dests: string[] = [];
    let degrade = false;
    for (const posting of txn.postings) {
        const sign = postingNetSign(posting);
        if (sign === null) {
            degrade = true;
            break;
        }
        if (sign === -1 && !sources.includes(posting.account)) sources.push(posting.account);
        else if (sign === 1 && !dests.includes(posting.account)) dests.push(posting.account);
    }
    if (!degrade && sources.length === 1 && dests.length === 1 && sources[0] !== dests[0]) {
        return {kind: "flow", source: sources[0], dest: dests[0]};
    }
    const accounts: string[] = [];
    for (const posting of txn.postings) {
        if (!accounts.includes(posting.account)) accounts.push(posting.account);
    }
    return {kind: "list", accounts};
}

function sumWhere(txn: Transaction, pick: (qty: Dec) => boolean, negate: boolean): Amount[] {
    const sums = new Map<string, {qty: Dec; style: AmountStyle}>();
    for (const posting of txn.postings) {
        for (const amount of posting.amounts) {
            if (!pick(amount.qty)) continue;
            const qty = negate ? neg(amount.qty) : amount.qty;
            const prev = sums.get(amount.commodity);
            sums.set(amount.commodity, prev === undefined ? {qty, style: amount.style} : {qty: add(prev.qty, qty), style: prev.style});
        }
    }
    return [...sums.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([commodity, {qty, style}]) => ({commodity, qty, style}));
}

/**
 * The transaction's displayed magnitude: per-commodity sum of the positive
 * (destination-side) amounts, styled from the txn's own amounts. Falls back to
 * |negatives| when no entry is positive; empty for zero-amount transactions.
 */
export function txnFlowAmounts(txn: Transaction): Amount[] {
    const positive = sumWhere(txn, (qty) => qty.m > 0n, false);
    return positive.length > 0 ? positive : sumWhere(txn, (qty) => qty.m < 0n, true);
}

/** Comment texts for the indicator tooltip: txn comment first, then posting comments prefixed with their account. */
export function txnComments(txn: Transaction): string[] {
    const out: string[] = [];
    if (txn.comment !== "") out.push(txn.comment);
    for (const posting of txn.postings) {
        if (posting.comment !== "") out.push(`${posting.account}: ${posting.comment}`);
    }
    return out;
}

export interface TotalLine {
    text: string;
    negative: boolean;
}

/** Format a MixedAmount for the totals footer, one line per commodity, sorted by commodity. */
export function formatTotals(totals: MixedAmount, styles: ReadonlyMap<string, AmountStyle>): TotalLine[] {
    return [...totals.entries()]
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([commodity, qty]) => {
            const style = styles.get(commodity) ?? {side: "R" as const, spaced: true, precision: qty.p, decimalPoint: ".", digitGroups: null};
            return {text: formatAmount({commodity, qty, style}), negative: qty.m < 0n};
        });
}

/** Human label for the footer's period, e.g. "2026-07-01 – 2026-07-31" or "all dates". */
export function periodLabel(from: ISODate | null, to: ISODate | null): string {
    if (from === null && to === null) return "all dates";
    if (from === null) return `through ${to}`;
    if (to === null) return `from ${from}`;
    return `${from} – ${to}`;
}

export interface RowWindow {
    start: number; // first rendered row index (inclusive)
    end: number; // one past the last rendered row index
    padTop: number; // px spacer above the rendered slice
    padBottom: number; // px spacer below the rendered slice
}

/**
 * Fixed-pitch windowing over a scroll container. Rows are laid out on a
 * `rowPitch`-px grid; only the slice covering the viewport (± `overscan` rows)
 * is rendered, so the rendered-row count is bounded by
 * `ceil(viewportHeight / rowPitch) + 1 + 2 * overscan` — independent of `total`.
 * Invariant: `padTop + (end - start) * rowPitch + padBottom === total * rowPitch`.
 */
export function computeWindow(scrollTop: number, viewportHeight: number, rowPitch: number, total: number, overscan = 10): RowWindow {
    if (rowPitch <= 0) throw new RangeError(`computeWindow: rowPitch must be positive, got ${rowPitch}`);
    if (total <= 0) return {start: 0, end: 0, padTop: 0, padBottom: 0};
    const top = Math.max(0, scrollTop);
    const start = Math.min(Math.max(0, Math.floor(top / rowPitch) - overscan), total);
    const end = Math.min(total, Math.max(start, Math.ceil((top + Math.max(0, viewportHeight)) / rowPitch) + overscan));
    return {start, end, padTop: start * rowPitch, padBottom: (total - end) * rowPitch};
}

/**
 * Scroll offset that CENTRES row `position`.
 *
 * This is the problems-drawer behaviour: a jump from somewhere else wants
 * context around its target. Arithmetic rather than `scrollIntoView`, because
 * the target row is usually not mounted — that is the whole point of the
 * windowing above, and it is why there is no `scrollIntoView` call anywhere in
 * this component.
 */
export function centerOffset(position: number, pitch: number, viewportHeight: number, topInset = 0): number {
    return Math.max(0, topInset + position * pitch - Math.max(0, viewportHeight - pitch) / 2);
}

/**
 * Smallest offset that brings row `position` fully into view, or `scrollTop`
 * unchanged when it already is.
 *
 * This is the CURSOR behaviour, and it is deliberately different from
 * `centerOffset`: `j` and `k` must not throw the whole page around on every
 * keystroke.
 *
 * `headroom` is the sticky `<thead>`'s height. The header is sticky INSIDE the
 * scroll box, so it paints over the first `headroom` px of a viewport that
 * `clientHeight` counts in full — without it the row `j` just landed on sits
 * underneath the header. `rowHeight` is the row's own height, which in card mode
 * is smaller than `pitch` (a 96px card in a 104px slot).
 */
export function nearestOffset(
    scrollTop: number,
    position: number,
    pitch: number,
    rowHeight: number,
    viewportHeight: number,
    total: number,
    headroom = 0,
    topInset = 0
): number {
    // `headroom` counts TWICE, and missing the first use was a real bug.
    //
    // A sticky <thead> is pinned when you scroll, but it still occupies its
    // normal flow space — so row 0 begins at `headroom` px into the scrollable
    // content, not at 0. Ignoring that under-scrolled every downward move by
    // exactly the header's height, which left the LAST row sitting just below
    // the fold: invisible in jsdom (no layout) and caught only by Playwright's
    // `toBeInViewport`.
    //
    //   rowsStart — where row 0 actually begins in content coordinates.
    //   headroom  — the strip at the top of the viewport the pinned header
    //               paints over, which a revealed row must clear.
    const rowsStart = topInset + headroom;
    const max = Math.max(0, rowsStart + total * pitch - viewportHeight);
    const top = rowsStart + position * pitch;
    const bottom = top + rowHeight;
    if (top - headroom < scrollTop) return Math.min(max, Math.max(0, top - headroom));
    if (bottom > scrollTop + viewportHeight) return Math.max(0, Math.min(max, bottom - viewportHeight));
    return scrollTop;
}

/** Rows in a Ctrl-d / Ctrl-u step. Always at least 1, so the key is never a silent no-op. */
export function halfPageRows(viewportHeight: number, pitch: number): number {
    if (pitch <= 0) return 1;
    return Math.max(1, Math.floor(viewportHeight / pitch / 2));
}

/** The next status in the inline cycle: unmarked → pending → cleared → unmarked. */
export function nextStatus(status: TxnStatus): TxnStatus {
    return status === "unmarked" ? "pending" : status === "pending" ? "cleared" : "unmarked";
}
