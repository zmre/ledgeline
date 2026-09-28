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

     - X IS THE BUCKET INDEX with explicit integer ticks, spaced for the
       measured width so labels never collide — see `periodAxis.ts`.
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
       place on the stack; filled solid, no top line, a 1px hairline of surface
       between layers. The dataviz ~10% wash is for areas that OVERLAP; stacked
       layers never overlap, they tile, and any translucency over the dark
       surface only muddies them. The hairline is what tells neighbours apart,
       and the haloed net line stays legible on top of solid fills.
     - THE NET IS DRAWN TWICE, a wide surface-coloured halo under the dashed
       ink, so it reads against the surface rather than against whichever mark
       it crosses. `labelNet` adds a dot and a value label, off by default and
       never on every point: the buckets are picked by `periodAxis.tickIndices`,
       the axis's own spacing rule, fitted to the plot width and the formatted
       labels. A bucket with nothing in it gets none.
     - ONE TOOLTIP PER BUCKET listing every non-zero series in legend order —
       values lead, labels follow, keyed by a short line — then the net.
     - The LEGEND is always shown, one entry per entity (a folded "(other)" on
       both sides is one entry), with the net as a dashed line key: identity is
       never colour alone. -->
<script lang="ts">
    import {Area, AreaChart, BarChart, Spline, Tooltip, type ChartState} from "layerchart";
    import {chartColors} from "$lib/format/chartColors.svelte";
    import ChartHeading from "./ChartHeading.svelte";
    import ChartLegend, {type LegendEntry} from "./ChartLegend.svelte";
    import {fittedTicks, labelFormatter, tickIndices} from "./periodAxis";
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
        labelNet = false,
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
        /** Dot and label the net at as many buckets as fit apart. */
        labelNet?: boolean;
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

    const PADDING = {top: 16, right: 8, bottom: 24, left: 56};
    const BAND_PADDING = 0.4;
    /** dataviz mark spec: a bar never exceeds this, however much room its band has. */
    const MAX_BAR_WIDTH = 24;

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

    const xTicks = $derived(fittedTicks(labels));
    const labelOf = $derived(labelFormatter(labels));

    /** The bar chart's own state: its band scale is what the bar cap is measured against. */
    let barChart = $state<ChartState<Row>>();
    /** The band's width capped at `MAX_BAR_WIDTH`; 0 (layerchart's own width) before layout. */
    const barWidth = $derived.by(() => {
        const band = (barChart?.xScale as Scale | undefined)?.bandwidth?.() ?? 0;
        return band > 0 ? Math.max(1, Math.min(MAX_BAR_WIDTH, Math.floor(band))) : 0;
    });

    // Layerchart keys are the series INDEX (the row holds one value per series,
    // index-aligned). The caller's key — an account name, a split suffix — still
    // names the legend entry and the tooltip row; it is never handed to the chart.
    const chartSeries = $derived(series.map((s, idx) => ({key: `s${idx}`, label: s.label, color: s.color, value: (d: Row) => d.v[idx] ?? 0})));

    /** Each entity's series (its one or two sign halves), in first-seen order: the legend's order and the tooltip's. */
    const entities = $derived.by(() => {
        // eslint-disable-next-line svelte/prefer-svelte-reactivity -- rebuilt wholesale inside $derived.by, never mutated afterwards
        const byEntity = new Map<string, Series[]>();
        for (const s of series) {
            const key = s.entity ?? s.key;
            byEntity.set(key, [...(byEntity.get(key) ?? []), s]);
        }
        return byEntity;
    });

    /** One entry per entity, then the net; two entities that look alike (the up and down "(other)") are one entry. */
    const legend = $derived.by((): LegendEntry[] => {
        // eslint-disable-next-line svelte/prefer-svelte-reactivity -- local to this derivation
        const seen = new Set<string>();
        const named = [...entities].flatMap(([key, [first]]): LegendEntry[] => {
            const look = `${first.label}\u0000${first.color}`;
            if (seen.has(look)) return [];
            seen.add(look);
            return [{key, label: first.label, color: first.color, swatch: "square"}];
        });
        return [...named, {key: "\u0000net", label: netLabel, color: chartColors.flowNet, swatch: "line", dash: true}];
    });

    /** The tooltip rows for bucket `i`: each entity's value (its halves summed), non-zero only, legend order. */
    function itemsAt(i: number): {key: string; label: string; color: string; value: number}[] {
        return [...entities]
            .map(([key, halves]) => ({key, label: halves[0].label, color: halves[0].color, value: halves.reduce((sum, s) => sum + (s.values[i] ?? 0), 0)}))
            .filter((item) => item.value !== 0);
    }

    const netOf = (d: Row): number => d.net;

    /** The px size the net value labels are drawn at (the `text-[11px]` below). */
    const NET_LABEL_FONT_PX = 11;
    const netTexts = $derived(rows.map((r) => axisFormat(r.net)));
    /**
     * The buckets whose net gets a dot and value label in a plot `plotWidth` px
     * wide: the x axis's own spacing rule over the formatted labels, so they
     * thin out exactly as the ticks do and never touch. The tooltip still
     * carries every value. A bucket with nothing in it (the years before the
     * journal starts) gets no "$0" label: it would be a number that says nothing.
     */
    function netLabelsAt(plotWidth: number): Set<number> {
        if (!labelNet) return new Set();
        const fitted = tickIndices(rows.length, {plotWidth, labels: netTexts, fontPx: NET_LABEL_FONT_PX});
        return new Set(fitted.filter((i) => rows[i].net !== 0 || rows[i].v.some((v) => v !== 0)));
    }

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
            class="stroke-base-200"
            strokeWidth={1}
        />
    {/each}
{/snippet}

{#snippet netLine({context}: {context: {xScale: Scale; yScale: Scale; width: number}})}
    <!-- Halo first, then the ink, so the net reads against the surface. -->
    <Spline y={netOf} class="{NET_HALO_CLASS} stroke-base-200" />
    <Spline y={netOf} stroke={chartColors.flowNet} class={NET_CLASS} />
    {@const labelled = netLabelsAt(context.width)}
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
                class="fill-base-content stroke-base-200 [stroke-width:3px] text-[11px] font-semibold tabular-nums [paint-order:stroke]">{netTexts[r.i]}</text
            >
        </g>
    {/each}
{/snippet}

<div class="w-full">
    {#if heading !== undefined}
        <ChartHeading {heading} {note} class="mb-1" />
    {/if}
    {#if emptyReason !== null}
        <p class="py-8 text-center text-sm text-base-content/60" data-testid={testid === undefined ? undefined : `${testid}-empty`}>
            {emptyReason === "no-data" ? empty : tooShort}
        </p>
    {:else}
        <div class="w-full {height}" data-testid={testid}>
            {#if mark === "bar"}
                <BarChart
                    bind:context={barChart}
                    data={rows}
                    x={(d) => d.i}
                    series={chartSeries}
                    {yDomain}
                    yNice
                    seriesLayout="stackDiverging"
                    bandPadding={BAND_PADDING}
                    stackPadding={stackGap}
                    brush={false}
                    padding={PADDING}
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
                    padding={PADDING}
                    props={{
                        xAxis: {format: labelOf, ticks: xTicks},
                        yAxis: {format: axisFormat},
                    }}
                    {tooltip}
                    aboveMarks={netLine}
                />
            {/if}
        </div>
        <ChartLegend entries={legend} class="mt-1" testid={testid === undefined ? undefined : `${testid}-legend`} />
    {/if}
</div>
