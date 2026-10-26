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
export function statementMinWidth(periods: number, total: boolean): string {
    return `${LABEL_REM + periods * PERIOD_REM + (total ? TOTAL_REM : 0)}rem`;
}
