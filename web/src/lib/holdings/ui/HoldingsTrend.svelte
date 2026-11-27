<!-- Holdings value-over-time: portfolio market value at each point of the
     window the scope bar's gain period names (a week of days, a year of month
     ends, all time at whatever interval the engine picked), from the native
     /api/holdings[/other]/series endpoints (decoded into HoldingsSeries).

     This is a thin adapter over `#lib/components/PeriodLineChart`, which is
     where the chart conventions live and are documented — x is the bucket
     index, explicit integer ticks, points only at ≤31 buckets, one series so no
     legend box, colours from the shared palette. Everything below the props is
     that component's.

     The portfolio is drawn solid and a little heavier: it is the subject, and
     it says which way the window went (plan 47 D4) — green up, red down, the
     foreground ink when unchanged, with the area beneath it filled in that
     colour (PeriodLineChart's `trend`). Never a palette slot: on Stocks the
     palette's first eight belong to the eight benchmarks. Both tabs draw it the
     same way. The y-axis is anchored on the engine's `anchor` (D1–D3, see
     ./yAxis.ts) so a quiet week does not fill the plot like a crash.
     `overlays` are extra lines drawn over it (the Stocks tab's benchmarks):
     each carries its own palette slot and dash, joins the y-domain so it is
     never clipped, and from the first one on the legend appears, because two
     lines are never told apart by colour alone. `controls` renders in the
     chart's heading row — the benchmark checkboxes — so they sit with the
     chart they change.

     While the last point is an intraday quote (`trend.liveAt`, D10) the note
     says so: "· live 2:35 PM (delayed)". `liveHint` is a muted aside for when
     a live quote was wanted and could not be had (a rate limit).

     Basis is intentionally not overlaid yet — it's null whenever any held lot
     is tainted/unpriced (honest-totals rule), so it would be blank for most
     real portfolios. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import PeriodLineChart, {type PeriodSeries} from "#lib/components/PeriodLineChart.svelte";
    import {toNumber} from "#lib/domain/money.js";
    import type {GainPeriod, HoldingsSeries} from "#lib/holdings/types.js";
    import {trendWindowNote} from "./gainPeriod";
    import {liveNote, trendLabels} from "./trendView";
    import {paddedDomain, trendOf} from "./yAxis";

    // formatValue = full-precision (tooltip/hover); formatAxis = compact ticks
    // (e.g. "$1.2K"/"$5.3M") so the left y-axis labels stay short and don't clip.
    // formatAxis defaults to formatValue when the caller doesn't supply a compact one.
    let {
        trend,
        period = "12mo",
        formatValue,
        formatAxis,
        overlays = [],
        controls,
        liveHint = null,
    }: {
        trend: HoldingsSeries;
        /** The window `trend` was fetched for; names it beside the heading. */
        period?: GainPeriod;
        formatValue: (n: number) => string;
        formatAxis?: (n: number) => string;
        overlays?: readonly PeriodSeries[];
        controls?: Snippet;
        /** A muted aside after the window note (e.g. why there is no live quote), or null. */
        liveHint?: string | null;
    } = $props();

    const labels = $derived(trendLabels(trend.points));
    const values = $derived(trend.points.map((p) => toNumber(p.marketValue)));
    const windowNote = $derived(trendWindowNote(period));
    const note = $derived(`${windowNote}${liveNote(trend.liveAt)}${trend.liveAt === null && liveHint !== null ? ` · ${liveHint}` : ""}`);
    // The portfolio's colour is its trend, whatever is overlaid on it, so an
    // overlay never repaints it. "Market value" alone; "Your portfolio" once a
    // benchmark sits beside it in the legend.
    const series = $derived<PeriodSeries[]>([{name: overlays.length > 0 ? "Your portfolio" : "Market value", values, hero: true}, ...overlays]);
    const direction = $derived(trendOf(values));
    // No anchor (an engine from before plan 47, or nothing > 0 in the horizon)
    // keeps the chart's own zero-based domain rather than fitting the window.
    const yDomain = $derived(
        trend.anchor === null
            ? null
            : paddedDomain(
                  trend.anchor,
                  series.map((s) => s.values)
              )
    );
</script>

<PeriodLineChart
    heading="Value over time"
    {note}
    {labels}
    {series}
    {formatValue}
    {formatAxis}
    empty={period === "all" ? "No priced holdings yet." : `No priced holdings in the ${windowNote}.`}
    testid="holdings-trend"
    actions={controls}
    trend={direction}
    {yDomain}
/>
