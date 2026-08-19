// Holdings sub-tab vocabulary (plans/14). Pure module (no Svelte/DOM imports)
// so the round-trip is unit-testable under node — the vitest project here is
// `node`-only and excludes `*.svelte.test.ts`, so anything living in a component
// is untested by construction. `holdings/purity.test.ts` enforces the same rule
// mechanically, which is why nothing below imports anything at all.
//
// Scheme: `?tab=other`, omitted for `stocks`. Where `lib/imports/params.ts` also
// owns the SERIALIZATION, this file stops at the vocabulary — the holdings
// screen already writes `?asof=&acct=&mode=&gain=` from `ui/urlCodec.ts`, and a
// second codec over the same query string would be a second writer, which is the
// one thing plans/14 rules out ("one writer, no second searchMirror"). So the
// tab joins that codec and this module is just the closed set both halves agree
// on.
//
// The tab is deliberately NOT part of `HoldingsScope` either: scope is the
// resource's refetch key (`stores/holdings.svelte.ts`), so a tab living there
// would refetch the stock report on every tab click.

export type HoldingsTab = "stocks" | "other";

/** "Stocks" is the first (default) tab — the screen Holdings has always opened on. */
export const TAB_ORDER: HoldingsTab[] = ["stocks", "other"];

export const TAB_LABELS: Record<HoldingsTab, string> = {
    stocks: "Stocks",
    // "Other" rather than "Other assets": the strip sits directly under the
    // Holdings nav item, so the noun is already on screen, and the longer label
    // would read as a category that Stocks is somehow not part of.
    other: "Other",
};

/** Narrow an arbitrary query value to a tab id — an unknown one is not a tab. */
export const isTab = (v: string): v is HoldingsTab => (TAB_ORDER as string[]).includes(v);
