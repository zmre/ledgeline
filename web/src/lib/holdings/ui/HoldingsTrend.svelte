<!-- Holdings value-over-time (post-MVP): portfolio market value at each of the
     last 12 month-ends for the current scope, from the native /api/holdings/series
     endpoint (decoded into HoldingsSeries).

     This is now a thin adapter over `$lib/components/PeriodLineChart`, which is
     where the chart conventions this file used to state now live and are
     documented — x is the bucket index, explicit integer ticks, points only at
     ≤31 buckets, one series so no legend box, colours from the shared palette.
     Everything below the props is that component's.

     Basis is intentionally not overlaid yet — it's null whenever any held lot
     is tainted/unpriced (honest-totals rule), so it would be blank for most
     real portfolios. `PeriodLineChart` now takes it as a second series with
     `dashed: true` whenever that changes. -->
<script lang="ts">
    import PeriodLineChart from "$lib/components/PeriodLineChart.svelte";
    import {toNumber} from "$lib/domain/money";
    import type {HoldingsSeries} from "$lib/holdings/types";

    // formatValue = full-precision (tooltip/hover); formatAxis = compact ticks
    // (e.g. "$1.2K"/"$5.3M") so the left y-axis labels stay short and don't clip.
    // formatAxis defaults to formatValue when the caller doesn't supply a compact one.
    let {trend, formatValue, formatAxis}: {trend: HoldingsSeries; formatValue: (n: number) => string; formatAxis?: (n: number) => string} = $props();

    const labels = $derived(trend.points.map((p) => p.label));
    const values = $derived(trend.points.map((p) => toNumber(p.marketValue)));
</script>

<PeriodLineChart
    heading="Value over time"
    note="last 12 months"
    {labels}
    series={[{name: "Market value", values}]}
    {formatValue}
    {formatAxis}
    empty="No priced holdings in the last 12 months."
    testid="holdings-trend"
/>
