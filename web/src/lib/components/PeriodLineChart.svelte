<!-- One or more numeric series over shared period buckets, as lines.
     Generalised from `holdings/ui/HoldingsTrend.svelte`, which now renders
     through it; the conventions below were its, and are restated here because
     this is the file a new chart will be copied from.

     - X IS THE BUCKET INDEX, NOT A DATE, and the labels are strings the caller
       already formatted, printed by the axis formatter. A month is 28 to 31
       days and a quarter is 90 to 92, so a time scale spaces the buckets
       unevenly and invites a reader to compare widths that mean nothing.
     - EXPLICIT INTEGER `xTicks`, about six of them (`periodAxis.tickIndices`).
       A continuous scale over 0..n-1 otherwise puts a tick at 2.5 and labels it
       with whichever bucket rounds to it.
     - `points` ONLY AT 31 BUCKETS OR FEWER. Past that the markers touch and the
       line disappears inside its own dots.
     - ONE SERIES GETS NO LEGEND — `heading` names it, and a box with a single
       swatch only restates the heading (dataviz single-series rule). Two or
       more always get one: identity is never colour-alone.
     - COLOURS ARE `colorAt(i)` FROM THE SHARED PALETTE ($lib/format/palette),
       which documents its validator run. Slots are taken in series order and
       never cycled.

     - THE ZERO RULE AND THE MARKER ARE OPT-IN, AND SOLID. `includeZero` seeds
       the y-domain at zero so the baseline is always in frame (the rule
       `PeriodFlowChart` states for its bars, which a line chart of a BALANCE
       needs for the same reason — a cash line floating above an invisible zero
       is the one thing a runway reader must not be shown). `markAt` puts a
       vertical rule on one bucket. Both are hairlines, never dashed: the
       dataviz rule against dashing is about exactly these marks. Neither is a
       channel — the sentence above the chart says what they mean.

     The legend is this component's own markup rather than layerchart's built-in
     one, for the reason `reports/ui/SankeyPanel.svelte` gives: it is then
     always visible, at every width, and it survives a container that has not
     been measured yet. -->
<script lang="ts" module>
    /** One line. `values` is index-aligned to the chart's `labels`. */
    export interface PeriodSeries {
        /** Legend and tooltip name. */
        name: string;
        /**
         * One number per bucket, zero-filled rather than sparse — a gap and a
         * zero are different claims and only the caller knows which it has.
         */
        values: readonly number[];
        /** Draw dashed: for a projection, a target, or any derived line beside actuals. */
        dashed?: boolean;
    }
</script>

