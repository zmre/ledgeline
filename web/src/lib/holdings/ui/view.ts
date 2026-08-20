// Pure display helpers for the holdings UI (WP-10). No Svelte/DOM imports —
// unit-tested under node; the .svelte components stay thin.
import {add, cmp, dec, formatAmount, formatDec, MAX_QUANTITY_DECIMALS, toNumber, type Dec, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import {DEFAULT_AMOUNT_STYLE, fmtSignedPct} from "$lib/format/amounts";
import {OTHER_LABEL} from "$lib/format/palette";
import type {Holding, OtherHolding} from "$lib/holdings/types";

/** Null cells render as an em-dash everywhere on the holdings page. */
export {EM_DASH} from "$lib/format/amounts";

/** The folded pie tail's label — context, not a series identity (muted gray, like the insights chart). */
export const PIE_OTHER = OTHER_LABEL;

// `stockAccounts` used to live here: "every account that ever held a
// non-currency commodity", walked over the journal feed, offered as the Stocks
// scope chooser's options. It is gone, and deliberately not deprecated.
//
// It was a re-derivation of engine logic that had drifted out of correctness the
// moment plans/14 landed: a home booked as `1 HOME` (the only way P directives
// can revalue it) holds a non-currency commodity, so it was offered as a STOCKS
// filter — where the `holdings: other` tag guarantees selecting it produces an
// empty report. The tag is invisible to the SPA, whose account declarations
// carry a name and a type, so no local heuristic can fix that.
//
// Both tabs now read their option list off their own report
// (`HoldingsReport.accounts` / `OtherHoldingsReport.accounts`), which the engine
// computes from the same membership rule that decides the rows, and which
// `crates/ledgeline-core/tests/other_holdings.rs` pins as disjoint.

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

// --- Other holdings (plans/14) ---------------------------------------------

/**
 * The "Holding" cell: the account's balance as the JOURNAL writes it ("1 HOUSE"),
 * or `""` when the only commodity it holds is the base.
 *
 * The blank is the point. A van booked as `$18,000.00` would otherwise print its
 * own dollar balance immediately left of the Value column showing the same
 * figure, which reads as a rounding discrepancy waiting to happen rather than as
 * information. A house booked as `1 HOUSE` prints its unit, and that unit is the
 * evidence that the row can revalue at all.
 *
 * The test is against `base`, NOT emptiness, and that distinction is the wire's:
 * `commodities` is a `WireMixed` that omits ZERO-valued commodities, so a
 * dollar-booked account still arrives as `{"$": {...}}` — never as `{}`. An
 * emptiness check would therefore blank nothing and print every van's balance
 * twice. (`{}` is unreachable for a real row anyway: membership requires a
 * non-zero balance.)
 *
 * `formatUnits` is injected (rather than a style map imported here) so this stays
 * a pure string function: the caller owns the journal feed the styles come from.
 */
export function formatHeldCommodities(commodities: MixedAmount, base: string, formatUnits: (commodity: string, qty: Dec) => string): string {
    const entries = [...commodities.entries()];
    if (entries.every(([commodity]) => commodity === base)) return "";
    return entries.map(([commodity, qty]) => formatUnits(commodity, qty)).join(", ");
}

/**
 * A unit-count formatter for `formatHeldCommodities`.
 *
 * `MAX_QUANTITY_DECIMALS`, not the 2-place money cap: `1 HOUSE` and `0.5 ACME`
 * are counts in whatever unit the journal chose, not cents — the same argument
 * `formatShares` makes, and the same bug avoided (a 0.00123456 balance reading
 * `0` beside a real dollar value).
 */
export function formatUnitsWith(styleOf: (commodity: string) => AmountStyle): (commodity: string, qty: Dec) => string {
    return (commodity, qty) => formatAmount({commodity, qty, style: styleOf(commodity)}, MAX_QUANTITY_DECIMALS);
}

/** Sortable other-holdings columns. No `holding` column: a mixed amount has no single order. */
export type OtherSortKey = "name" | "account" | "value" | "cost" | "change" | "changePct";

/** The raw sortable value for a column, null when the row has none. */
function otherSortValue(h: OtherHolding, key: OtherSortKey): Dec | number | string | null {
    switch (key) {
        case "name":
            return h.name;
        case "account":
            return h.account;
        case "value":
            return h.value;
        case "cost":
            return h.cost;
        case "change":
            return h.change;
        case "changePct":
            return h.changePct;
    }
}

/**
 * Non-mutating sort of the Other table by one column — `sortHoldings`' rules,
 * re-keyed on the account.
 *
 * Ties break by ACCOUNT rather than by symbol because that is this table's row
 * identity (rows are flat, one per posting-bearing account), and it is the only
 * field guaranteed unique: two accounts may share a `name:` tag, and a journal
 * with a house and a cabin both named "Home" would otherwise reorder on every
 * render.
 */
export function sortOtherHoldings(holdings: readonly OtherHolding[], key: OtherSortKey, dir: "asc" | "desc"): OtherHolding[] {
    const byAccount = (a: OtherHolding, b: OtherHolding): number => (a.account < b.account ? -1 : a.account > b.account ? 1 : 0);
    const compare = (a: Dec | number | string, b: Dec | number | string): number => {
        if (typeof a === "number" && typeof b === "number") return a - b;
        if (typeof a === "string" && typeof b === "string") return a.toLowerCase().localeCompare(b.toLowerCase());
        return cmp(a as Dec, b as Dec);
    };
    return [...holdings].sort((a, b) => {
        const va = otherSortValue(a, key);
        const vb = otherSortValue(b, key);
        if (va === null || vb === null) {
            if (va === null && vb === null) return byAccount(a, b);
            return va === null ? 1 : -1; // nulls last regardless of direction
        }
        const ordered = dir === "asc" ? compare(va, vb) : compare(vb, va);
        return ordered !== 0 ? ordered : byAccount(a, b);
    });
}
