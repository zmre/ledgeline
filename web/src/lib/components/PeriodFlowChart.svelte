<!-- Money in as bars above a zero rule, money out as bars below it, and the net
     as a line over the top, over shared period buckets.

     THE FIRST BAR CHART IN THIS CODEBASE, so it sets the idiom; the specs below
     are the dataviz skill's, not inventions, and a second bar chart should copy
     them rather than re-derive them.

     BUILT FOR TWO CALLERS, ON PURPOSE. Plan 22's Net income tab is the first,
     and the second is the Cash Flow report, where the TODO asks for this exact
     picture — an inflow/outflow graph "with a dotted line over the top showing
     the Net" (`plans/22-projections.md`, Phase 2). Hence props of plain
     index-aligned number arrays plus labels, and nothing about projections,
     scenarios or `PeriodReport` anywhere in them: a report that can produce
     three columns of numbers can render this. It is NOT wired into the Cash
     Flow report here.

     The specs:

     - X IS THE BUCKET INDEX with explicit integer `xTicks`, exactly as
       `PeriodLineChart` — see `periodAxis.ts` for why.
     - THE ZERO RULE IS ALWAYS IN FRAME. `yDomain` is seeded at [0, 0] before
       the data widens it, so the baseline every bar grows from is always drawn
       and a reader is never shown bars floating above an invisible zero. It
       also covers `net`, which the caller may supply independently, so an
       out-of-band net line is scaled rather than clipped away.
     - INFLOWS AND OUTFLOWS ARE MAGNITUDES; THE CHART OWNS THE SIGN. Callers
       disagree about which way an expense points — the engine's reports carry
       one convention and a projection another — and a chart that renders the
       outflow bar upward because the caller's sign was the other one is worse
       than a chart that will not take the sign at all.
     - BARS ARE CAPPED AT 24px AND CENTRED IN THEIR BAND, which needs the
       container's real width, so the width is measured rather than assumed
       (the rule `SankeyPanel.svelte` states). Six buckets across a desktop card
       is a 90px band, and a 90px bar reads as a block of colour rather than a
       measurement.
     - 4px ROUNDED DATA-END, SQUARE AT THE BASELINE, and a band padding that
       leaves the bars well clear of each other; layerchart's defaults, named
       here because they are load-bearing rather than incidental.
     - RED/GREEN IS A DIVERGING PAIR, NOT TWO CATEGORICAL SLOTS, and it is not
       `--color-success` / `--color-error` either — $lib/format/palette explains
       why at length and carries the validator run. Colour is reinforcement
       here, never the channel: which side of the zero rule a bar is on already
       says the direction.
     - THE NET LINE IS DRAWN TWICE, the lower copy wider and in the surface
       colour. It crosses the bars constantly and its ink is only 2.62:1 against
       the green; the halo means it is always read against the surface. Dashed
       because it is derived from the other two marks rather than measured
       beside them — and because the ask asked for it. (The dataviz rule against
       dashing is about gridlines and axes, which stay solid hairlines here.) -->
