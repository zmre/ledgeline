<!-- The chart above the Cash Flow table: stacked areas per bucket, money into
     cash above zero and out of it below, with the net change as a dashed line.

     It draws exactly the table below it: the same displayed rows, under the same
     labels, and the table's Net row as the net line (`cashFlowStack`). A parent
     row is its children stacked, plus its own postings when it has any, so every
     figure in the table is a segment or a sum of segments here. The table is the
     chart's accessible twin. -->
<script lang="ts">
    import type {AmountStyle} from "$lib/domain/types";
    import {settings} from "$lib/stores/settings.svelte";
    import type {PeriodReport} from "../types";
    import {cashFlowStack} from "./cashFlowStack";
    import ChartPanel from "./ChartPanel.svelte";
    import PeriodStackView from "./PeriodStackView.svelte";

    let {report, styles}: {report: PeriodReport; styles: ReadonlyMap<string, AmountStyle>} = $props();
</script>

<ChartPanel
    heading="Cash flow over time"
    note="into cash above zero, out below"
    open={settings.cashFlowChartOpen}
    onToggle={(next) => (settings.cashFlowChartOpen = next)}
    testid="cashflow-chart-panel"
>
    <PeriodStackView {report} model={cashFlowStack} {styles} mark="area" netLabel="Net change" empty="No cash moved in this range." testid="cashflow-chart" />
</ChartPanel>
