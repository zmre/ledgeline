// The Holdings pie's "by category" slices: each holding's market value split
// by its classification weights and summed per category.
//
// # Why here, and in numbers
//
// The server owns the RULES — tag precedence, how a fund's sector weights meet
// its asset mix (`commodity_profile.rs`) — and hands back, per symbol, a list
// of `(label, weight)` per dimension. What is left is one multiply and one sum,
// and it belongs beside the pie rather than in the engine because nothing
// authoritative rests on it: the weights are Yahoo's float percentages, the
// result is only ever drawn, and no total anywhere is computed from it. So it
// runs in plain numbers at the display boundary (`toNumber`), exactly like the
// by-holding pie beside it, and the market values it splits are the engine's
// own.
//
// Pure, relative imports only (`purity.test.ts`).

import {dec, toNumber, type Dec} from "../domain/money";
import {OTHER_LABEL} from "../format/palette";
import type {Breakdown, CategoryDimension, HoldingsProfiles} from "./profileTypes";
import {CATEGORY_DIMENSIONS} from "./profileTypes";
import type {Holding} from "./types";

/**
 * The slice for value nobody has classified. Distinct from `OTHER_LABEL` (the
 * folded tail of KNOWN categories): one is "too small to name", the other is
 * "we don't know", and a reader must never mistake either for the other.
 */
export const UNCLASSIFIED_LABEL = "(unclassified)";

export type CategorySliceKind = "named" | "other" | "unclassified";

export interface CategorySlice {
    /**
     * Stable, unique identity for keyed rendering: `named:<lower-cased label>`,
     * `other` or `unclassified` — namespaced so a category someone happened
     * to call "(other)" can never collide with the fold.
     */
    key: string;
    label: string;
    kind: CategorySliceKind;
    /**
     * The categorical colour slot (0-based) for a `named` slice; null for the
     * two muted buckets. Assigned by rank, except where a dimension pins a
     * label to a slot (see `PINNED_SLOTS`), so "Equity" is the same colour
     * whatever the portfolio holds.
     */
    slot: number | null;
    /** Summed market value — display only. */
    value: number;
    /** Percentage of the charted total (0–100). */
    share: number;
    /** The same value as a Dec, for the page's money formatter. Rounded to cents; display only. */
    amount: Dec;
    /** How many holdings contribute any value to this slice. */
    holdings: number;
}

/**
 * Labels that always take the same colour slot in a dimension with a small,
 * closed-ish vocabulary. Colour follows identity: Equity is slot 1 in every
 * portfolio and every scope, so switching accounts never repaints it.
 * Open-ended dimensions (sector, industry, category) rank instead.
 */
const PINNED_SLOTS: Partial<Record<CategoryDimension, ReadonlyMap<string, number>>> = {
    assetClass: new Map([
        ["equity", 0],
        ["bonds", 1],
        ["cash", 2],
        ["other assets", 3],
        ["crypto", 4],
    ]),
    securityType: new Map([
        ["stock", 0],
        ["etf", 1],
        ["mutual fund", 2],
        ["money market", 3],
        ["bond", 4],
        ["crypto", 5],
    ]),
};

const NO_PINS: ReadonlyMap<string, number> = new Map();

/** A weight shortfall below this is float noise, not an unclassified share. */
const EPSILON = 1e-6;

/** Merge key: labels differing only in case ("real estate" / "Real Estate") are one category. */
const keyOf = (label: string): string => label.trim().toLowerCase();

function toAmount(value: number): Dec {
    return dec(BigInt(Math.round(value * 100)), 2);
}

/**
 * Pie slices for `dimension`.
 *
 * Only priced holdings with a positive market value are charted (a pie cannot
 * draw a negative wedge, and the unpriced ones are covered by the page's own
 * warning), exactly as the by-holding pie does. Each holding's value is split
 * by its weights; whatever its weights leave over — all of it, for a holding
 * with no profile — goes to `UNCLASSIFIED_LABEL`. The slices therefore always
 * sum to the charted holdings' total.
 *
 * Named categories are sorted by value (ties by label), the top `maxNamed`
 * keep their names, and the rest fold into one `OTHER_LABEL` slice — the
 * dataviz rule the by-holding pie follows too: fold, never generate a ninth
 * hue. The unclassified slice, when there is one, is always last.
 */
