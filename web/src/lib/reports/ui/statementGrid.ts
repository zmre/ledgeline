// The column geometry every multi-column statement shares.
//
// The statements are drawn as a stack of separate tables: a box per section,
// with ruled subtotals and summary blocks between them. Left to the browser's
// AUTO table layout, each table sizes its own columns to its own content, so
// once the period columns outgrow the page the label column of each table takes
// a different width and nothing lines up from one block to the next.
//
// The fix is one template for all of them: the same `<colgroup>` widths (see
// `FigureColgroup.svelte`), `table-layout: fixed`, and one width for the whole
// stack (`statementMinWidth`) inside ONE horizontal scroller. Every table then
// has identical column edges, and they all scroll sideways together.
//
// Single-column statements (Interval = Total, a single-date balance sheet) do
// not use any of this and render exactly as before.

/** The line-name column's floor. It takes whatever width is left over above it. */
export const LABEL_REM = 19;
/** One period column. */
export const PERIOD_REM = 8;
/** The Total column, a little wider: it sums the periods. */
export const TOTAL_REM = 9;

/**
 * The width the whole stack is held to: every block in it is then the same
 * width, so with fixed layout and the same colgroup their columns coincide.
 */
export function statementMinWidth(periods: number, total: boolean, compare = false): string {
    const label = compare ? COMPARE_LABEL_REM : LABEL_REM;
    const figures = compare
        ? periods * COMPARE_PERIOD_REM + (total ? TOTAL_REM : 0) + DELTA_REM + DELTA_PCT_REM
        : periods * PERIOD_REM + (total ? TOTAL_REM : 0);
    return `calc(var(${NARROW_LABEL_VAR}, ${label}rem) + ${figures}rem)`;
}

/**
 * Below `md` the label column is sticky (app.css, `.ll-sticky-col`), so it is
 * on screen the whole time the figures scroll under it: at 19rem it would
 * leave a phone one figure column. `app.css` sets this custom property to a
 * narrower floor there; elsewhere it is unset and the floor above applies.
 */
export const NARROW_LABEL_VAR = "--ll-statement-label";

// A projection comparison (plan 34) draws every period twice plus Δ and Δ%,
// so it runs on a tighter template: at the usual one, three actual months
// came to 85rem and pushed Δ% past the 1280px page. On this one they come to
// 74.5rem and fit; wider windows still scroll (see `HScroll.svelte`).

/** The line-name column's floor while comparing. Long names truncate, as they do at the usual width. */
export const COMPARE_LABEL_REM = 15;
/** One period column while comparing: still clears a bold six-figure total. */
export const COMPARE_PERIOD_REM = 7.5;
/** The Δ column: an amount, like a Total. */
export const DELTA_REM = TOTAL_REM;
/** The Δ% column: at most `>999%` or `-100.0%`. */
export const DELTA_PCT_REM = 5.5;
