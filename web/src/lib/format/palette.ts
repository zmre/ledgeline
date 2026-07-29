// The ONE categorical chart palette.
//
// This existed three times and had already drifted: the holdings pie carried
// all 8 slots, the journal chart widget carried only the first 6 AND cycled
// them with `% PALETTE.length`, and the holdings trend line carried slot 1
// alone. Cycling is the specific thing the dataviz skill forbids — a 7th series
// took slot 1's blue and became indistinguishable from the 1st.
//
// SLOT ORDER IS LOAD-BEARING, AND IT CHANGED. The order the app used to carry
// (blue, aqua, yellow, green, violet, red, magenta, orange) put `#e66767` red
// next to `#d55181` magenta, a pair separated by only ΔE 7.8 for NORMAL colour
// vision — under the ≥15 floor, i.e. two adjacent pie slices most people cannot
// tell apart. That is a hard fail the skill says secondary encoding does not
// excuse, and the "worst adjacent CVD dE 10.3" claim in the old comments does
// not reproduce. The order below is the skill's own documented dark-mode
// sequence, re-validated against THIS app's daisyUI dark surface (#191e24):
//
//   node scripts/validate_palette.js \
//     "#3987e5,#008300,#d55181,#c98500,#199e70,#d95926,#9085e9,#e66767" \
//     --mode dark --surface "#191e24"
//   → lightness PASS · chroma PASS · CVD ΔE 8.4 PASS · normal-vision ΔE 19.3 PASS
//     · contrast PASS — ALL CHECKS PASS (the 6-slot prefix passes identically)
//
// CVD separation sits in the 6–8 floor band, which the skill permits only with
// secondary encoding — every consumer here supplies it (always-on legends
// carrying the symbol/account name, pad-angle gaps between slices, and full
// tooltips), so identity is never colour-alone.

/** Dark-mode categorical slots 1..8, fixed order. The app theme is dark-only. */
export const CATEGORICAL: readonly string[] = ["#3987e5", "#008300", "#d55181", "#c98500", "#199e70", "#d95926", "#9085e9", "#e66767"];

/** Muted gray for the folded tail — context, not a series identity, so it is deliberately outside the palette. */
export const OTHER_COLOR = "#898781";

/** The folded tail's label. One literal: `series.ts` and `holdings/ui/view.ts` both alias this. */
export const OTHER_LABEL = "(other)";

/**
 * The colour for categorical slot `i` (0-based).
 *
 * Past the last slot this FOLDS to the muted tail colour rather than cycling
 * back to slot 1 — the dataviz non-negotiable. Callers that can produce more
 * groups than there are slots should be folding their data into an `OTHER_LABEL`
 * bucket before they get here; this is the backstop that keeps a slip from
 * silently painting two different series the same hue.
 */
export function colorAt(i: number): string {
    return CATEGORICAL[i] ?? OTHER_COLOR;
}
