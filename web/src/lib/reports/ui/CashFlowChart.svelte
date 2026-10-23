<!-- The chart above the statement of cash flows (plans/27): per period, one
     stacked bar per activity (operating, investing, financing, and the effect
     of exchange rates when there is one), cash in above zero and out below,
     with the net change in cash as the line.

     Every segment is a section total in the statement below, and the line is its
     Net change in cash row, so the statement is the chart's accessible twin. -->
<script lang="ts">
    import type {AmountStyle} from "#lib/domain/types.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import type {CashFlowStatement} from "../types";
    import {cashFlowChartReport} from "./cashFlowRows";
    import ChartPanel from "./ChartPanel.svelte";
    import PeriodStackView from "./PeriodStackView.svelte";

    let {report, styles}: {report: CashFlowStatement; styles: ReadonlyMap<string, AmountStyle>} = $props();

    const chart = $derived(cashFlowChartReport(report));
</script>

<ChartPanel
    heading="Cash flow over time"
    note="cash in above zero, out below"
    open={settings.cashFlowChartOpen}
    onToggle={(next) => (settings.cashFlowChartOpen = next)}
    testid="cashflow-chart-panel"
>
    <PeriodStackView report={chart} {styles} mark="bar" netLabel="Net change in cash" labelNet empty="No cash moved in this range." testid="cashflow-chart" />
</ChartPanel>
