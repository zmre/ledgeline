<!-- The bottom half: six readings of one projection, behind a tab strip over
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
     - ACCOUNTS is every account per period over the same columns, as written
       (`hledger balance -M`), as each period's Change or its closing Balance,
       optionally with the accounts nothing touches. JOURNAL is the generated
       transactions themselves (`hledger print --forecast tag:generated`).
       Both are computed only while their tab is open (`projection.accounts`
       and `projection.journal` are null otherwise), so a projection held from
       another tab shows a placeholder until the re-run lands.

     The runway chart plots one commodity (`chartCommodity`) because a mixed
     amount has no single number in it; what that leaves out is named under the
     chart rather than silently dropped. The statements need no such rule: they
     are valued into one base commodity by the engine, as on the Reports tab. -->
<script lang="ts">
    import PeriodLineChart from "#lib/components/PeriodLineChart.svelte";
    import ExportButton from "#lib/reports/ui/ExportButton.svelte";
    import {projectionExport} from "../export";
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
    import ForecastJournalView from "./ForecastJournalView.svelte";
    import AccountReportView from "#lib/reports/ui/AccountReportView.svelte";

    let {
        tab = $bindable(),
        emptyAccounts = $bindable(false),
        projection,
        interval,
        styles,
        fallbackCommodity,
        scenarioName = "",
    }: {
        tab: ProjectionTab;
        /** The Accounts tab: whether accounts with nothing in any column are rows too. */
        emptyAccounts?: boolean;
        projection: Projection;
        /** The interval the buckets were built on — the noun the runway sentence counts in. */
        interval: PeriodInterval;
        styles: ReadonlyMap<string, AmountStyle>;
        /** Used only when the projection names no commodity at all (an empty scenario over an empty journal). */
        fallbackCommodity: string;
        /** The scenario's display name, for the downloaded workbook's title and filename. */
        scenarioName?: string;
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

    let strip: HTMLDivElement | undefined = $state();

    /** The Journal tab's search, held here so the download matches the list. */
    let journalQuery = $state("");
    /** The open reading's download; null while it is not on hand (Accounts/Journal before their run). */
    const download = $derived(
        projectionExport(tab, projection, {
            scenarioName,
            runwaySentence: [statement.headline, statement.detail].filter((part) => part !== null).join(" "),
            base: projection.balanceSheet.base ?? commodity,
            journalQuery,
            styleOf: (symbol) => styleOf(styles, symbol),
        })
    );

    /**
     * Open a report and bring its tab strip to the top of the window, so the
     * reading fills the screen. The panels differ a lot in height (Journal and
     * Accounts are a one-line placeholder until their run lands), and without
     * this a shorter panel clamped the page's scroll and left the strip at the
     * bottom of the viewport. `min-h-dvh` on the wrapper below is what lets the
     * strip reach the top even under the shortest panel.
     */
    function select(next: ProjectionTab): void {
        tab = next;
        strip?.scrollIntoView({block: "start", behavior: "smooth"});
    }

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
            run: () => select(t),
        })),
    });
</script>

<div class="flex min-h-dvh flex-col gap-3">
    <div bind:this={strip} class="flex flex-wrap items-end justify-between gap-2">
        <div role="tablist" class="tabs tabs-border" aria-label="Projection report">
            {#each PROJECTION_TABS as t (t)}
                <button
                    type="button"
                    role="tab"
                    class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}"
                    aria-selected={t === tab}
                    onclick={() => select(t)}
                >
                    {PROJECTION_TAB_LABELS[t]}
                </button>
            {/each}
        </div>
        {#if download !== null}
            {@const current = download}
            <ExportButton run={current.run} label="Export {PROJECTION_TAB_LABELS[tab]} as xlsx" />
        {/if}
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
    {:else if tab === "cf"}
        <div data-testid="projection-cf">
            <CashFlowStatementView report={projection.cashFlow} {styles} base={projection.balanceSheet.base} projectedFrom={projection.projectedFrom} />
        </div>
    {:else if tab === "accounts"}
        <div class="flex flex-col gap-3" data-testid="projection-accounts">
            <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
                <label class="label cursor-pointer gap-2 text-sm">
                    <input type="checkbox" class="checkbox checkbox-sm" bind:checked={emptyAccounts} />
                    Show empty accounts
                </label>
            </div>
            {#if projection.accounts === null}
                <p class="text-sm text-base-content/60" role="status" data-testid="projection-accounts-pending">Reading every account…</p>
            {:else}
                <AccountReportView report={projection.accounts} {styles} projectedFrom={projection.projectedFrom} />
            {/if}
        </div>
    {:else}
        <div data-testid="projection-journal">
            {#if projection.journal === null}
                <p class="text-sm text-base-content/60" role="status" data-testid="projection-journal-pending">Listing the generated transactions…</p>
            {:else}
                <ForecastJournalView journal={projection.journal} bind:query={journalQuery} />
            {/if}
        </div>
    {/if}
</div>
