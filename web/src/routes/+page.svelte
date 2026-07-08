<script lang="ts">
    // Journal route (WP-03): filter bar (WP-04) and insights panel (WP-05) mount
    // above the virtualized transaction table; the totals footer stays pinned.
    // On mount (and whenever a server URL is first configured) → journal.refresh().
    import TotalsFooter from "$lib/journal/TotalsFooter.svelte";
    import TransactionTable from "$lib/journal/TransactionTable.svelte";
    import {commodityStyles, periodLabel} from "$lib/journal/rowModel";
    import {filters} from "$lib/stores/filters.svelte";
    import {getFilteredTotals, getFilteredTxns, journal} from "$lib/stores/journal.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    const txns = $derived(getFilteredTxns());
    const totals = $derived(getFilteredTotals());
    const styles = $derived(commodityStyles(journal.txns));
    const period = $derived(periodLabel(filters.value.from, filters.value.to));

    let attemptedUrl: string | null = null;
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== null && url !== attemptedUrl) {
            attemptedUrl = url;
            void journal.refresh();
        }
    });
</script>

<svelte:head><title>Ledgeline — Journal</title></svelte:head>

<div class="flex min-h-0 flex-col gap-3" style="height: calc(100dvh - 7rem)">
    <!-- INTEGRATION(WP-04): FilterBar mounts here (replace this placeholder div) -->
    <div id="filterbar-placeholder" class="border-base-300 text-base-content/40 rounded-box border border-dashed px-3 py-2 text-sm">Filter bar (WP-04)</div>

    <!-- INTEGRATION(WP-05): InsightsPanel mounts here (replace this placeholder div) -->
    <div id="insights-placeholder" class="border-base-300 text-base-content/40 rounded-box border border-dashed px-3 py-2 text-sm">Insights panel (WP-05)</div>

    {#if journal.status === "loading" && journal.txns.length === 0}
        <div class="flex grow items-center justify-center" aria-label="Loading transactions">
            <span class="loading loading-spinner loading-lg"></span>
        </div>
    {:else}
        <TransactionTable {txns} />
    {/if}

    <TotalsFooter {totals} {styles} count={txns.length} {period} />
</div>

{#if journal.status === "error" && journal.error !== null}
    <div class="toast toast-end z-30">
        <div class="alert alert-error">
            <span class="max-w-xs truncate" title={journal.error}>{journal.error}</span>
            <button type="button" class="btn btn-sm" onclick={() => void journal.refresh()}>Retry</button>
        </div>
    </div>
{/if}
