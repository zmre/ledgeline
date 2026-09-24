<!-- The bottom half: three readings of one projection, behind a tab strip over
     shared interval/count controls.

     - NET INCOME is a `PeriodReport`, so `ReportTable` renders it with no edits
       at all. It arrives in CASH-FLOW ORIENTATION — revenue positive, expenses
       negative — which is the opposite of `/api/reports/incomestatement` and is
       exactly what the flow chart under it wants. Nothing here re-signs it; the
       chart is handed magnitudes because `PeriodFlowChart` owns the sign.
     - CASH & RUNWAY answers the question the tab exists for, and answers it in
       WORDS above the chart. A crossing point a reader has to find by eye on a
       line is not an answer to "when do I run out".
     - NET WORTH is one line, with the opening balance named beside the heading
       rather than plotted: it is a real figure as of today, not a projected
       bucket, and drawing it as a point on the same series would make the first
       segment a claim the engine is not making. Under it, when the scenario
       holds asset rows, a BREAKDOWN of which of them the growth came from —
       one line is self-explanatory until there are three assets behind it, and
       then it stops being (plan 23, Phase 3). A table rather than a second
       chart: these are a handful of labelled exact figures, which is a table's
       job, and a second series on the same axes would say the parts and the
       whole are the same kind of thing.

     One commodity is charted (`chartCommodity`) because there is no market
     price for a future date to value a mixed amount with. What that leaves out
     is named under the chart rather than silently dropped. -->
<script lang="ts">
    import PeriodFlowChart from "$lib/components/PeriodFlowChart.svelte";
    import PeriodLineChart from "$lib/components/PeriodLineChart.svelte";
    import type {AmountStyle} from "$lib/domain/types";
    import {styleOf} from "$lib/format/amounts";
    import {formatChartValue, formatCompactChartValue} from "$lib/insights/series";
    import {bucketLabel} from "$lib/reports/periods";
    import ReportTable from "$lib/reports/ui/ReportTable.svelte";
    import type {ReportInterval} from "$lib/reports/ui/params";
    import {PROJECTION_TABS, PROJECTION_TAB_LABELS, type ProjectionTab} from "../params";
    import {assetContributions, balanceNumbers, chartCommodity, flowSeries, otherCommodities, runwayStatement} from "../projectionView";
    import type {Projection} from "../types";

    let {
        tab = $bindable(),
        projection,
        interval,
        styles,
        fallbackCommodity,
    }: {
        tab: ProjectionTab;
        projection: Projection;
        /** The interval the buckets were built on — the noun the runway sentence counts in. */
        interval: ReportInterval;
        styles: ReadonlyMap<string, AmountStyle>;
        /** Used only when the projection names no commodity at all (an empty scenario over an empty journal). */
        fallbackCommodity: string;
    } = $props();

    const commodity = $derived(chartCommodity(projection, fallbackCommodity));
    const style = $derived<AmountStyle>(styleOf(styles, commodity));
    const format = $derived((n: number) => formatChartValue(n, commodity, style));
    const formatAxis = $derived((n: number) => formatCompactChartValue(n, commodity, style));

    const labels = $derived(projection.buckets.map(bucketLabel));
    const flows = $derived(flowSeries(projection.netIncome, commodity));
    const cash = $derived(balanceNumbers(projection.cash, commodity));
    const worth = $derived(balanceNumbers(projection.netWorth, commodity));
    const statement = $derived(runwayStatement(projection, interval, commodity, format));
    const unchartedCommodities = $derived(otherCommodities(projection, commodity));
    const contributions = $derived(assetContributions(projection, commodity));
    const grownTotal = $derived(contributions.reduce((sum, row) => sum + row.growth, 0));
</script>