export function categorySlices(holdings: readonly Holding[], profiles: HoldingsProfiles | null, dimension: CategoryDimension, maxNamed = 8): CategorySlice[] {
    const named = new Map<string, {label: string; value: number; holdings: Set<string>}>();
    let unclassified = 0;
    const unclassifiedHoldings = new Set<string>();

    for (const holding of holdings) {
        if (holding.marketValue === null) continue;
        const value = toNumber(holding.marketValue);
        if (!(value > 0)) continue;
        const weights = profiles?.profiles.get(holding.symbol)?.breakdown[dimension] ?? [];
        let assigned = 0;
        for (const {label, weight} of weights) {
            if (!(weight > 0)) continue;
            const key = keyOf(label);
            const bucket = named.get(key) ?? {label, value: 0, holdings: new Set<string>()};
            bucket.value += value * weight;
            bucket.holdings.add(holding.symbol);
            named.set(key, bucket);
            assigned += weight;
        }
        const rest = 1 - Math.min(1, assigned);
        if (rest > EPSILON) {
            unclassified += value * rest;
            unclassifiedHoldings.add(holding.symbol);
        }
    }

    const ranked = [...named.entries()].sort(([ka, a], [kb, b]) => b.value - a.value || (ka < kb ? -1 : ka > kb ? 1 : 0));
    const kept = ranked.slice(0, maxNamed);
    const tail = ranked.slice(maxNamed);
    const slots = assignSlots(
        kept.map(([key]) => key),
        PINNED_SLOTS[dimension] ?? NO_PINS,
        maxNamed
    );

    const slices: Omit<CategorySlice, "share">[] = kept.map(([key, bucket], i) => ({
        key: `named:${key}`,
        label: bucket.label,
        kind: "named",
        slot: slots[i],
        value: bucket.value,
        amount: toAmount(bucket.value),
        holdings: bucket.holdings.size,
    }));
    if (tail.length > 0) {
        const value = tail.reduce((sum, [, bucket]) => sum + bucket.value, 0);
        const contributors = new Set(tail.flatMap(([, bucket]) => [...bucket.holdings]));
        slices.push({key: "other", label: OTHER_LABEL, kind: "other", slot: null, value, amount: toAmount(value), holdings: contributors.size});
    }
    if (unclassified > 0) {
        slices.push({
            key: "unclassified",
            label: UNCLASSIFIED_LABEL,
            kind: "unclassified",
            slot: null,
            value: unclassified,
            amount: toAmount(unclassified),
            holdings: unclassifiedHoldings.size,
        });
    }
    const total = slices.reduce((sum, slice) => sum + slice.value, 0);
    return slices.map((slice) => ({...slice, share: total > 0 ? (slice.value / total) * 100 : 0}));
}

/**
 * Colour slots for ranked keys: a pinned key takes its pinned slot; every
 * other key takes, in rank order, the lowest slot no label is pinned to — and
 * only once those run out, a pinned slot whose label is absent. So a portfolio
 * with no bonds does not paint its real estate in the bonds colour unless it
 * has run out of others. Never more distinct slots than `count`.
 */
function assignSlots(keys: readonly string[], pinned: ReadonlyMap<string, number>, count: number): number[] {
    const reserved = new Set([...pinned.values()].filter((slot) => slot < count));
    const used = new Set(keys.map((key) => pinned.get(key)).filter((slot): slot is number => slot !== undefined && slot < count));
    const all = Array.from({length: count}, (_, slot) => slot);
    const free = [...all.filter((slot) => !reserved.has(slot)), ...all.filter((slot) => reserved.has(slot) && !used.has(slot))];
    let next = 0;
    return keys.map((key) => {
        const pin = pinned.get(key);
        if (pin !== undefined && pin < count) return pin;
        const slot = free[next] ?? count;
        next += 1;
        return slot;
    });
}

/**
 * The category dimensions worth offering for these holdings: those for which
 * at least one of them has any classification at all. A dimension nobody has
 * data for would be a pie of one grey "(unclassified)" disc.
 */
export function availableDimensions(holdings: readonly Holding[], profiles: HoldingsProfiles | null): CategoryDimension[] {
    if (profiles === null) return [];
    const breakdowns: Breakdown[] = holdings.flatMap((holding) => {
        const profile = profiles.profiles.get(holding.symbol);
        return profile === undefined ? [] : [profile.breakdown];
    });
    return CATEGORY_DIMENSIONS.filter((dimension) => breakdowns.some((breakdown) => breakdown[dimension].length > 0));
}
