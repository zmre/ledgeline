<!-- Signed series stacked away from a zero rule over shared period buckets —
     positives up, negatives down — with the net drawn over the top as a dashed
     line. The one stacked-diverging period chart: `PeriodFlowChart` (money in /
     money out) is this with two series, the Net Worth report is this with one
     bar series per account, and the Cash Flow report is this as areas.

     Props are plain index-aligned number arrays plus labels, and nothing about
     reports, projections or `PeriodReport`: a caller that can produce signed
     columns of numbers can render this. `$lib/reports/ui/periodStack` is the
     model that turns a report into them.

     The specs (the dataviz skill's, first set down in `PeriodFlowChart`):

     - X IS THE BUCKET INDEX with explicit integer ticks — see `periodAxis.ts`.
     - THE ZERO RULE IS ALWAYS IN FRAME: `yDomain` is seeded at [0, 0] and
       widened by each bucket's positive stack, negative stack AND the net, so a
       net outside the stacks is scaled rather than clipped.
     - A SERIES MUST BE ONE SIGN. A layer of a stacked area is one band from
       one edge to the other, so it cannot cross the axis; a caller with an
       account that flips sign splits it into halves sharing one `entity`
       (`periodStack.splitBySign`). Halves share a legend entry and a tooltip row.
     - BARS: capped at 24px and centred in their band (the width is measured),
       4px rounded data-end, square at the baseline, no outline. `stackGap`
       inserts the surface gap between stacked segments.
     - AREAS: stacked here (`bandsAt`), not by layerchart, so a zero keeps its
       place on the stack; filled at `AREA_OPACITY`, no top line, a hairline of
       surface between layers.
     - THE NET IS DRAWN TWICE, a wide surface-coloured halo under the dashed
       ink, so it reads against the surface rather than against whichever mark
       it crosses. `netLabelAt` adds a dot and a value label at those buckets
       only; the caller decides which (never every point by default).
     - ONE TOOLTIP PER BUCKET listing every non-zero series in legend order —
       values lead, labels follow, keyed by a short line — then the net.
     - The LEGEND is always shown, one entry per entity (a folded "(other)" on
       both sides is one entry), with the net as a dashed line key: identity is
       never colour alone. -->
<script lang="ts">
    import {Area, AreaChart, BarChart, Spline, Tooltip} from "layerchart";
    import {chartColors} from "$lib/format/chartColors.svelte";
    import {labelFormatter, tickIndices} from "./periodAxis";
    import {stackEmptyReason} from "./periodStackEmpty";

    /** One drawn series. Every value must share one sign (zeros aside). */
    interface Series {
        key: string;
        /** The legend entry / tooltip row this series belongs to; defaults to `key`. */
        entity?: string;
        label: string;
        color: string;
        values: readonly number[];
    }

    let {
        heading,
        note,
        labels,
        series,
        net,
        netLabel = "Net",
        netLabelAt = [],
        mark = "bar",
        stackGap = 0,
        minBuckets = 1,
        formatValue,
        formatAxis,
        empty = "Nothing to chart in this range.",
        tooShort = "Only one period in this range — widen it to see a trend.",
        height = "h-56 sm:h-64",
        testid,
    }: {
        /** Names what is plotted. Omit when a surrounding panel header already does. */
        heading?: string;
        /** Muted qualifier beside the heading, rendered after a separating "·". */
        note?: string;
        /** One label per bucket; its length is the bucket count. */
        labels: readonly string[];
        /** Stacked outward from zero in this order, positives up and negatives down. */
        series: readonly Series[];
        /** The net per bucket, signed. Drawn as given: nothing reconciles it with the stacks. */
        net: readonly number[];
        netLabel?: string;
        /** Bucket indices that get a net dot and value label. */
        netLabelAt?: readonly number[];
        mark?: "bar" | "area";
        /** Px of surface between stacked bar segments (dataviz: 2 when segments touch). */
        stackGap?: number;
        /** Fewer buckets than this shows `tooShort` instead of a plot (`stackEmptyReason`). */
        minBuckets?: number;
        /** Full precision, for the tooltip and net labels. */
        formatValue: (n: number) => string;
        /** Compact, for the y-axis ticks. Defaults to `formatValue`. */
        formatAxis?: (n: number) => string;
        /** Shown instead of the plot when there is nothing to draw. */
        empty?: string;
        tooShort?: string;
        /** Height utilities for the plot box. */
        height?: string;
        testid?: string;
    } = $props();

    const axisFormat = $derived(formatAxis ?? formatValue);

    const PAD_LEFT = 56;
    const PAD_RIGHT = 8;
    const BAND_PADDING = 0.4;
    /** dataviz mark spec: a bar never exceeds this, however much room its band has. */
    const MAX_BAR_WIDTH = 24;
    /**
     * Stacked-area fill. The dataviz ~10% wash is for areas that OVERLAP; stacked
     * layers never overlap, they tile, and at 10% neighbouring layers on the dark
     * surface are indistinguishable. Opaque enough to tell the layers apart,
     * translucent enough that the dashed net over them stays the loudest mark.
     */
    const AREA_OPACITY = 0.55;

    interface Row {
        i: number;
        /** One value per series, index-aligned with `series`. */
        v: number[];
        /** Each series' stacked `[y0, y1]` band, index-aligned with `series`. */
        band: [number, number][];
        net: number;
    }

    /**
     * Each series' band in one bucket: stacked outward from zero, positives on
     * the positives and negatives on the negatives, in series order.
     *
     * Computed here rather than by layerchart's `stackDiverging` for the AREAS.
     * d3's diverging offset parks a ZERO value at the axis ([0, 0]), not on top
     * of the stack beneath it. Bars do not care, since a zero bar is invisible
     * wherever it sits. An area interpolates between buckets, though, so a layer
     * that is zero in March drops from the top of the stack to the axis and
     * back, cutting a triangle through every layer below it. Here a zero is a
     * zero-thickness band at the height of the stack it sits on, which is what a
     * stacked area means.
     */
    function bandsAt(values: readonly number[], negative: readonly boolean[]): [number, number][] {
        let up = 0;
        let down = 0;
        return values.map((v, idx) => {
            // The SERIES' sign, not the point's: a zero in a negative series
            // sits on the negative stack.
            if (negative[idx]) {
                const band: [number, number] = [down, down + v];
                down += v;
                return band;
            }
            const band: [number, number] = [up, up + v];
            up += v;
            return band;
        });
    }

    const negative = $derived(series.map((s) => s.values.some((v) => v < 0)));
    const rows = $derived<Row[]>(
        labels.map((_, i) => {
            const v = series.map((s) => s.values[i] ?? 0);
            return {i, v, band: bandsAt(v, negative), net: net[i] ?? 0};
        })
    );

    const emptyReason = $derived(stackEmptyReason(labels, series, net, minBuckets));

    // Seeded at [0, 0] so zero is always in frame; widened by each bucket's
    // stacked extremes and by the net.
    const yDomain = $derived.by<[number, number]>(() => {
        let lo = 0;
        let hi = 0;
        for (const r of rows) {
            const up = r.v.reduce((a, v) => a + Math.max(0, v), 0);
            const down = r.v.reduce((a, v) => a + Math.min(0, v), 0);
            lo = Math.min(lo, down, r.net);
            hi = Math.max(hi, up, r.net);
        }
        return [lo, hi];
    });

    const xTicks = $derived(tickIndices(rows.length));
    const labelOf = $derived(labelFormatter(labels));

    let width = $state(0);
    const barWidth = $derived.by(() => {
        if (mark !== "bar" || width <= 0 || rows.length === 0) return 0;
        const plot = Math.max(0, width - PAD_LEFT - PAD_RIGHT);
        const band = (plot / rows.length) * (1 - BAND_PADDING);
        return Math.max(1, Math.min(MAX_BAR_WIDTH, Math.floor(band)));
    });

    // Layerchart keys are the series INDEX (the row holds one value per series,
    // index-aligned). The caller's key — an account name, a split suffix — still
    // names the legend entry and the tooltip row; it is never handed to the chart.
    const chartSeries = $derived(series.map((s, idx) => ({key: `s${idx}`, label: s.label, color: s.color, value: (d: Row) => d.v[idx] ?? 0})));

    const entityOf = (s: Series): string => s.entity ?? s.key;

    /** One entry per entity, in series order; a label+colour seen twice (the two "(other)"s) is one entry. */
    const legend = $derived(
        series
            .filter((s, idx) => series.findIndex((t) => t.label === s.label && t.color === s.color) === idx)
            .map((s) => ({key: entityOf(s), label: s.label, color: s.color}))
    );

    /** The tooltip rows for bucket `i`: each entity's value (its halves summed), non-zero only, legend order. */
    function itemsAt(i: number): {key: string; label: string; color: string; value: number}[] {
        return series
            .filter((s, idx) => series.findIndex((t) => entityOf(t) === entityOf(s)) === idx)
            .map((s) => ({
                key: entityOf(s),
                label: s.label,
                color: s.color,
                value: series.filter((t) => entityOf(t) === entityOf(s)).reduce((sum, t) => sum + (t.values[i] ?? 0), 0),
            }))
            .filter((item) => item.value !== 0);
    }

    const netOf = (d: Row): number => d.net;
    /**
     * Px one compact net label needs (`$211.9K` at 11px, plus air). The caller's
     * `netLabelAt` assumes a phone-width budget; this MEASURES instead. Labels sit
     * one per bucket, so they fit only when a BUCKET is at least this wide; when
     * it is not, only the last is kept, since overlapping labels are unreadable
     * and the latest figure is the one worth keeping. The tooltip still carries
     * every value.
     */
    const NET_LABEL_PX = 64;
    const labelled = $derived.by(() => {
        // A bucket with nothing in it (the years before the journal starts)
        // gets no "$0" label: it would be a number that says nothing.
        const wanted = netLabelAt.filter((i) => {
            const row = rows[i];
            return row !== undefined && (row.net !== 0 || row.v.some((v) => v !== 0));
        });
        const bucketPx = width <= 0 || rows.length === 0 ? Infinity : (width - PAD_LEFT - PAD_RIGHT) / rows.length;
        return new Set(wanted.length <= 1 || bucketPx >= NET_LABEL_PX ? wanted : wanted.slice(-1));
    });

    const NET_CLASS = "stroke-2 [stroke-dasharray:5_3]";
    const NET_HALO_CLASS = "stroke-[6px] [stroke-dasharray:5_3]";

    type Scale = ((value: number) => number) & {bandwidth?: () => number};
</script>

{#snippet tooltip()}
    <Tooltip.Root contained="window">
        {#snippet children({data})}
            {@const row = data as Row}
            <div class="flex max-w-[min(15rem,60vw)] min-w-36 flex-col gap-1 text-xs" data-testid={testid === undefined ? undefined : `${testid}-tooltip`}>
                <div class="font-semibold">{labelOf(row.i)}</div>
                {#each itemsAt(row.i) as item (item.key)}
                    <div class="flex items-center gap-2">
                        <span class="inline-block w-3 shrink-0 border-t-2" style="border-color:{item.color}"></span>
                        <span class="grow truncate text-base-content/60">{item.label}</span>
                        <span class="font-semibold tabular-nums">{formatValue(item.value)}</span>
                    </div>
                {/each}
                <div class="mt-0.5 flex items-center gap-2 border-t border-base-content/10 pt-1">
                    <span class="inline-block w-3 shrink-0 border-t-2 border-dashed" style="border-color:{chartColors.flowNet}"></span>
                    <span class="grow text-base-content/60">{netLabel}</span>
                    <span class="font-semibold tabular-nums">{formatValue(row.net)}</span>
                </div>
            </div>
        {/snippet}
    </Tooltip.Root>
{/snippet}

{#snippet areas()}
    <!-- Stacked by `bandsAt`, not by layerchart: see it for why. -->
    {#each series as s, idx (s.key)}
        <Area
            data={rows}
            x={(d: Row) => d.i}
            y0={(d: Row) => d.band[idx][0]}
            y1={(d: Row) => d.band[idx][1]}
            fill={s.color}
            fillOpacity={AREA_OPACITY}
            class="stroke-base-200"
            strokeWidth={1}
        />
    {/each}
{/snippet}

{#snippet netLine({context}: {context: {xScale: Scale; yScale: Scale}})}
    <!-- Halo first, then the ink, so the net reads against the surface. -->
    <Spline y={netOf} class="{NET_HALO_CLASS} stroke-base-200" />
    <Spline y={netOf} stroke={chartColors.flowNet} class={NET_CLASS} />
    {#each rows.filter((r) => labelled.has(r.i)) as r (r.i)}
        {@const cx = context.xScale(r.i) + (context.xScale.bandwidth?.() ?? 0) / 2}
        {@const cy = context.yScale(r.net)}
        {@const edge = mark === "area" && rows.length > 1 ? (r.i === 0 ? "start" : r.i === rows.length - 1 ? "end" : "middle") : "middle"}
        <g class="lc-net-point" data-bucket={r.i}>
            <circle {cx} {cy} r="4" fill={chartColors.flowNet} class="stroke-base-200" stroke-width="2" />
            <text
                x={cx}
                y={r.net >= 0 ? cy - 9 : cy + 16}
                text-anchor={edge}
                class="fill-base-content stroke-base-200 [stroke-width:3px] text-[11px] font-semibold tabular-nums [paint-order:stroke]"
                >{axisFormat(r.net)}</text
            >
        </g>
    {/each}
{/snippet}

<div class="w-full">
    {#if heading !== undefined}
        <h3 class="mb-1 text-xs font-semibold tracking-tight text-base-content/70">
            {heading}
            {#if note !== undefined}<span class="font-normal text-base-content/40">· {note}</span>{/if}
        </h3>
    {/if}
    {#if emptyReason !== null}
        <p class="py-8 text-center text-sm text-base-content/60" data-testid={testid === undefined ? undefined : `${testid}-empty`}>
            {emptyReason === "no-data" ? empty : tooShort}
        </p>
    {:else}
        <div class="w-full {height}" bind:clientWidth={width} data-testid={testid}>
            {#if mark === "bar"}
                <BarChart
                    data={rows}
                    x={(d) => d.i}
                    series={chartSeries}
                    {yDomain}
                    yNice
                    seriesLayout="stackDiverging"
                    bandPadding={BAND_PADDING}
                    stackPadding={stackGap}
                    brush={false}
                    padding={{top: 16, right: PAD_RIGHT, bottom: 24, left: PAD_LEFT}}
                    props={{
                        xAxis: {format: labelOf, ticks: xTicks},
                        yAxis: {format: axisFormat},
                        // `strokeWidth: 0` undoes BarChart's 1px black outline on
                        // every bar — a border around a mark is ink that is not data.
                        bars: {stroke: "none", strokeWidth: 0, ...(barWidth > 0 ? {width: barWidth} : {})},
                    }}
                    {tooltip}
                    aboveMarks={netLine}
                />
            {:else}
                <AreaChart
                    data={rows}
                    x={(d) => d.i}
                    series={chartSeries}
                    {yDomain}
                    brush={false}
                    highlight={{lines: true, points: false}}
                    marks={areas}
                    padding={{top: 16, right: PAD_RIGHT, bottom: 24, left: PAD_LEFT}}
                    props={{
                        xAxis: {format: labelOf, ticks: xTicks},
                        yAxis: {format: axisFormat},
                    }}
                    {tooltip}
                    aboveMarks={netLine}
                />
            {/if}
        </div>
        <ul class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-base-content/70" data-testid={testid === undefined ? undefined : `${testid}-legend`}>
            {#each legend as entry (entry.key)}
                <li class="flex items-center gap-1">
                    <span class="inline-block h-2 w-2 shrink-0 rounded-xs" style="background:{entry.color}"></span>
                    {entry.label}
                </li>
            {/each}
            <li class="flex items-center gap-1">
                <span class="inline-block w-4 shrink-0 border-t-2 border-dashed" style="border-color:{chartColors.flowNet}"></span>
                {netLabel}
            </li>
        </ul>
    {/if}
</div>
