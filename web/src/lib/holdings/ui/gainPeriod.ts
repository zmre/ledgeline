// Pure gain-window helpers (WP-10 gain period): map a GainPeriod selection to
// the `gainSince=YYYY-MM-DD` query param the engine's /api/holdings accepts, and
// to the label suffix the UI appends so a windowed gain number isn't misread.
// No Svelte/DOM/$app imports — node-testable, ports cleanly.
//
// The window ALWAYS ends at the scope's asOf (which itself defaults to today),
// so gainSince ≤ asOf by construction and YTD/12mo stay coherent even when the
// user views holdings as of a past date:
//   all  → undefined (send nothing ⇒ all-time gain = marketValue − basis)
//   ytd  → Jan 1 of asOf's year
//   12mo → asOf minus one year (Feb-29 normalizes forward, e.g. 2024-02-29 → 2023-03-01)
import type {GainPeriod} from "$lib/holdings/types";
import type {ISODate} from "$lib/domain/types";

/** Selector options in display order; the first is the default (all-time). */
export const GAIN_PERIODS: ReadonlyArray<{value: GainPeriod; label: string}> = [
    {value: "all", label: "All time"},
    {value: "ytd", label: "Year to date"},
    {value: "12mo", label: "Trailing 12 months"},
];

function pad(n: number): string {
    return String(n).padStart(2, "0");
}

/**
 * The `gainSince` query value for `period`, relative to `asOf`, or undefined for
 * all-time (send no param). Pure date-parts math — never `new Date('YYYY-MM-DD')`
 * (that parses UTC and can drift a day in negative zones).
 */
export function gainSinceFor(period: GainPeriod, asOf: ISODate): string | undefined {
    switch (period) {
        case "all":
            return undefined;
        case "ytd":
            return `${asOf.slice(0, 4)}-01-01`;
        case "12mo": {
            const y = Number(asOf.slice(0, 4));
            const m = Number(asOf.slice(5, 7));
            const d = Number(asOf.slice(8, 10));
            const t = new Date(Date.UTC(y - 1, m - 1, d));
            return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
        }
    }
}

/** Short suffix appended to gain labels/headers so the active window is visible (""/" (YTD)"/" (12mo)"). */
export function gainWindowSuffix(period: GainPeriod): string {
    switch (period) {
        case "all":
            return "";
        case "ytd":
            return " (YTD)";
        case "12mo":
            return " (12mo)";
    }
}