{#snippet uncharted()}
    {#if unchartedCommodities.length > 0}
        <p class="text-xs text-base-content/60" data-testid="projection-uncharted">
            Charted in {commodity} only. This projection also holds {unchartedCommodities.join(", ")}, which no future price can value into {commodity}.
        </p>
    {/if}
{/snippet}

<div class="flex flex-col gap-3">
    <div role="tablist" class="tabs tabs-border" aria-label="Projection report">
        {#each PROJECTION_TABS as t (t)}
            <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
                {PROJECTION_TAB_LABELS[t]}
            </button>
        {/each}
    </div>

    {#if tab === "net"}
        <div class="flex flex-col gap-3" data-testid="projection-net">
            <!-- Unchanged: a `PeriodReport` is a `PeriodReport`. -->
            <ReportTable report={projection.netIncome} {styles} />
            <PeriodFlowChart
                heading="Projected net income"
                note="from {projection.start}"
                {labels}
                inflows={flows.inflows}
                outflows={flows.outflows}
                net={flows.net}
                formatValue={format}
                {formatAxis}
                empty="Add a line above to project net income."
                testid="projection-flow-chart"
            />
            {@render uncharted()}
        </div>
    {:else if tab === "cash"}
        <div class="flex flex-col gap-3" data-testid="projection-cash">
            <!-- The answer, in a sentence, above the picture of it. -->
            <div
                class="alert items-start py-2 text-sm {statement.tone === 'warn' ? 'alert-warning' : ''}"
                role={statement.tone === "warn" ? "alert" : "status"}
                data-testid="projection-runway"
            >
                <span class="grow">
                    <span class="font-semibold">{statement.headline}</span>
                    {#if statement.detail !== null}<span class="text-base-content/70"> {statement.detail}</span>{/if}
                </span>
            </div>
            <PeriodLineChart
                heading="Projected cash"
                note="opening {format(cash.opening)} · from {projection.start}"
                {labels}
                series={[{name: "Cash", values: cash.values}]}
                formatValue={format}
                {formatAxis}
                includeZero
                markAt={projection.runway?.bucket ?? null}
                empty="Add a line above to project cash."
                testid="projection-cash-chart"
            />
            {@render uncharted()}
        </div>
    {:else}
        <div class="flex flex-col gap-3" data-testid="projection-worth">
            <PeriodLineChart
                heading="Projected net worth"
                note="opening {format(worth.opening)} · from {projection.start}"
                {labels}
                series={[{name: "Net worth", values: worth.values}]}
                formatValue={format}
                {formatAxis}
                empty="Add a line above to project net worth."
                testid="projection-worth-chart"
            />
            {#if contributions.length > 0}
                <div class="overflow-x-auto">
                    <table class="table table-xs" data-testid="projection-asset-breakdown">
                        <caption class="px-1 pb-1 text-left text-xs text-base-content/60">
                            Where the growth came from. Appreciation is unrealised: it never reaches cash or net income.
                        </caption>
                        <thead>
                            <tr>
                                <th>Asset</th>
                                <th class="text-right">Balance it grew from</th>
                                <th class="text-right">Growth</th>
                            </tr>
                        </thead>
                        <tbody>
                            {#each contributions as row (row.account)}
                                <tr>
                                    <td>{row.account}</td>
                                    <td class="text-right tabular-nums">{format(row.opening)}</td>
                                    <td class="text-right tabular-nums">{format(row.growth)}</td>
                                </tr>
                            {/each}
                        </tbody>
                        <tfoot>
                            <tr>
                                <!-- The total of the GROWTH column only. The balances are
                                     already inside the opening figure above the chart, so
                                     summing them here would offer a number that is part of
                                     one total and all of a different one. -->
                                <th colspan="2" class="text-right font-normal">Total growth</th>
                                <th class="text-right tabular-nums">{format(grownTotal)}</th>
                            </tr>
                        </tfoot>
                    </table>
                </div>
            {/if}
            <p class="text-xs text-base-content/60">
                Summary level only: an asset's balance is held flat unless an asset row gives it a rate or a line posts to it, and no liability is amortised.
            </p>
            {@render uncharted()}
        </div>
    {/if}
</div>
