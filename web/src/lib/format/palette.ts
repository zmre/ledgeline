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

// ─────────────────────────────────────────────────────────────────────────────
// The diverging pair, for a chart whose subject is SIGN
// ─────────────────────────────────────────────────────────────────────────────
//
// Money in above a baseline and money out below it is a POLARITY job, not an
// identity job, so it takes a diverging encoding — two opposed hues with a
// neutral between them — and not two slots out of CATEGORICAL. Spending slot 1
// and slot 2 on it would say "these are two of the series" when what the chart
// means is "these are the two directions".
//
// THESE ARE NOT `--color-success` / `--color-error`, AND THAT IS DELIBERATE.
// daisyUI dark's own tokens are oklch(76% 0.177 163.2) and oklch(71% 0.194
// 13.4) — text steps, tuned to be read as words on a dark surface, and at very
// nearly the same lightness. As marks they are the textbook deuteranopia
// collapse:
//
//   node scripts/validate_palette.js "#00d390,#ff627d" --mode dark --surface "#191e24"
//   → CVD ΔE 5.1 FAIL — below even the 6 floor, which no amount of secondary
//     encoding excuses, and lightness band FAIL on both
//
// So both hues are held and their lightness moved, which is the skill's own
// snap-to-passing procedure (`color-formula.md`): green to L 0.67 / C 0.14,
// red to L 0.54 / C 0.19, same two hue families the theme already uses.
//
//   node scripts/validate_palette.js "#17af7c,#c4284d" --mode dark --surface "#191e24"
//   → lightness PASS · chroma PASS · CVD ΔE 11.8 PASS · normal-vision ΔE 34.4
//     PASS · contrast PASS — ALL CHECKS PASS
//
// ΔE 11.8 clears the ≥8 target outright, so unlike CATEGORICAL this pair needs
// no secondary encoding to be legal. It gets some anyway and for free: which
// side of the zero rule a bar is on says the direction without reference to
// colour at all.

/** Money in: a bar above the zero rule. oklch(0.67 0.14 163.2). */
export const FLOW_IN = "#17af7c";

/** Money out: a bar below the zero rule. oklch(0.54 0.19 13.4). */
export const FLOW_OUT = "#c4284d";

/**
 * The net line drawn over the two.
 *
 * The diverging midpoint is neutral by rule, and neutral is also what the net
 * IS: a derived summary of the other two rather than a third direction. This is
 * daisyUI dark's `--color-base-content`, 15.62:1 on the #191e24 chart surface
 * (the skill's WCAG check for a lone non-categorical colour — the six
 * categorical checks do not apply to it). Against `FLOW_IN` it is only 2.62:1,
 * which is why the line is drawn twice, the lower copy wider and in the surface
 * colour, so it is always read against the surface and never against a bar.
 */
export const FLOW_NET = "#ecf9ff";
