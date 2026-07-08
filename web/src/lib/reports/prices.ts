// Market-price database + valuation (WP-06). Pure TS: no Svelte/DOM imports.
//
// Direct conversions only: a commodity is valued via the latest P directive
// dated ≤ asOf that prices it directly in the target commodity.
// TODO(post-MVP): chain conversion (e.g. AAPL→$→EUR when no AAPL→EUR price).

import {add, dec, mul, type Dec, type MixedAmount} from "../domain/money";
import type {Amount, ISODate, PriceDirective} from "../domain/types";

export interface PriceDb {
    /** Latest P directive for `commodity` dated ≤ asOf, regardless of what it is priced in. */
    lookup(commodity: string, asOf: ISODate): Amount | null;
    /** Latest P directive dated ≤ asOf pricing `commodity` directly in `target` (contract extension). */
    lookupIn(commodity: string, target: string, asOf: ISODate): Amount | null;
    /** Default valuation target: the most frequent price commodity among the directives; null when there are none (contract extension). */
    baseCommodity(): string | null;
}

export function buildPriceDb(directives: PriceDirective[]): PriceDb {
    const byCommodity = new Map<string, PriceDirective[]>();
    for (const directive of directives) {
        const list = byCommodity.get(directive.commodity);
        if (list === undefined) byCommodity.set(directive.commodity, [directive]);
        else list.push(directive);
    }
    // Stable sort: same-date directives keep journal order, so the last-declared wins on the reverse scan below (hledger semantics).
    for (const list of byCommodity.values()) list.sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));

    let base: string | null = null;
    const counts = new Map<string, number>();
    for (const directive of directives) {
        const target = directive.price.commodity;
        const count = (counts.get(target) ?? 0) + 1;
        counts.set(target, count);
        if (base === null || count > (counts.get(base) ?? 0) || (count === counts.get(base) && target < base)) base = target;
    }

    const latest = (commodity: string, asOf: ISODate, matches: (p: PriceDirective) => boolean): Amount | null => {
        const list = byCommodity.get(commodity);
        if (list === undefined) return null;
        for (let i = list.length - 1; i >= 0; i -= 1) {
            if (list[i].date <= asOf && matches(list[i])) return list[i].price;
        }
        return null;
    };

    return {
        lookup: (commodity, asOf) => latest(commodity, asOf, () => true),
        lookupIn: (commodity, target, asOf) => latest(commodity, asOf, (p) => p.price.commodity === target),
        baseCommodity: () => base,
    };
}

/** Out-param for `valueAt` (contract extension): commodities that had to be skipped. */
export interface ValuationMeta {
    /** Commodities with no direct price to the target at asOf (deduped, in encounter order). */
    unpriced: string[];
}

/**
 * Value a MixedAmount in `target` at `asOf`: identity for `target` itself,
 * exact `mul` via the latest direct price otherwise. Commodities without a
 * direct price are SKIPPED (never guessed) and reported via `meta` when given.
 */
export function valueAt(ma: MixedAmount, target: string, db: PriceDb, asOf: ISODate, meta?: ValuationMeta): Dec {
    let total = dec(0n, 0);
    for (const [commodity, qty] of ma) {
        if (commodity === target) {
            total = add(total, qty);
            continue;
        }
        const price = db.lookupIn(commodity, target, asOf);
        if (price === null) {
            if (meta !== undefined && !meta.unpriced.includes(commodity)) meta.unpriced.push(commodity);
            continue;
        }
        total = add(total, mul(qty, price.qty));
    }
    return total;
}
