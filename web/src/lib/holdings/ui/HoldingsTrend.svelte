<!-- Holdings value-over-time: portfolio market value at each point of the
     window the scope bar's gain period names (a week of days, a year of month
     ends, all time at whatever interval the engine picked), from the native
     /api/holdings[/other]/series endpoints (decoded into HoldingsSeries).

     This is a thin adapter over `$lib/components/PeriodLineChart`, which is
     where the chart conventions live and are documented — x is the bucket
     index, explicit integer ticks, points only at ≤31 buckets, one series so no
     legend box, colours from the shared palette. Everything below the props is
     that component's.

     The portfolio is drawn in the foreground ink, solid and a little heavier:
     it is the subject, and the palette's eight slots belong to the eight
     benchmarks. `overlays` are extra lines drawn over it (the Stocks tab's
     benchmarks): each carries its own palette slot and dash, and from the first
     one on the legend appears, because two lines are never told apart by
     colour alone. `controls` renders in the chart's heading row — the
     benchmark checkboxes — so they sit with the chart they change.

     Basis is intentionally not overlaid yet — it's null whenever any held lot
     is tainted/unpriced (honest-totals rule), so it would be blank for most
     real portfolios. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import PeriodLineChart, {type PeriodSeries} from "$lib/components/PeriodLineChart.svelte";
    import {toNumber} from "$lib/domain/money";
    import {chartColors} from "$lib/format/chartColors.svelte";
    import type {GainPeriod, HoldingsSeries} from "$lib/holdings/types";
    import {trendWindowNote} from "./gainPeriod";
    import {trendLabels} from "./trendView";

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
    }: {
        trend: HoldingsSeries;
        /** The window `trend` was fetched for; names it beside the heading. */
        period?: GainPeriod;
        formatValue: (n: number) => string;
        formatAxis?: (n: number) => string;
        overlays?: readonly PeriodSeries[];
        controls?: Snippet;
    } = $props();

    const labels = $derived(trendLabels(trend.points));
    const values = $derived(trend.points.map((p) => toNumber(p.marketValue)));
    const note = $derived(trendWindowNote(period));
    // The portfolio is ink whatever is overlaid on it, so it never repaints.
    // "Market value" alone; "Your portfolio" once a benchmark sits beside it in the legend.
    const series = $derived<PeriodSeries[]>([
        {name: overlays.length > 0 ? "Your portfolio" : "Market value", values, color: chartColors.flowNet, hero: true},
        ...overlays,
    ]);
</script>

<PeriodLineChart
    heading="Value over time"
    {note}
    {labels}
    {series}
    {formatValue}
    {formatAxis}
    empty={period === "all" ? "No priced holdings yet." : `No priced holdings in the ${note}.`}
    testid="holdings-trend"
    actions={controls}
/>
