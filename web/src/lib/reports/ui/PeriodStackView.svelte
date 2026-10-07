<!-- One `PeriodReport` drawn as a stacked diverging period chart: the model
     (`periodStack`, the table's displayed rows), the commodity's display style,
     and the "charted in X only" note, joined to `PeriodStackChart`. Shared by
     the Net Worth and Cash Flow panels so neither restates the commodity pick
     or the formatting. -->
<script lang="ts">
    import PeriodStackChart from "$lib/components/PeriodStackChart.svelte";
    import type {AmountStyle} from "$lib/domain/types";
    import {styleOf} from "$lib/format/amounts";
    import {chartColors} from "$lib/format/chartColors.svelte";
    import {formatChartValue, formatCompactChartValue} from "$lib/insights/series";
    import type {PeriodReport} from "../types";
    import {periodStack} from "./periodStack";

    let {
        report,
        styles,
        mark,
        netLabel,
        labelNet = false,
        empty,
        testid,
    }: {
        report: PeriodReport;
        styles: ReadonlyMap<string, AmountStyle>;
        mark: "bar" | "area";
        netLabel: string;
        /** Dot and label the net where the chart has room (`PeriodStackChart`); off draws the line alone. */
        labelNet?: boolean;
        empty: string;
        testid: string;
    } = $props();

    const stack = $derived(periodStack(report, chartColors.current, ""));
    const style = $derived<AmountStyle>(styleOf(styles, stack.commodity));
    const format = $derived((n: number) => formatChartValue(n, stack.commodity, style));
    const formatAxis = $derived((n: number) => formatCompactChartValue(n, stack.commodity, style));
</script>

<PeriodStackChart
    labels={stack.labels}
    series={stack.series}
    net={stack.net}
    {netLabel}
    {labelNet}
    {mark}
    stackGap={mark === "bar" ? 2 : 0}
    minBuckets={2}
    formatValue={format}
    {formatAxis}
    {empty}
    {testid}
/>
{#if stack.omitted.length > 0}
    <p class="text-xs text-base-content/60" data-testid="{testid}-omitted">
        Charted in {stack.commodity} only; {stack.omitted.join(", ")} not shown.
    </p>
{/if}
