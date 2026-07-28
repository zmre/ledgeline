// Shared amount/percent display primitives.
//
// Each of these existed two or three times, and the copies had drifted in ways
// that were visible on screen rather than merely untidy:
//
//   * The fallback `AmountStyle` (for a commodity the journal feed gave no
//     style for) was written five times in TWO behaviours — `digitGroups:
//     [",", [3]]` rendering `1,234.56` and `digitGroups: null` rendering
//     `1234.56`. Grouped wins: it is what the report tables, the insights boxes
//     and the holdings table all already do.
//   * Signed percent was written twice, with DIFFERENT MINUS CHARACTERS (ASCII
//     `-` in the holdings view, U+2212 `−` in the insights format helpers) and
//     different treatment of zero (`+0.0%` vs `0.0%`).
//
// U+2212 and an unsigned zero are canonical here. U+2212 is the typographic
// minus — it aligns with the digits and matches `fmtSignedAmount`, which
// already used it. "+0.0%" claims a gain that did not happen, so zero carries
// no sign at all.
//
// NOTE the deliberate limit: `formatDec` in `lib/domain/money.ts` renders
// negative MONEY with an ASCII hyphen (`$-630.00`), so the app still mixes the
// two characters — money via the domain formatter, explicitly-signed display
// figures via this module. Unifying that is a change to the domain layer.

import {dec, formatAmount, neg, type Dec} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";

/** Null / absent cells render as an em-dash everywhere. */
export const EM_DASH = "—";

/** Exact zero, for MixedAmount lookups that miss and for reduce seeds. */
export const ZERO: Dec = dec(0n, 0);

/** |d|, allocating only when the sign actually has to change. */
export function absDec(d: Dec): Dec {
    return d.m < 0n ? neg(d) : d;
}

/**
 * Style for a commodity the journal feed carries no display style for — a
 * period with no postings, an empty journal, or a failed styles fetch.
 *
 * Grouped thousands, two places, symbol on the left: what a reader expects of
 * money, and what every other surface renders when a style IS present.
 */
export const DEFAULT_AMOUNT_STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

/** The display style for `commodity`, falling back to `DEFAULT_AMOUNT_STYLE`. */
export function styleOf(styles: ReadonlyMap<string, AmountStyle>, commodity: string): AmountStyle {
    return styles.get(commodity) ?? DEFAULT_AMOUNT_STYLE;
}

/** Format one commodity amount, e.g. "$1,234.56". */
export function fmt(commodity: string, qty: Dec, styles: ReadonlyMap<string, AmountStyle>): string {
    return formatAmount({commodity, qty, style: styleOf(styles, commodity)});
}

/** Format a percent with an explicit sign: "+12.3%" / "−4.0%" / "0.0%" / "—" when absent. */
export function fmtSignedPct(pct: number | null): string {
    if (pct === null) return EM_DASH;
    const sign = pct > 0 ? "+" : pct < 0 ? "−" : "";
    return `${sign}${Math.abs(pct).toFixed(1)}%`;
}

/** Format a signed base-commodity amount: "+$1,234.56" / "−$40.00" / "—" when absent. */
export function fmtSignedAmount(gain: Dec | null, base: string, styles: ReadonlyMap<string, AmountStyle>): string {
    if (gain === null) return EM_DASH;
    const sign = gain.m > 0n ? "+" : gain.m < 0n ? "−" : "";
    return `${sign}${fmt(base, absDec(gain), styles)}`;
}
