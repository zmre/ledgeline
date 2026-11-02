// The review launch buttons' look (plan 32 D18), shared by the Journal toolbar
// and the import screens' "Review Unknowns" / "Review All" (D26, D30).

/**
 * Low-key, but a few shades above the page so they still read as buttons in
 * dark mode. A default `btn` paints `base-200`, which in the dark theme is
 * DARKER than the `base-100` page and sinks into it. One subtle border, so the
 * Journal's ▾ matches the button it splits.
 */
export const REVIEW_BUTTON = "btn btn-sm border-base-content/10 bg-base-content/10 shadow-none hover:border-base-content/20 hover:bg-base-content/15";
