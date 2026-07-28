// Pure display helpers for the holdings UI (WP-10). No Svelte/DOM imports —
// unit-tested under node; the .svelte components stay thin.
import {add, cmp, dec, formatDec, MAX_QUANTITY_DECIMALS, toNumber, type Dec} from "$lib/domain/money";
import type {AmountStyle, Transaction} from "$lib/domain/types";
import {DEFAULT_AMOUNT_STYLE, fmtSignedPct} from "$lib/format/amounts";
import {OTHER_LABEL} from "$lib/format/palette";
import {isCurrency} from "$lib/holdings/commodities";
import type {Holding} from "$lib/holdings/types";

/** Null cells render as an em-dash everywhere on the holdings page. */
export {EM_DASH} from "$lib/format/amounts";

/** The folded pie tail's label — context, not a series identity (muted gray, like the insights chart). */
export const PIE_OTHER = OTHER_LABEL;

/**
 * Accounts that EVER hold a stock commodity (any posting amount in a
 * non-currency commodity, any date) — what the scope chooser offers. Sorted,
 * deduped; scope/asOf deliberately ignored so options never vanish while the
 * user is composing a scope.
 */
export function stockAccounts(txns: readonly Transaction[]): string[] {
    const out = new Set<string>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (posting.amounts.some((a) => !isCurrency(a.commodity))) out.add(posting.account);
        }
    }
    return [...out].sort();
}

/**
 * Split the engine's holdings into the rows the table SHOWS and the net-short
 * rows it hides.
 *
 * A negative share count means more of a symbol was sold than was ever bought,
 * i.e. the opening purchase is missing from the journal. Nobody "holds" −2
 * shares, so the row is noise in a portfolio table — but the balance sheet
 * carries and values exactly those shares, so the engine reports it and counts
 * its (negative) market value in `totals.marketValue`. That is the only way the
 * portfolio total and net worth agree. Hiding it here therefore leaves the
 * visible rows summing to MORE than the total shown beneath them, which is what
 * `shortPositionNote` exists to explain.
 */
export function partitionShortPositions(holdings: readonly Holding[]): {shown: Holding[]; hidden: Holding[]} {
    const shown: Holding[] = [];
    const hidden: Holding[] = [];
    for (const h of holdings) (h.shares.m < 0n ? hidden : shown).push(h);
    return {shown, hidden};
}

/**
 * The muted note under the holdings table naming the rows `partitionShortPositions`
 * hid, why, and how much value they still contribute to the totals. `null` when
 * nothing is hidden (the overwhelmingly common case), which hides the note.
 *
 * The amount is summed EXACTLY over the hidden rows that are priced and only
 * formatted at the end; hidden rows with no price contribute nothing to the
 * totals either, so with no priced row at all the value clause is dropped rather
 * than claiming a $0 contribution.
 */
export function shortPositionNote(hidden: readonly Holding[], format: (v: Dec) => string): string | null {
    if (hidden.length === 0) return null;
    const one = hidden.length === 1;
    const symbols = hidden.map((h) => h.symbol).join(", ");
    const head =
        `${hidden.length} short position${one ? "" : "s"} ${one ? "is" : "are"} hidden (${symbols}): ` +
        `net shares are negative, so the opening purchase${one ? " was" : "s were"} likely never recorded.`;

    const priced = hidden.filter((h): h is Holding & {marketValue: Dec} => h.marketValue !== null);
    if (priced.length === 0) return `${head} No price is known for ${one ? "it" : "them"}, so ${one ? "it adds" : "they add"} nothing to the totals.`;
    const value = priced.reduce((acc, h) => add(acc, h.marketValue), dec(0n, 0));
    return `${head} ${one ? "Its" : "Their"} market value (${format(value)}) is still counted in the totals above.`;
}

export interface PieSlice {
    /** Symbol, or PIE_OTHER for the folded tail. */
    symbol: string;
    /** Display name (tooltip), PIE_OTHER for the tail. */
    name: string;
    /** toNumber(marketValue) — display boundary only. */
    value: number;
    /** Percentage share of the priced total (0–100). */
    share: number;
    /** Exact formatted market value. */
    formatted: string;
}

/**
 * Pie slices by market value: priced holdings only (unpriced are covered by
 * the inline warning), top `maxNamed` keep their symbol, the rest fold into
 * one PIE_OTHER bucket (summed exactly, converted to number only for the
 * slice value). `maxNamed` defaults to 8 — the validated categorical palette
 * has exactly 8 slots and the dataviz rule is to fold, never to cycle hues.
 */
