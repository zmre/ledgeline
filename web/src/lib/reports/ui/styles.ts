// Report display styles (WP-07). commodityStyles (WP-03) keeps the FIRST-seen
// style per commodity, which is right for rendering individual postings but
// wrong for aggregated report totals: if the first "AAPL" amount was written
// "10 AAPL" (precision 0), a 19.5 AAPL total would round to "20 AAPL". hledger
// solves this with canonical commodity styles; the JSON API doesn't expose
// `commodity` directives, so the closest we can get is raising each style's
// precision to the maximum ACTUAL decimal places (qty.p) seen. Other amounts'
// declared style precisions are deliberately ignored — cost/conversion postings
// carry inflated asprecision values on the wire (e.g. 4 for a plain $ amount)
// that hledger's own display does not use.

import type {AmountStyle, Transaction} from "$lib/domain/types";
import {commodityStyles} from "$lib/journal/rowModel";

/** First-seen style per commodity, precision raised to the max actual decimal places seen (never lowered). */
export function reportStyles(txns: readonly Transaction[]): Map<string, AmountStyle> {
    const maxPlaces = new Map<string, number>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            for (const amount of posting.amounts) {
                const prev = maxPlaces.get(amount.commodity) ?? 0;
                if (amount.qty.p > prev) maxPlaces.set(amount.commodity, amount.qty.p);
            }
        }
    }
    const styles = new Map<string, AmountStyle>();
    for (const [commodity, style] of commodityStyles(txns)) {
        const precision = Math.max(style.precision, maxPlaces.get(commodity) ?? 0);
        styles.set(commodity, precision === style.precision ? style : {...style, precision});
    }
    return styles;
}
