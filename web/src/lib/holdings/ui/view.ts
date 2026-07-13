// Pure display helpers for the holdings UI (WP-10). No Svelte/DOM imports —
// unit-tested under node; the .svelte components stay thin.
import {add, dec, formatDec, toNumber, type Dec} from "$lib/domain/money";
import type {AmountStyle, Transaction} from "$lib/domain/types";
import {isCurrency} from "$lib/holdings/commodities";
import type {Holding} from "$lib/holdings/types";

/** Null cells render as an em-dash everywhere on the holdings page. */
export const EM_DASH = "—";

/** The folded pie tail's label — context, not a series identity (muted gray, like the insights chart). */
export const PIE_OTHER = "(other)";

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

const SHARES_STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

/**
 * Share quantities for the table: exact Dec formatting capped at 2 decimal
 * places (the app-wide display rule), with trailing fraction zeros trimmed —
 * "19.5" and "17", never "19.50" / "17.0".
 */
export function formatShares(shares: Dec): string {
    const s = formatDec(shares, SHARES_STYLE);
    return s.includes(".") ? s.replace(/\.?0+$/, "") : s;
}

/** Gain percent for display: explicit sign, one decimal ("+21.3%", "-3.4%"); em-dash when null. */
export function formatGainPct(pct: number | null): string {
    if (pct === null) return EM_DASH;
    return `${pct >= 0 ? "+" : ""}${pct.toFixed(1)}%`;
}
