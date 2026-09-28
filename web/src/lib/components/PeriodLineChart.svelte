<!-- One or more numeric series over shared period buckets, as lines.
     Generalised from `holdings/ui/HoldingsTrend.svelte`, which now renders
     through it; the conventions below were its, and are restated here because
     this is the file a new chart will be copied from.

     - X IS THE BUCKET INDEX, NOT A DATE, and the labels are strings the caller
       already formatted, printed by the axis formatter. A month is 28 to 31
       days and a quarter is 90 to 92, so a time scale spaces the buckets
       unevenly and invites a reader to compare widths that mean nothing.
     - EXPLICIT INTEGER x ticks, at most about six, fewer when the plot's
       width cannot fit the labels apart (`periodAxis.fittedTicks`).
       A continuous scale over 0..n-1 otherwise puts a tick at 2.5 and labels it
       with whichever bucket rounds to it.
     - `points` ONLY AT 31 BUCKETS OR FEWER. Past that the markers touch and the
       line disappears inside its own dots.
     - ONE SERIES GETS NO LEGEND — `heading` names it, and a box with a single
       swatch only restates the heading (dataviz single-series rule). Two or
       more always get one: identity is never colour-alone.
     - COLOURS ARE `chartColors.colorAt(i)`, THE THEME'S `--chart-N` TOKENS
       ($lib/format/palette documents the validator run). Slots are taken in series order and
       never cycled — unless a series names its own `slot`, which is how an
       optional line (a benchmark the reader can tick on and off) keeps ONE
       colour whatever else is shown: colour follows the entity, not its row.
     - `null` IS A GAP. The line breaks there and no marker is drawn, and the
       tooltip reads "—". A missing value is never drawn as zero.

     - THE ZERO RULE AND THE MARKER ARE OPT-IN, AND SOLID. `includeZero` seeds
       the y-domain at zero so the baseline is always in frame (the rule
       `PeriodFlowChart` states for its bars, which a line chart of a BALANCE
       needs for the same reason — a cash line floating above an invisible zero
       is the one thing a runway reader must not be shown). `markAt` puts a
       vertical rule on one bucket. Both are hairlines, never dashed: the
       dataviz rule against dashing is about exactly these marks. Neither is a
       channel — the sentence above the chart says what they mean.

     The legend is `ChartLegend`, not layerchart's built-in one — see there. -->
<script lang="ts" module>
    // Declared in a plain .ts file so .ts modules can import it too (tsc cannot
    // see a type exported from a .svelte module); re-exported here for callers.
    import type {PeriodSeries as Series} from "./periodSeries";
    export type PeriodSeries = Series;
</script>

<script lang="ts">
    import type {Snippet} from "svelte";
    import {LineChart, Rule} from "layerchart";
    import {chartColors} from "$lib/format/chartColors.svelte";
    import ChartLegend from "./ChartLegend.svelte";
    import {fittedTicks, labelFormatter} from "./periodAxis";

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
        actions,
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
        /** Controls that change what the chart shows (toggles, pickers), set in the heading row. */
        actions?: Snippet;
    } = $props();

    const axisFormat = $derived(formatAxis ?? formatValue);

    /** One row per bucket; `v[k]` is series `k`'s value there, `null` for a gap. */
    interface Row {
        i: number;
        v: (number | null)[];
    }
    const rows = $derived<Row[]>(labels.map((_, i) => ({i, v: series.map((s) => (i < s.values.length ? (s.values[i] ?? null) : 0))})));

    // A plot of nothing but zeroes (and gaps) is a flat line on the axis that
    // says less than a sentence does, and an empty bucket list draws nothing.
    const nothingToDraw = $derived(rows.length === 0 || rows.every((r) => r.v.every((n) => n === null || n === 0)));

    /** The tooltip's formatter: a gap reads as an em-dash, never as "$0.00" or "NaN". */
    const tooltipFormat = $derived((n: number | null | undefined): string => (typeof n === "number" && Number.isFinite(n) ? formatValue(n) : "—"));

    const xTicks = $derived(fittedTicks(labels));
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
                if (n === null) continue;
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
            color: chartColors.colorAt(s.slot ?? k),
            // `null` (not 0) where the series has a gap: layerchart's default
            // `defined` breaks the line there and its points skip the marker.
            value: (d: Row) => d.v[k] ?? null,
            ...(s.dashed === true ? {props: {class: DASHED_CLASS}} : {}),
        }))
    );
</script>

<div class="w-full">
    <div class="mb-1 flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <h3 class="text-xs font-semibold tracking-tight text-base-content/70">
            {heading}
            {#if note !== undefined}<span class="font-normal text-base-content/40">· {note}</span>{/if}
        </h3>
        {#if actions !== undefined}{@render actions()}{/if}
    </div>
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
                    tooltip: {header: {format: labelOf}, item: {format: tooltipFormat}},
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
                        <Rule x={marker} stroke={chartColors.flowOut} data-rule="mark" />
                    {/if}
                {/snippet}
            </LineChart>
        </div>
        <!-- Two or more series: always visible, so identity is never colour-alone.
             A short line-key rather than a dot, because it can also carry the dash. -->
        {#if series.length > 1}
            <ChartLegend
                entries={series.map((s, k) => ({key: String(k), label: s.name, color: chartSeries[k].color, swatch: "line", dash: s.dashed === true}))}
                class="mt-1"
                testid={testid === undefined ? undefined : `${testid}-legend`}
            />
        {/if}
    {/if}
</div>
