<script lang="ts">
    // Holdings route (WP-10, now native): scope bar on top, collapsible insight
    // section (pie + stat tiles + gainers/losers), details table below. The
    // report + trend are fetched from the ledgeline-engine /api/holdings[/series]
    // endpoints for the current scope and decoded into the existing domain types,
    // so the UI renders unchanged; scope/as-of changes refetch. Scope lives in
    // the URL (?asof=&acct=&mode=) via the WP-04 replaceState pattern. Display
    // styles for the base commodity come from the journal wire feed (styleFor).
    import {onMount} from "svelte";
    import {NativeApiUnavailableError} from "$lib/api/native";
    import {formatAmount, type Dec} from "$lib/domain/money";
    import {exportHoldingsXlsx} from "$lib/export/xlsx";
    import GainersLosers from "$lib/holdings/ui/GainersLosers.svelte";
    import HoldingsPie from "$lib/holdings/ui/HoldingsPie.svelte";
    import HoldingsStats from "$lib/holdings/ui/HoldingsStats.svelte";
    import HoldingsTable from "$lib/holdings/ui/HoldingsTable.svelte";
    import HoldingsTrend from "$lib/holdings/ui/HoldingsTrend.svelte";
    import ScopeBar from "$lib/holdings/ui/ScopeBar.svelte";
    import {startHoldingsUrlSync} from "$lib/holdings/ui/urlSync";
    import {stockAccounts} from "$lib/holdings/ui/view";
    import {formatChartValue, styleFor} from "$lib/insights/series";
    import ExportButton from "$lib/reports/ui/ExportButton.svelte";
    import {holdingsData, holdingsScope} from "$lib/stores/holdings.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    // Reset the scope from the URL once (fresh visits open at today), then
    // mirror changes back (debounced replaceState). onMount's return value is
    // its cleanup.
    onMount(() => startHoldingsUrlSync());

    // Load the journal once a server URL is configured (base-commodity styles + the scope-bar account list only).
    let attemptedUrl: string | null = null;
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== null && url !== attemptedUrl) {
            attemptedUrl = url;
            void journal.refresh();
        }
    });

    // Fetch the native holdings report + trend whenever the scope changes (or the server is first configured).
    $effect(() => {
        const url = settings.serverUrl;
        const scope = holdingsScope.value;
        if (url !== null) void holdingsData.load(url, scope);
    });

    const report = $derived(holdingsData.report);
    const trend = $derived(holdingsData.trend);
    const ready = $derived(report !== null && journal.txns.length > 0);
    const nativeUnavailable = $derived(holdingsData.error instanceof NativeApiUnavailableError);

    const base = $derived(report?.base ?? "$");
    const style = $derived(styleFor(journal.txns, base));
    const format = (qty: Dec): string => formatAmount({commodity: base, qty, style});
    const formatTrendValue = (v: number): string => formatChartValue(v, base, style);
    const accountNames = $derived(stockAccounts(journal.txns));

    let insightsOpen = $state(true);
</script>

<svelte:head><title>Ledgeline — Holdings</title></svelte:head>

<div class="flex flex-col gap-3">
    <ScopeBar {accountNames} />

    {#if ready && report !== null}
        {#if report.holdings.length > 0}
            <section class="collapse-arrow bg-base-200 collapse" data-testid="holdings-insights">
                <input type="checkbox" bind:checked={insightsOpen} aria-label="Toggle holdings insights" />
                <div class="collapse-title flex min-h-0 items-center justify-between gap-2 py-3 pr-10">
                    <h2 class="text-sm font-semibold tracking-tight">Insights</h2>
                    <span class="text-sm">
                        <span class="text-base-content/60 mr-1">Market value</span>
                        <span class="font-semibold">{format(report.totals.marketValue)}</span>
                    </span>
                </div>
                <div class="collapse-content flex flex-col gap-4">
                    <HoldingsStats totals={report.totals} {format} />
                    <div class="grid grid-cols-1 items-center gap-4 lg:grid-cols-2">
                        <div>
                            <HoldingsPie holdings={report.holdings} {format} />
                        </div>
                        <GainersLosers {report} {format} />
                    </div>
                    {#if trend !== null}
                        <HoldingsTrend {trend} formatValue={formatTrendValue} />
                    {/if}
                </div>
            </section>
        {/if}

        {#if report.warnings.length > 0}
            <div class="alert alert-warning rounded-box items-start px-3 py-2 text-sm" role="alert" data-testid="holdings-warnings">
                <ul class="list-inside list-disc">
                    {#each report.warnings as warning (warning.symbol + warning.kind)}
                        <li>{warning.message}</li>
                    {/each}
                </ul>
            </div>
        {/if}

        {#if report.holdings.length === 0}
            <div class="card bg-base-200" data-testid="holdings-empty">
                <div class="card-body items-center py-16 text-center">
                    <h2 class="card-title">No stock holdings in scope</h2>
                    <p class="text-base-content/60">
                        No non-currency commodities are held by the selected accounts as of {report.asOf}. Widen the scope or pick a later date.
                    </p>
                </div>
            </div>
        {:else}
            <div class="flex justify-end">
                <ExportButton run={() => exportHoldingsXlsx(report, {title: "Holdings", params: `As of ${report.asOf}`}, `holdings-${report.asOf}.xlsx`)} />
            </div>
            <HoldingsTable holdings={report.holdings} totals={report.totals} {format} />
        {/if}
    {:else if report === null && holdingsData.status === "error"}
        <div class="alert alert-error rounded-box flex-col items-start gap-2 px-3 py-3 text-sm" role="alert" data-testid="holdings-error">
            <span>{nativeUnavailable ? holdingsData.error?.message : `Couldn't load holdings: ${holdingsData.error?.message ?? "unknown error"}`}</span>
            {#if !nativeUnavailable}
                <button type="button" class="btn btn-sm" onclick={() => void holdingsData.load(settings.serverUrl ?? "", holdingsScope.value)}>Retry</button>
            {/if}
        </div>
    {:else}
        <div class="flex items-center justify-center py-24" aria-label="Loading holdings">
            <span class="loading loading-spinner loading-lg"></span>
        </div>
    {/if}
</div>

{#if journal.status === "error" && journal.error !== null}
    <div class="toast toast-end z-30">
        <div class="alert alert-error">
            <span class="max-w-xs truncate" title={journal.error}>{journal.error}</span>
            <button type="button" class="btn btn-sm" onclick={() => void journal.refresh()}>Retry</button>
        </div>
    </div>
{/if}
