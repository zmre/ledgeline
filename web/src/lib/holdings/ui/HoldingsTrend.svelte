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

     Plan 47 Amendment A1 splits it in two (`view`). VALUE (the default, and
     the only view the Other tab has) is the market value, coloured by the
     period's GAIN (`gain`, the stat tiles' number; null falls back to value
     first-vs-last, which is what the Other tab always uses), with a ▲/▼
     marker and a tooltip line on every bucket where enough money moved in or
     out to show on the axis (./flowMarkers.ts). PERFORMANCE is the cumulative
     time-weighted return since the window's first point, on a percent axis
     anchored on `returnAnchor` (no zero floor) with a 0 % rule, coloured by
     the sign of the last return. Overlays (benchmarks) are only ever handed
     in for Performance — StocksTrend's rule, since they are returns too.

     Basis is intentionally not overlaid yet — it's null whenever any held lot
     is tainted/unpriced (honest-totals rule), so it would be blank for most
     real portfolios. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import PeriodLineChart, {type PeriodSeries} from "#lib/components/PeriodLineChart.svelte";
    import {toNumber} from "#lib/domain/money.js";
    import type {ChartView} from "#lib/holdings/params.js";
    import type {GainPeriod, HoldingsSeries} from "#lib/holdings/types.js";
    import {flowLine, flowMarkers} from "./flowMarkers";
    import {trendWindowNote} from "./gainPeriod";
    import {formatReturn, liveNote, trendLabels} from "./trendView";
    import {gainTrend, paddedDomain, paddedReturnDomain, returnTrend} from "./yAxis";

    // formatValue = full-precision (tooltip/hover); formatAxis = compact ticks
    // (e.g. "$1.2K"/"$5.3M") so the left y-axis labels stay short and don't clip.
    // formatAxis defaults to formatValue when the caller doesn't supply a compact one.
    // Both are MONEY formatters: Performance formats its own percent axis.
    let {
        trend,
        period = "12mo",
        formatValue,
        formatAxis,
        formatFlow,
        overlays = [],
        controls,
        liveHint = null,
        view = "value",
        gain = null,
    }: {
        trend: HoldingsSeries;
        /** The window `trend` was fetched for; names it beside the heading. */
        period?: GainPeriod;
        formatValue: (n: number) => string;
        formatAxis?: (n: number) => string;
        /** Money for the buy/sell marker lines ("Net sold $48,200"); `formatValue` when omitted. */
        formatFlow?: (n: number) => string;
        overlays?: readonly PeriodSeries[];
        controls?: Snippet;
        /** A muted aside after the window note (e.g. why there is no live quote), or null. */
        liveHint?: string | null;
        /** Value (market value) or Performance (return since the first point). */
        view?: ChartView;
        /** The period's gain (the stat tiles' `totals.gain`) the Value line is coloured by; null → value first-vs-last. */
        gain?: number | null;
    } = $props();

    const perf = $derived(view === "perf");
    const labels = $derived(trendLabels(trend.points));
    const values = $derived(perf ? trend.points.map((p) => p.returnPct) : trend.points.map((p) => toNumber(p.marketValue)));
    const windowNote = $derived(trendWindowNote(period));
    const note = $derived(`${windowNote}${liveNote(trend.liveAt)}${trend.liveAt === null && liveHint !== null ? ` · ${liveHint}` : ""}`);
    // The portfolio's colour is its own, whatever is overlaid on it, so an
    // overlay never repaints it. "Market value" / "Return" alone; "Your
    // portfolio" once a benchmark sits beside it in the legend.
    const series = $derived<PeriodSeries[]>([
        {name: overlays.length > 0 ? "Your portfolio" : perf ? "Return" : "Market value", values, hero: true},
        ...overlays,
    ]);
    const direction = $derived(perf ? returnTrend(values) : gainTrend(gain, values));
    // Value: no anchor (an engine from before plan 47, or nothing > 0 in the
    // horizon) keeps the chart's own zero-based domain rather than fitting the
    // window. Performance always pads its own, anchor or not: returns have no
    // floor for the chart's default to fall back on.
    const yDomain = $derived.by(() => {
        const shown = series.map((s) => s.values);
        if (perf) return paddedReturnDomain(trend.returnAnchor, shown);
        return trend.anchor === null ? null : paddedDomain(trend.anchor, shown);
    });

    // A1e: buying and selling, marked on the Value line where it is big enough to see.
    const markers = $derived(
        perf
            ? []
            : flowMarkers(
                  trend.points.map((p) => toNumber(p.flow)),
                  values,
                  yDomain
              )
    );
    const markerAt = $derived(new Map(markers.map((m) => [m.index, m])));
    const tooltipLines = $derived.by(() => {
        if (markers.length === 0) return undefined;
        return (i: number): readonly string[] => {
            const marker = markerAt.get(i);
            return marker === undefined ? [] : [flowLine(marker, formatFlow ?? formatValue)];
        };
    });

    /** Half-width of a marker triangle, and its gap from the line, in px. */
    const MARK = 5;
    const GAP = 4;
    /** ▲ sits above the point, apex up; ▼ below it, apex down. */
    function glyph(x: number, y: number, up: boolean): string {
        if (up) {
            const base = y - GAP;
            return `M${x - MARK},${base}L${x + MARK},${base}L${x},${base - MARK * 1.6}Z`;
        }
        const base = y + GAP;
        return `M${x - MARK},${base}L${x + MARK},${base}L${x},${base + MARK * 1.6}Z`;
    }
</script>

{#snippet flowMarks(scales: {x: (index: number) => number; y: (value: number) => number})}
    <!-- Ink on the page's own background (theme tokens), outlined so it reads
         over the line and the gradient alike; the SHAPE carries the direction,
         never the colour. -->
    <g pointer-events="none" data-flow-markers>
        {#each markers as m (m.index)}
            <path
                d={glyph(scales.x(m.index), scales.y(m.value), m.direction === "in")}
                class="fill-base-content stroke-base-100"
                stroke-width="1.5"
                paint-order="stroke"
                data-flow-marker={m.direction}
                role="img"
                aria-label={flowLine(m, formatFlow ?? formatValue)}
            />
        {/each}
    </g>
{/snippet}

<PeriodLineChart
    heading={perf ? "Performance" : "Value over time"}
    {note}
    {labels}
    {series}
    formatValue={perf ? formatReturn : formatValue}
    formatAxis={perf ? formatReturn : formatAxis}
    empty={period === "all" ? "No priced holdings yet." : `No priced holdings in the ${windowNote}.`}
    testid="holdings-trend"
    actions={controls}
    trend={direction}
    {yDomain}
    includeZero={perf}
    zeroIsData={perf}
    overlay={markers.length > 0 ? flowMarks : undefined}
    {tooltipLines}
/>
