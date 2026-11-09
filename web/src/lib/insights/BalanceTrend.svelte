<!-- The pinned account's balance over time (plan 36 amendment A1): what the
     Balances tab draws in place of its cash pie while an account is pinned,
     from /api/journal/running/series (decoded into BalanceSeries).

     This is a thin adapter over `#lib/components/PeriodLineChart`, where the
     chart conventions live and are documented — x is the period index, explicit
     integer ticks, points only at ≤31 periods, one series so no legend box.
     What it adds, through that component's extension points:

     - ONE LINE, IN THE FOREGROUND INK, ZERO IN FRAME. Values as written, so a
       liability plots below zero exactly as the Balance column shows it.
     - THE HEADING NAMES THE ACCOUNT, COMMODITY AND WINDOW, because the account
       may not be in the list beside the chart (a brokerage pinned from a
       report) and the window is the journal filter's, not this panel's.
     - FAILING ASSERTIONS ARE A SHAPE, NOT A COLOUR. A diamond on the line's
       point for the period, and a hollow circle where the line would be had the
       books matched the assertion, joined by a solid hairline: the vertical
       distance IS the "off by". Passing assertions are not drawn — the column
       shows them, and here they would only crowd the line. The details are in
       the tooltip (and the marker's `<title>`, for a screen reader).
     - CLICKING A PERIOD SCROLLS THE TABLE to the newest transaction counted in
       it, through the Problems drawer's `requestFocus`; a row the filter hides
       is simply not found, so nothing happens. -->
<script lang="ts">
    import PeriodLineChart from "#lib/components/PeriodLineChart.svelte";
    import {chartColors} from "#lib/format/chartColors.svelte.js";
    import type {BalanceSeries} from "#lib/journalView/types.js";
    import {problems} from "#lib/stores/problems.svelte.js";
    import {balanceLabels, balanceValues, failureMarkers, markerExtent, periodLines, periodTarget, trendNote} from "./balanceTrend";
    import {formatChartValue, formatCompactChartValue} from "./series";

    let {
        series,
        onJump = (tindex: number) => problems.requestFocus(tindex),
    }: {
        series: BalanceSeries;
        /** Scroll the journal to the transaction with this index. */
        onJump?: (tindex: number) => void;
    } = $props();

    const labels = $derived(balanceLabels(series));
    const markers = $derived(failureMarkers(series));
    const lines = $derived([{name: "Balance", values: balanceValues(series), color: chartColors.flowNet, hero: true}]);

    /** Half the diamond's diagonal, in px. */
    const DIAMOND = 5;

    function jump(index: number): void {
        const tindex = periodTarget(series, index);
        if (tindex !== null) onJump(tindex);
    }
</script>

{#snippet failures({x, y}: {x: (index: number) => number; y: (value: number) => number})}
    <!-- `pointer-events-none` so the markers never take the hover from the
         chart's own tooltip, which carries the same detail. -->
    {#each markers as marker (marker.index)}
        {@const cx = x(marker.index)}
        {@const cy = y(marker.balance)}
        <g class="pointer-events-none" data-failure={marker.index}>
            <title>{marker.title}</title>
            <line x1={cx} x2={cx} y1={cy} y2={y(marker.expected)} stroke={chartColors.flowOut} stroke-width="1" data-failure-gap />
            <circle {cx} cy={y(marker.expected)} r="3.5" class="fill-base-100" stroke={chartColors.flowOut} stroke-width="1.5" data-failure-expected />
            <path
                d="M {cx} {cy - DIAMOND} L {cx + DIAMOND} {cy} L {cx} {cy + DIAMOND} L {cx - DIAMOND} {cy} Z"
                fill={chartColors.flowOut}
                class="stroke-base-100"
                stroke-width="1"
                data-failure-marker
            />
        </g>
    {/each}
{/snippet}

<PeriodLineChart
    heading={series.account}
    note={trendNote(series)}
    {labels}
    series={lines}
    formatValue={(n) => formatChartValue(n, series.commodity, series.style)}
    formatAxis={(n) => formatCompactChartValue(n, series.commodity, series.style)}
    includeZero
    extent={markerExtent(markers)}
    overlay={markers.length > 0 ? failures : undefined}
    tooltipLines={(i) => periodLines(series, i)}
    onPeriodClick={jump}
    empty="The balance is zero throughout this range."
    height="h-40"
    maxPoints={12}
    testid="balance-trend"
/>
