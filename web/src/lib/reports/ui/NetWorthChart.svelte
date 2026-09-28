<!-- The chart above the Net Worth table: one stacked column per bucket, each
     asset account stacked above zero and each liability below it, with the net
     worth as a dashed line and, where few enough to stay legible, its value at
     each point.

     Which side an account stacks on is the ENGINE's call (the row `kind`, by
     effective declared type). Within a bucket each segment is drawn on the side
     its sign puts it: an overdrawn asset is a short asset-coloured segment below
     zero, a liability carrying a refund one above it. -->
<script lang="ts">
    import type {AmountStyle} from "$lib/domain/types";
    import {settings} from "$lib/stores/settings.svelte";
    import type {PeriodReport} from "../types";
    import ChartPanel from "./ChartPanel.svelte";
    import {periodStack} from "./periodStack";
    import PeriodStackView from "./PeriodStackView.svelte";

    let {report, styles}: {report: PeriodReport; styles: ReadonlyMap<string, AmountStyle>} = $props();
</script>

<ChartPanel
    heading="Net worth over time"
    note="assets above zero, liabilities below"
    open={settings.netWorthChartOpen}
    onToggle={(next) => (settings.netWorthChartOpen = next)}
    testid="networth-chart-panel"
>
    <PeriodStackView
        {report}
        model={periodStack}
        {styles}
        mark="bar"
        netLabel="Net worth"
        labelNet
        empty="No assets or liabilities in this range."
        testid="networth-chart"
    />
</ChartPanel>