<script lang="ts">
    import {LineChart, Rule} from "layerchart";
    import {colorAt, FLOW_OUT} from "$lib/format/palette";
    import {labelFormatter, tickIndices} from "./periodAxis";

    let {
        heading,
        note,
        labels,
        series,
        formatValue,
        formatAxis,
        includeZero = false,
        markAt = null,
        empty = "Nothing to chart in this range.",
        height = "h-56 sm:h-64",
        testid,
    }: {
        /** Names what is plotted. A single series relies on this instead of a legend. */
        heading: string;
        /** Muted qualifier beside the heading, rendered after a separating "·". */
        note?: string;
        /** One label per bucket; its length is the bucket count. */
        labels: readonly string[];
        series: readonly PeriodSeries[];
        /** Full precision, for the tooltip. */
        formatValue: (n: number) => string;
        /** Compact, for the y-axis ticks ("$1.2K") so they stay short and do not clip. Defaults to `formatValue`. */
        formatAxis?: (n: number) => string;
        /** Keep zero in frame and draw a rule on it. For a BALANCE, where "above or below zero" is the reading. */
        includeZero?: boolean;
        /** Bucket index to mark with a vertical rule, or null. What it means is the caller's to say in words. */
        markAt?: number | null;
        /** Shown instead of the plot when there is nothing to draw. */
        empty?: string;
        /** Height utilities for the plot box. */
        height?: string;
        testid?: string;
    } = $props();

    const axisFormat = $derived(formatAxis ?? formatValue);

    /** One row per bucket; `v[k]` is series `k`'s value there. */
    interface Row {
        i: number;
        v: number[];
    }
    const rows = $derived<Row[]>(labels.map((_, i) => ({i, v: series.map((s) => s.values[i] ?? 0)})));

    // A plot of nothing but zeroes is a flat line on the axis that says less
    // than a sentence does, and an empty bucket list draws nothing at all.
    const nothingToDraw = $derived(rows.length === 0 || rows.every((r) => r.v.every((n) => n === 0)));

    const xTicks = $derived(tickIndices(rows.length));
    const labelOf = $derived(labelFormatter(labels));

    /**
     * The y-domain, seeded at [0, 0] when `includeZero` — the rule
     * `PeriodFlowChart` states for its bars. `undefined` otherwise, which leaves
     * layerchart's own domain exactly as it was.
     */
    const yDomain = $derived.by<[number, number] | undefined>(() => {
        if (!includeZero) return undefined;
        let lo = 0;
        let hi = 0;
        for (const r of rows) {
            for (const n of r.v) {
                lo = Math.min(lo, n);
                hi = Math.max(hi, n);
            }
        }
        return [lo, hi];
    });

    /** Only a marker that lands on a real bucket is drawn. */
    const marker = $derived(markAt !== null && Number.isInteger(markAt) && markAt >= 0 && markAt < rows.length ? markAt : null);

    const LINE_CLASS = "stroke-2";
    /** Overrides `props.spline` for the one series, since a series' own props are spread last. */
    const DASHED_CLASS = "stroke-2 [stroke-dasharray:4_3]";

    const chartSeries = $derived(
        series.map((s, k) => ({
            // Keyed by slot, not by name: two series may legitimately share a
            // label (the same metric under two scenarios) and a duplicate key
            // would silently collapse them into one line.
            key: String(k),
            label: s.name,
            color: colorAt(k),
            value: (d: Row) => d.v[k] ?? 0,
            ...(s.dashed === true ? {props: {class: DASHED_CLASS}} : {}),
        }))
    );
</script>

<div class="w-full">
    <h3 class="mb-1 text-xs font-semibold tracking-tight text-base-content/70">
        {heading}
        {#if note !== undefined}<span class="font-normal text-base-content/40">· {note}</span>{/if}
    </h3>
    {#if nothingToDraw}
        <p class="py-8 text-center text-sm text-base-content/60">{empty}</p>
    {:else}
        <div class="w-full {height}" data-testid={testid}>
            <LineChart
                data={rows}
                x={(d) => d.i}
                series={chartSeries}
                points={rows.length <= 31}
                brush={false}
                {yDomain}
                yNice={includeZero}
                padding={{top: 8, right: 8, bottom: 24, left: 56}}
                props={{
                    xAxis: {format: labelOf, ticks: xTicks},
                    yAxis: {format: axisFormat},
                    spline: {class: LINE_CLASS},
                    tooltip: {header: {format: labelOf}, item: {format: formatValue}},
                }}
            >
                {#snippet belowMarks()}
                    <!-- Under the marks, so the data is never read through a rule.
                         The `data-rule` attributes are passed straight through to
                         the SVG `<line>`: layerchart gives its own baseline the
                         same `lc-rule-y-line` class these would carry, so there
                         is otherwise no way — in a test or in the inspector — to
                         tell our zero rule from its axis. -->
                    {#if includeZero}
                        <Rule y={0} class="stroke-base-content/40" data-rule="zero" />
                    {/if}
                    {#if marker !== null}
                        <Rule x={marker} stroke={FLOW_OUT} data-rule="mark" />
                    {/if}
                {/snippet}
            </LineChart>
        </div>
        <!-- Two or more series: always visible, so identity is never colour-alone.
             A short line-key rather than a dot, because it can also carry the dash. -->
        {#if series.length > 1}
            <ul class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-base-content/70" data-testid={testid === undefined ? undefined : `${testid}-legend`}>
                {#each series as s, k (k)}
                    <li class="flex items-center gap-1">
                        <span class="inline-block w-4 shrink-0 border-t-2 {s.dashed === true ? 'border-dashed' : ''}" style="border-color:{colorAt(k)}"></span>
                        {s.name}
                    </li>
                {/each}
            </ul>
        {/if}
    {/if}
</div>
