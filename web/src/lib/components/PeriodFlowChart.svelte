<!-- Money in as bars above a zero rule, money out as bars below it, and the net
     as a line over the top, over shared period buckets.

     The first bar chart in this codebase, and the idiom the others copy. The
     drawing now lives in `PeriodStackChart`, which generalises exactly this —
     two series stacked away from zero, a haloed dashed net — to any number of
     signed series; the Net Worth and Cash Flow report charts are the same
     component with more of them. What stays here is what makes THIS chart
     this chart:

     - INFLOWS AND OUTFLOWS ARE MAGNITUDES; THE CHART OWNS THE SIGN. Callers
       disagree about which way an expense points — the engine's reports carry
       one convention and a projection another — and a chart that renders the
       outflow bar upward because the caller's sign was the other one is worse
       than a chart that will not take the sign at all.
     - `net` DEFAULTS TO inflow − outflow, and a supplied one is drawn as given.
     - RED/GREEN IS A DIVERGING PAIR, NOT TWO CATEGORICAL SLOTS: the theme's
       `--chart-in` / `--chart-out` tokens via `chartColors`. `in` is daisy
       success; `out` is a deeper rose than daisy error, which collapses into
       success under deuteranopia — $lib/format/palette carries the numbers.
       Colour is reinforcement here, never the channel: which side of the zero
       rule a bar is on already says the direction.

     The bar cap, zero rule, rounded data-ends, stroke-less bars, the net's halo
     and the legend are specified in `PeriodStackChart`'s header. -->
<script lang="ts">
    import {chartColors} from "$lib/format/chartColors.svelte";
    import PeriodStackChart from "./PeriodStackChart.svelte";

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

    const ins = $derived(labels.map((_, i) => Math.abs(inflows[i] ?? 0)));
    const outs = $derived(labels.map((_, i) => -Math.abs(outflows[i] ?? 0)));
    const nets = $derived(labels.map((_, i) => net?.[i] ?? ins[i] + outs[i]));

    const series = $derived([
        {key: "in", label: inflowLabel, color: chartColors.flowIn, values: ins},
        {key: "out", label: outflowLabel, color: chartColors.flowOut, values: outs},
    ]);
</script>

<PeriodStackChart {heading} {note} {labels} {series} net={nets} {netLabel} {formatValue} {formatAxis} {empty} {height} {testid} />