export function pieSlices(holdings: readonly Holding[], format: (v: Dec) => string, maxNamed = 8): PieSlice[] {
    const priced = holdings.filter((h): h is Holding & {marketValue: Dec} => h.marketValue !== null);
    const named = priced.slice(0, maxNamed);
    const tail = priced.slice(maxNamed);

    const slices = named.map((h) => ({symbol: h.symbol, name: h.name, value: toNumber(h.marketValue), formatted: format(h.marketValue)}));
    if (tail.length > 0) {
        const sum = tail.reduce((acc, h) => add(acc, h.marketValue), dec(0n, 0));
        slices.push({symbol: PIE_OTHER, name: PIE_OTHER, value: toNumber(sum), formatted: format(sum)});
    }
    const total = slices.reduce((acc, s) => acc + s.value, 0);
    return slices.map((s) => ({...s, share: total > 0 ? (s.value / total) * 100 : 0}));
}

// Same grouping/point as money; only the precision differs, and it comes from
// the quantity itself (see below).
const SHARES_STYLE: Omit<AmountStyle, "precision"> = DEFAULT_AMOUNT_STYLE;

/**
 * Share quantities for the table.
 *
 * A share count is NOT money: its unit of account is whatever the journal
 * wrote, not the cent, so the 2-place money cap does not apply here — under it
 * a 0.00123456 BTC position read `0` in the Shares column next to a real dollar
 * market value, and 1.00123456 BTC read `1`. Formatted at the quantity's OWN
 * precision (bounded by MAX_QUANTITY_DECIMALS) with trailing fraction zeros
 * trimmed, so whole and half-share counts are unchanged: "19.5" and "17", never
 * "19.50" / "17.0".
 */
export function formatShares(shares: Dec): string {
    const s = formatDec(shares, {...SHARES_STYLE, precision: shares.p}, MAX_QUANTITY_DECIMALS);
    return s.includes(".") ? s.replace(/\.?0+$/, "") : s;
}

/**
 * Gain percent for display: explicit sign, one decimal ("+21.3%", "−3.4%");
 * em-dash when null.
 *
 * This was a second, drifted implementation of `fmtSignedPct`: it wrote its
 * minus as an ASCII hyphen where the insights dashboard wrote U+2212, and it
 * signed ZERO as "+0.0%" — claiming a gain of nothing. Now one function, so the
 * portfolio and the dashboard cannot disagree about what a percent looks like.
 */
export const formatGainPct = fmtSignedPct;

/**
 * How many DISPLAYED holdings are left out of the PARTIAL cost-basis / gain
 * totals because they have no recorded basis (tainted rows still shown on the
 * page). The engine sums basis/gain over only the rows that have one, so this
 * count drives the muted "totals exclude N holding(s)" note under the stat
 * tiles; `0` hides it.
 */
export function untotaledBasisCount(holdings: readonly Holding[]): number {
    return holdings.reduce((n, h) => (h.basis === null ? n + 1 : n), 0);
}

/** Sortable holdings-table columns; `price`/`priceDate` read the nested price field. */
export type SortKey = "name" | "symbol" | "shares" | "basis" | "firstBasisDate" | "price" | "priceDate" | "marketValue" | "gain" | "gainPct";

/** The raw sortable value for a column, null when the holding has none. */
function sortValue(h: Holding, key: SortKey): Dec | number | string | null {
    switch (key) {
        case "name":
            return h.name;
        case "symbol":
            return h.symbol;
        case "shares":
            return h.shares;
        case "basis":
            return h.basis;
        case "firstBasisDate":
            return h.firstBasisDate;
        case "price":
            return h.price?.qty ?? null;
        case "priceDate":
            return h.price?.date ?? null;
        case "marketValue":
            return h.marketValue;
        case "gain":
            return h.gain;
        case "gainPct":
            return h.gainPct;
    }
}

/**
 * Non-mutating sort of the holdings table by one column. Dec columns compare
 * exactly via cmp, gainPct numerically, name/symbol case-insensitively
 * (localeCompare), and ISO dates lexically (chronological by construction).
 * Nulls always sort LAST regardless of direction; equal keys and null ties
 * both break by symbol asc, so the order is deterministic.
 */
export function sortHoldings(holdings: readonly Holding[], key: SortKey, dir: "asc" | "desc"): Holding[] {
    const bySymbol = (a: Holding, b: Holding): number => (a.symbol < b.symbol ? -1 : a.symbol > b.symbol ? 1 : 0);
    const compare = (a: Dec | number | string, b: Dec | number | string): number => {
        if (typeof a === "number" && typeof b === "number") return a - b;
        if (typeof a === "string" && typeof b === "string") {
            if (key === "name" || key === "symbol") return a.toLowerCase().localeCompare(b.toLowerCase());
            return a < b ? -1 : a > b ? 1 : 0; // ISO dates: lexical is chronological
        }
        return cmp(a as Dec, b as Dec);
    };
    return [...holdings].sort((a, b) => {
        const va = sortValue(a, key);
        const vb = sortValue(b, key);
        if (va === null || vb === null) {
            if (va === null && vb === null) return bySymbol(a, b);
            return va === null ? 1 : -1; // nulls last regardless of direction
        }
        const ordered = dir === "asc" ? compare(va, vb) : compare(vb, va);
        return ordered !== 0 ? ordered : bySymbol(a, b);
    });
}
