// Ledgeline's help-sheet sections, in the order the `?` sheet renders them.
// The keymap engine lives in packages/ui (`#ui/keys/`, shared with the
// invoicer, invoicer PLAN J13), and sections are each app's own, so the
// order is handed to it once at startup (`keymap.setGroupOrder`, in
// `+layout.svelte`). A group not listed here still shows, after these.
//
// The middle groups mirror the nav bar's own order (Journal, Holdings,
// Projections, Reports, Imports, Settings), so a reader scanning `?` finds the
// section where they expect the page to be. "Holdings" arrived with plans/14:
// the holdings table's row cursor had been borrowing "Journal" while it was the
// page's only keyboard surface, and once the tab strip added a second one, that
// would have filed one feature under two headings. "Review" (plan 32 D16) and
// "Matches" (plan 35) sit right after "Journal", the page their overlays open
// from.

export const LEDGELINE_KEY_GROUPS = [
    "Global",
    "Navigation",
    "Journal",
    "Review",
    "Matches",
    "Holdings",
    "Projections",
    "Reports",
    "Imports",
    "Settings",
    "Filters",
] as const;

export type LedgelineKeyGroup = (typeof LEDGELINE_KEY_GROUPS)[number];

// The shared keymap types a binding's `group` by these names (`KeyGroupNames`
// in packages/ui): a group not listed above is a type error in web/, not a
// stray section at the end of the `?` sheet.
type LedgelineKeyGroupNames = {[G in LedgelineKeyGroup]: true};
declare module "#ui/keys/types.js" {
    // eslint-disable-next-line @typescript-eslint/no-empty-object-type -- declaration merging
    interface KeyGroupNames extends LedgelineKeyGroupNames {}
}
