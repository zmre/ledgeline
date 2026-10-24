<!-- The bottom half: four readings of one projection, behind a tab strip over
     shared interval/count/lookback controls (plans/28-projected-statements.md).

     - RUNWAY answers the question only this tab asks, and answers it in WORDS
       above the chart: a crossing point a reader has to find by eye on a line
       is not an answer to "when do I run out". Cash only (Decision 8), over the
       forecast periods only — the runway counts from the cutoff.
     - BALANCE SHEET, INCOME STATEMENT and CASH FLOW are the Reports tab's own
       statements, rendered by the Reports tab's own views, over the real
       journal before the cutoff and the scenario's generated transactions
       after it. The first `lookback` columns are actuals; `projectedFrom` is
       where the views draw the divider and print "Projected".

     The runway chart plots one commodity (`chartCommodity`) because a mixed
     amount has no single number in it; what that leaves out is named under the
     chart rather than silently dropped. The statements need no such rule: they
     are valued into one base commodity by the engine, as on the Reports tab. -->
<script lang="ts">
    import PeriodLineChart from "#lib/components/PeriodLineChart.svelte";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {styleOf} from "#lib/format/amounts.js";
    import {formatChartValue, formatCompactChartValue} from "#lib/insights/series.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import BalanceSheetView from "#lib/reports/ui/BalanceSheetView.svelte";
    import CashFlowStatementView from "#lib/reports/ui/CashFlowStatementView.svelte";
    import IncomeStatementView from "#lib/reports/ui/IncomeStatementView.svelte";
    import type {PeriodInterval} from "#lib/reports/ui/params.js";
    import {PROJECTION_TABS, PROJECTION_TAB_LABELS, type ProjectionTab} from "../params";
    import {balanceNumbers, chartCommodity, otherCommodities, runwayStatement} from "../projectionView";
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
        interval: PeriodInterval;
        styles: ReadonlyMap<string, AmountStyle>;
        /** Used only when the projection names no commodity at all (an empty scenario over an empty journal). */
        fallbackCommodity: string;
    } = $props();

    const commodity = $derived(chartCommodity(projection, fallbackCommodity));
    const style = $derived<AmountStyle>(styleOf(styles, commodity));
    const format = $derived((n: number) => formatChartValue(n, commodity, style));
    const formatAxis = $derived((n: number) => formatCompactChartValue(n, commodity, style));

    /** The forecast periods' labels: the engine's own, off the statements' columns past the divider. */
    const labels = $derived(projection.balanceSheet.columns.slice(projection.projectedFrom).map((column) => column.label));
    const cash = $derived(balanceNumbers(projection.cash, commodity));
    const statement = $derived(runwayStatement(projection, interval, commodity, format));
    const unchartedCommodities = $derived(otherCommodities(projection, commodity));

    // Digits, as on the Reports and Holdings strips: page-local, so they appear
    // in `?` only while this strip is mounted. Like every key they are ignored
    // while focus is in a field, and most of this page is fields.
    registerKeys({
        id: "projection-tabs",
        priority: PRIORITY.page,
        bindings: PROJECTION_TABS.map((t, at) => ({
            keys: String(at + 1),
            label: PROJECTION_TAB_LABELS[t],
            group: "Projections" as const,
            run: () => (tab = t),
        })),
    });
</script>

<div class="flex flex-col gap-3">
    <div role="tablist" class="tabs tabs-border" aria-label="Projection report">
        {#each PROJECTION_TABS as t (t)}
            <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
                {PROJECTION_TAB_LABELS[t]}
            </button>
        {/each}
    </div>

    {#if tab === "cash"}
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
                note="opening {format(cash.opening)} · from {projection.actualsEnd}"
                {labels}
                series={[{name: "Cash", values: cash.values}]}
                formatValue={format}
                {formatAxis}
                includeZero
                markAt={projection.runway?.bucket ?? null}
                empty="Add a line above to project cash."
                testid="projection-cash-chart"
            />
            {#if unchartedCommodities.length > 0}
                <p class="text-xs text-base-content/60" data-testid="projection-uncharted">
                    Charted in {commodity} only. This projection also holds {unchartedCommodities.join(", ")}, which no future price can value into {commodity}.
                </p>
            {/if}
        </div>
    {:else if tab === "bs"}
        <div data-testid="projection-bs">
            <BalanceSheetView report={projection.balanceSheet} {styles} projectedFrom={projection.projectedFrom} />
        </div>
    {:else if tab === "is"}
        <div data-testid="projection-is">
            <IncomeStatementView report={projection.incomeStatement} {styles} projectedFrom={projection.projectedFrom} />
        </div>
    {:else}
        <div data-testid="projection-cf">
            <CashFlowStatementView report={projection.cashFlow} {styles} base={projection.balanceSheet.base} projectedFrom={projection.projectedFrom} />
        </div>
    {/if}
</div>
