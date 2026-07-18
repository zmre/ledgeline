// Pure row-model helpers for the journal view (WP-03): filter/totals
// derivations, the source→destination arrow heuristic, display amounts, and
// the fixed-pitch windowing math for the virtualized table. No runtime Svelte
// imports (the filter shape is a type-only import) so vitest runs these in node.

import {accountMatches} from "$lib/domain/accounts";
import {add, formatAmount, isZero, neg, type Dec, type MixedAmount} from "$lib/domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction} from "$lib/domain/types";
import type {JournalFilter} from "$lib/stores/filters.svelte";

/** Display order: date descending, then hledger index descending. Returns a new array. */
export function sortTxnsDesc(txns: readonly Transaction[]): Transaction[] {
    return [...txns].sort((a, b) => (a.date === b.date ? b.index - a.index : a.date < b.date ? 1 : -1));
}

function matchesSelection(account: string, selected: readonly string[]): boolean {
    return selected.some((sel) => accountMatches(sel, account));
}

/**
 * WP-03 filter semantics: date range inclusive on both ends against `txn.date`;
 * a txn matches the account selection when ANY posting matches ANY selected
 * account (empty selection = all); query is a case-insensitive substring
 * match against the precomputed `txn.haystack`.
 */
export function filterTxns(txns: readonly Transaction[], filter: JournalFilter): Transaction[] {
    const query = filter.query.trim().toLowerCase();
    const selected = filter.accounts.size > 0 ? [...filter.accounts] : null;
    return txns.filter(
        (txn) =>
            (filter.from === null || txn.date >= filter.from) &&
            (filter.to === null || txn.date <= filter.to) &&
            (selected === null || txn.postings.some((posting) => matchesSelection(posting.account, selected))) &&
            (query === "" || txn.haystack.includes(query))
    );
}

/**
 * Sum postings whose account matches the selection (ALL postings when the
 * selection is empty) across the given — already filtered — transactions.
 * Commodities that sum to zero are dropped.
 */
export function filteredTotals(txns: readonly Transaction[], accounts: ReadonlySet<string>): MixedAmount {
    const selected = accounts.size > 0 ? [...accounts] : null;
    const totals: MixedAmount = new Map();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (selected !== null && !matchesSelection(posting.account, selected)) continue;
            for (const amount of posting.amounts) {
                const prev = totals.get(amount.commodity);
                totals.set(amount.commodity, prev === undefined ? amount.qty : add(prev, amount.qty));
            }
        }
    }
    for (const [commodity, qty] of totals) {
        if (isZero(qty)) totals.delete(commodity);
    }
    return totals;
}

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

/** First-seen display style per commodity (MixedAmount totals carry no style of their own). */
export function commodityStyles(txns: readonly Transaction[]): Map<string, AmountStyle> {
    const styles = new Map<string, AmountStyle>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            for (const amount of posting.amounts) {
                if (!styles.has(amount.commodity)) styles.set(amount.commodity, amount.style);
            }
        }
    }
    return styles;
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
