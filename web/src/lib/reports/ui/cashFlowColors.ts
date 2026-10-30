// The cash flow chart's colours (plans/31, A1): each activity is drawn in the
// colour of its heading in the statement below it, so the chart and the
// statement read as one thing.
//
// The STATEMENT is the source. `CashFlowStatementView.svelte`'s `ACCENT` gives
// operating `text-info`, investing `text-secondary` and financing
// `text-accent`; this map names the same daisyUI colours as `ThemePin`s, which
// `periodStack` resolves through the live palette, so a theme switch recolours
// the bars and the headings together. `ACCENT` stays its own static map because
// Tailwind's scanner needs the literal classes; `cashFlowColors.test.ts` reads
// it and fails if the two maps ever name different colours.
//
// The effect of exchange rates is a residual, not an activity, and the
// statement gives it no accent, so it takes the palette's muted `other` grey.

import type {ThemePin} from "#lib/format/palette.js";
import type {CfSectionKind} from "../types";

/** Each activity's series colour: the daisyUI colour of its heading in the statement. */
export const CF_PIN: Readonly<Record<CfSectionKind, ThemePin>> = Object.freeze({
    operating: "info",
    investing: "secondary",
    financing: "accent",
});

/** The effect of exchange rates: muted, because it is not an activity. */
export const CF_EXCHANGE_PIN: ThemePin = "other";
