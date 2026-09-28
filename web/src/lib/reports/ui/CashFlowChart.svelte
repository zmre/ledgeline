<!-- The chart above the Cash Flow table: stacked areas per bucket, money into
     cash above zero and out of it below, with the net change as a dashed line.

     Two breakdowns of the SAME movement, toggled and persisted:

     - BY SOURCE (default): why cash moved — the counterparty accounts (salary,
       rent, a card payment, a stock purchase) from
       /api/reports/cashflow/sources. A separate request, made only while this
       view is showing, and drawn only when it answers the table's own window.
     - BY ACCOUNT: where it moved — the table's own rows, the cash accounts. A
       transfer between two cash accounts shows here as one layer up and one
       down, and cancels in the net.

     Either way the net line is the same figure, and the table below is the
     accessible twin of both. -->
<script lang="ts">
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import type {AmountStyle} from "$lib/domain/types";
    import type {SourcesPanel} from "$lib/stores/cashFlowSources.svelte";
    import {settings, type CashFlowChartMode} from "$lib/stores/settings.svelte";
    import type {PeriodReport} from "../types";
    import ChartPanel from "./ChartPanel.svelte";
    import PeriodStackView from "./PeriodStackView.svelte";

    let {report, sources, styles}: {report: PeriodReport; sources: SourcesPanel; styles: ReadonlyMap<string, AmountStyle>} = $props();

    const MODES: {mode: CashFlowChartMode; label: string}[] = [
        {mode: "source", label: "By source"},
        {mode: "account", label: "By account"},
    ];
    const mode = $derived(settings.cashFlowChartMode);
</script>

<ChartPanel
    heading="Cash flow over time"
    note="into cash above zero, out below"
    open={settings.cashFlowChartOpen}
    onToggle={(next) => (settings.cashFlowChartOpen = next)}
    testid="cashflow-chart-panel"
>
    <div class="join self-start" role="radiogroup" aria-label="Cash flow breakdown">
        {#each MODES as option (option.mode)}
            <button
                type="button"
                role="radio"
                aria-checked={mode === option.mode}
                class="btn join-item btn-xs {mode === option.mode ? 'btn-active' : ''}"
                data-testid="cashflow-chart-mode-{option.mode}"
                onclick={() => (settings.cashFlowChartMode = option.mode)}>{option.label}</button
            >
        {/each}
    </div>
    {#if mode === "account"}
        <PeriodStackView {report} {styles} mark="area" netLabel="Net change" empty="No cash moved in this range." testid="cashflow-chart" />
    {:else}
        <AsyncSection
            view={sources.view}
            value={sources.report}
            error={sources.error}
            testid="cashflow-sources-error"
            label="the cash-flow sources"
            loadingLabel="Loading cash-flow sources"
            onRetry={sources.retry}
        >
            {#snippet children(breakdown)}
                <PeriodStackView report={breakdown} {styles} mark="area" netLabel="Net change" empty="No cash moved in this range." testid="cashflow-chart" />
            {/snippet}
        </AsyncSection>
    {/if}
</ChartPanel>
