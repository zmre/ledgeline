// Where a statement's actual columns end and its projected ones begin
// (plans/28-projected-statements.md, Phase 5).
//
// The Projections tab renders the Reports tab's own statement views over real
// periods followed by forecast ones, and marks the boundary: a divider down the
// first projected column, and "Projected" over the forecast columns. Every view
// asks the same two questions of a column index, so they are answered once.
//
// `projectedFrom` is the index of the first FORECAST column, or null when the
// statement has none (every Reports-tab statement), in which case nothing here
// adds anything and the views render exactly as before.

/** Whether column `at` is a forecast. */
export function isProjected(at: number, projectedFrom: number | null): boolean {
    return projectedFrom !== null && at >= projectedFrom;
}

/**
 * The divider's classes for column `at`: a rule down the left edge of the first
 * projected column, and nothing anywhere else. Static literals, so Tailwind's
 * scanner sees them.
 */
export function projectedEdge(at: number, projectedFrom: number | null): string {
    return projectedFrom !== null && at === projectedFrom ? "border-l-2 border-l-primary/50" : "";
}

/**
 * The header band above a statement's column headings: how many columns are
 * actuals and how many projected, or null when there is nothing to mark.
 */
export function projectedSpans(columns: number, projectedFrom: number | null): {actual: number; projected: number} | null {
    if (projectedFrom === null || projectedFrom >= columns) return null;
    return {actual: projectedFrom, projected: columns - projectedFrom};
}