<script lang="ts">
    import {BarChart, Spline} from "layerchart";
    import {FLOW_IN, FLOW_NET, FLOW_OUT} from "$lib/format/palette";
    import {labelFormatter, tickIndices} from "./periodAxis";

    let {
        heading,
        note,
        labels,
        inflows,
        outflows,
        net,
        inflowLabel = "Money in",
        outflowLabel = "Money out",
        netLabel = "Net",
        formatValue,
        formatAxis,
        empty = "Nothing to chart in this range.",
        height = "h-56 sm:h-64",
        testid,
    }: {
        /** Names what is plotted. */
        heading: string;
        /** Muted qualifier beside the heading, rendered after a separating "·". */
        note?: string;
        /** One label per bucket; its length is the bucket count. */
        labels: readonly string[];
        /** Money in per bucket, as a MAGNITUDE. Drawn above the zero rule. */
        inflows: readonly number[];
        /** Money out per bucket, as a MAGNITUDE. Drawn below the zero rule. */
        outflows: readonly number[];
        /**
         * The net per bucket, signed. Defaults to `inflow − outflow`.
         *
         * Supply it when the caller has an authoritative net that is not just
         * the difference of these two columns — a report whose net includes
         * lines that are in neither, say. Nothing reconciles the two: the
         * number given is the number drawn.
         */
        net?: readonly number[];
        inflowLabel?: string;
        outflowLabel?: string;
        netLabel?: string;
        /** Full precision, for the tooltip. */
        formatValue: (n: number) => string;
        /** Compact, for the y-axis ticks ("$1.2K") so they stay short and do not clip. Defaults to `formatValue`. */
        formatAxis?: (n: number) => string;
        /** Shown instead of the plot when there is nothing to draw. */
        empty?: string;
        /** Height utilities for the plot box. */
        height?: string;
        testid?: string;
    } = $props();

    const axisFormat = $derived(formatAxis ?? formatValue);

    /** Plot insets, in px. Repeated as numbers below because the bar cap does arithmetic on them. */
    const PAD_LEFT = 56;
    const PAD_RIGHT = 8;
    /** Fraction of each band left as air. layerchart's default, restated because the bar cap needs it. */
    const BAND_PADDING = 0.4;
    /** dataviz mark spec: a bar never exceeds this, however much room its band has. */
    const MAX_BAR_WIDTH = 24;

    interface Row {
        i: number;
        inflow: number;
        outflow: number;
        net: number;
    }
    const rows = $derived<Row[]>(
        labels.map((_, i) => {
            const inflow = Math.abs(inflows[i] ?? 0);
            const outflow = -Math.abs(outflows[i] ?? 0);
            return {i, inflow, outflow, net: net?.[i] ?? inflow + outflow};
        })
    );

    const nothingToDraw = $derived(rows.length === 0 || rows.every((r) => r.inflow === 0 && r.outflow === 0 && r.net === 0));

    // Seeded at [0, 0] so zero is in the domain whatever the data does, and
    // widened by `net` as well as the bars so a caller-supplied net is scaled
    // rather than clipped.
    const yDomain = $derived.by<[number, number]>(() => {
        let lo = 0;
        let hi = 0;
        for (const r of rows) {
            lo = Math.min(lo, r.outflow, r.net);
            hi = Math.max(hi, r.inflow, r.net);
        }
        return [lo, hi];
    });

    const xTicks = $derived(tickIndices(rows.length));
    const labelOf = $derived(labelFormatter(labels));

    /** The container's measured width; 0 until it has been laid out. */
    let width = $state(0);

    // The natural band, capped. 0 means "not measured yet" and the bars keep
    // layerchart's own band-filling width until a real number arrives.
    const barWidth = $derived.by(() => {
        if (width <= 0 || rows.length === 0) return 0;
        const plot = Math.max(0, width - PAD_LEFT - PAD_RIGHT);
        const band = (plot / rows.length) * (1 - BAND_PADDING);
        return Math.max(1, Math.min(MAX_BAR_WIDTH, Math.floor(band)));
    });

    const chartSeries = $derived([
        {key: "in", label: inflowLabel, color: FLOW_IN, value: (d: Row) => d.inflow},
        {key: "out", label: outflowLabel, color: FLOW_OUT, value: (d: Row) => d.outflow},
    ]);

    const netOf = (d: Row): number => d.net;

    const NET_CLASS = "stroke-2 [stroke-dasharray:5_3]";
    const NET_HALO_CLASS = "stroke-[6px] [stroke-dasharray:5_3]";
</script>

<div class="w-full">
    <h3 class="mb-1 text-xs font-semibold tracking-tight text-base-content/70">
        {heading}
        {#if note !== undefined}<span class="font-normal text-base-content/40">· {note}</span>{/if}
    </h3>
    {#if nothingToDraw}
        <p class="py-8 text-center text-sm text-base-content/60">{empty}</p>
    {:else}
        <div class="w-full {height}" bind:clientWidth={width} data-testid={testid}>
            <BarChart
                data={rows}
                x={(d) => d.i}
                series={chartSeries}
                {yDomain}
                yNice
                seriesLayout="stackDiverging"
                bandPadding={BAND_PADDING}
                brush={false}
                padding={{top: 8, right: PAD_RIGHT, bottom: 24, left: PAD_LEFT}}
                props={{
                    xAxis: {format: labelOf, ticks: xTicks},
                    yAxis: {format: axisFormat},
                    // `strokeWidth: 0` UNDOES A LAYERCHART DEFAULT, and it matters:
                    // `BarChart` outlines every bar with a 1px black stroke, which is
                    // the dataviz anti-pattern verbatim — a border drawn around a mark
                    // to separate it, adding data-weight ink that is not data. The band
                    // padding separates neighbours and the zero rule separates the two
                    // halves of a column; neither needs help.
                    bars: {stroke: "none", strokeWidth: 0, ...(barWidth > 0 ? {width: barWidth} : {})},
                    tooltip: {header: {format: labelOf}, item: {format: formatValue}},
                }}
            >
                {#snippet aboveMarks()}
                    <!-- Halo first, then the line, so the net is read against the surface
                         rather than against whichever bar it happens to cross. -->
                    <Spline y={netOf} class="{NET_HALO_CLASS} stroke-base-200" />
                    <Spline y={netOf} stroke={FLOW_NET} class={NET_CLASS} />
                {/snippet}
            </BarChart>
        </div>
        <!-- Always visible: three marks, so identity is never colour-alone. -->
        <ul class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-base-content/70" data-testid={testid === undefined ? undefined : `${testid}-legend`}>
            <li class="flex items-center gap-1">
                <span class="inline-block h-2 w-2 shrink-0 rounded-xs" style="background:{FLOW_IN}"></span>
                {inflowLabel}
            </li>
            <li class="flex items-center gap-1">
                <span class="inline-block h-2 w-2 shrink-0 rounded-xs" style="background:{FLOW_OUT}"></span>
                {outflowLabel}
            </li>
            <li class="flex items-center gap-1">
                <span class="inline-block w-4 shrink-0 border-t-2 border-dashed" style="border-color:{FLOW_NET}"></span>
                {netLabel}
            </li>
        </ul>
    {/if}
</div>
