// One valued amount → one figure plus footnotes.
//
// Extracted from `balanceSheetRows.ts` when the income statement needed the
// identical treatment. It is not a formatting nicety, it is the rule both
// grouped statements rest on: every line is valued into ONE base commodity, so
// a cell has exactly one number in it, and whatever the valuation could NOT
// convert (a holding with no `P` directive) is demoted to a small secondary
// line rather than stacked as a second balance in the same cell.
//
// The alternative — one `<div>` per commodity, which is what `formatTotals`
// did — puts three numbers in the space of one and is what made those tables
// unreadable. Dropping the leftovers instead would be worse: "unpriced
// commodities are surfaced, never silently dropped" is the rule the whole
// redesign rests on.

import type {MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import {fmt} from "$lib/format/amounts";
import {extras as nonBaseLines, fmtBase} from "./insights/format";

export interface AmountCell {
    /** The headline figure. A real formatted zero ("$0.00") when the amount has no base part. */
    text: string;
    /** Whether `text` is negative — the caller paints it `text-error`. */
    negative: boolean;
    /** Non-base commodities, formatted, sorted, zeroes dropped. */
    extras: string[];
}

/**
 * `base` may be null: the engine reports no base commodity for a journal that
 * has none, and there is then no figure to promote. Rather than invent one, the
 * first commodity (in sort order) becomes the headline and the rest stay extras
 * — deterministic, and honest that nothing was converted.
 */
export function amountCell(ma: MixedAmount, base: string | null, styles: ReadonlyMap<string, AmountStyle>): AmountCell {
    if (base !== null) {
        const qty = ma.get(base);
        return {text: fmtBase(ma, base, styles), negative: qty !== undefined && qty.m < 0n, extras: nonBaseLines(ma, base, styles)};
    }
    const sorted = [...ma.entries()].filter(([, qty]) => qty.m !== 0n).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    if (sorted.length === 0) return {text: "0", negative: false, extras: []};
    const [commodity, qty] = sorted[0];
    return {
        text: fmt(commodity, qty, styles),
        negative: qty.m < 0n,
        extras: sorted.slice(1).map(([c, q]) => fmt(c, q, styles)),
    };
}
