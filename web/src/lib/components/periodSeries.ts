// The series contract of `PeriodLineChart.svelte` — see that file's header for
// the conventions it draws them by.

/** One line. `values` is index-aligned to the chart's `labels`. */
export interface PeriodSeries {
    /** Legend and tooltip name. */
    name: string;
    /**
     * One value per bucket. `null` is a GAP — no value there, drawn as a
     * break in the line — and a number is a value, zero included. The two
     * are different claims and only the caller knows which it has. (An
     * array shorter than `labels` reads as zero past its end, as before.)
     */
    values: readonly (number | null)[];
    /** Draw dashed: for a projection, a target, or any derived line beside actuals. */
    dashed?: boolean;
    /**
     * Palette slot for this line, overriding its position. For a series
     * that comes and goes, so it keeps its colour and never repaints the
     * others.
     */
    slot?: number;
}
