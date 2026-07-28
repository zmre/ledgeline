<script lang="ts">
    // Journal route (WP-03): filter bar (WP-04) and insights panel (WP-05) mount
    // above the virtualized transaction table; the totals footer stays pinned.
    // On mount (and whenever a server URL is first configured) → journal.refresh().
    import {onMount, untrack} from "svelte";
    import FilterBar from "$lib/filters/FilterBar.svelte";
    import {startUrlSync} from "$lib/filters/urlSync";
    import InsightsPanel from "$lib/insights/InsightsPanel.svelte";
    import {visibleNet} from "$lib/insights/series";
    import {declaredTypes} from "$lib/domain/accountTypes";
    import TotalsFooter from "$lib/journal/TotalsFooter.svelte";
    import TransactionTable from "$lib/journal/TransactionTable.svelte";
    import TransactionModal from "$lib/journal/edit/TransactionModal.svelte";
    import {periodLabel} from "$lib/journal/rowModel";
    import {editing} from "$lib/stores/editing.svelte";
    import {filters} from "$lib/stores/filters.svelte";
    import {getFilteredTxns, journal, startPolling} from "$lib/stores/journal.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    const txns = $derived(getFilteredTxns());
    const period = $derived(periodLabel(filters.value.from, filters.value.to));
    const declared = $derived(declaredTypes(journal.accountDecls));
    const total = $derived(visibleNet(txns, filters.value.accounts, journal.txns, declared));

    /**
     * The journal could not be loaded and there is nothing held over from a
     * previous load.
     *
     * Everything below summarizes `journal.txns`, and an empty array is a
     * perfectly valid input to all of it — so a failed load used to fall
     * through to the happy path and describe itself as a clean, empty journal:
     * "No transactions match the current filters" (a causal claim about the
     * filters that is simply untrue), Income / Expenses / Net all `$0.00` with
     * Net in success green, "0 transactions" in the footer. Only a corner toast
     * said otherwise (FE-5b). A load that failed AFTER data arrived is
     * different and keeps the old rows plus that toast, which is the intended
     * stale-but-labelled behaviour.
     */
    const loadFailed = $derived(journal.status === "error" && journal.txns.length === 0);

    // Restore filters from ?from=&to=&acct=&q= once, then mirror changes to the
    // URL (debounced replaceState). onMount's return value is its cleanup.
    onMount(() => startUrlSync());

    let attempted: string | null = null;
    $effect(() => {
        const url = settings.serverUrl;
        // Keyed on the nonce too: reconnecting usually leaves the URL identical
        // (the engine restarted on the same port), and this guard latching on
        // the URL alone is why a reconnect never re-probed (FE-5d).
        const key = `${settings.serverNonce}|${url}`;
        if (url !== null && key !== attempted) {
            attempted = key;
            void journal.refresh();
            // Detect the native write endpoints so edit affordances only show
            // against the Ledgeline engine (not a plain, read-only hledger-web).
            void editing.probe();
        }
    });

    // A probe that could not reach the server left `canEdit` a guess rather than
    // a fact (FE-5g). Re-ask on the next round that DID reach it, so a transient
    // blip costs the edit affordances one refresh, not the session. Keyed on
    // `fetchedAt` so it fires once per successful round; `probeError` is read
    // untracked so re-probing cannot re-trigger this effect.
    let lastProbedAt: number | null = null;
    $effect(() => {
        const at = journal.fetchedAt;
        if (at === null || at === lastProbedAt) return;
        lastProbedAt = at;
        if (untrack(() => editing.probeError) !== null) void editing.probe();
    });

    // WP-08: live updates while the journal page is open. startPolling pauses
    // itself while the tab is hidden; the returned stop fn is the effect cleanup
    // (runs on unmount and if the server URL changes).
    $effect(() => {
        if (settings.serverUrl === null) return;
        return startPolling();
    });
</script>

<svelte:head><title>Ledgeline — Journal</title></svelte:head>

<div class="flex min-h-0 flex-col gap-3" style="height: calc(100dvh - 7rem)">
    <FilterBar accountNames={journal.accountNames} />

    <!-- The summaries are suppressed rather than fed an empty journal: zeroes
         computed from nothing are indistinguishable from zeroes computed from a
         journal that really nets to zero. -->
    {#if !loadFailed}
        <InsightsPanel {txns} accounts={filters.value.accounts} allTxns={journal.txns} {declared} />
    {/if}

    {#if loadFailed}
        <div class="flex grow items-center justify-center" data-testid="journal-error">
            <div class="alert alert-error rounded-box max-w-xl flex-col items-start gap-2 px-4 py-3 text-sm" role="alert">
                <span class="font-semibold">Couldn't load the journal — no transactions were read.</span>
                <span class="break-words">{journal.error ?? "unknown error"}</span>
                <button type="button" class="btn btn-sm" onclick={() => void journal.refresh({force: true})}>Retry</button>
            </div>
        </div>
    {:else if journal.status === "loading" && journal.txns.length === 0}
        <div class="flex grow items-center justify-center" aria-label="Loading transactions">
            <span class="loading loading-spinner loading-lg"></span>
        </div>
    {:else}
        <TransactionTable {txns} />
    {/if}

    {#if !loadFailed}
        <TotalsFooter count={txns.length} {period} {total} />
    {/if}
</div>

<!-- The add/edit-all transaction popup (mounted once; driven by the txnModal store). -->
<TransactionModal />

{#if editing.conflict}
    <div class="toast toast-center toast-top z-40">
        <div class="alert alert-warning">
            <span>The journal changed on disk — the view was refreshed. Re-apply your edit if needed.</span>
            <button type="button" class="btn btn-sm" onclick={() => editing.clearConflict()}>Dismiss</button>
        </div>
    </div>
{/if}

{#if editing.notice !== null}
    <div class="toast toast-end z-40">
        <div class="alert alert-error max-w-md">
            <span class="grow break-words whitespace-pre-wrap">{editing.notice.message}</span>
            <button type="button" class="btn btn-sm shrink-0" onclick={() => editing.clearNotice()}>Dismiss</button>
        </div>
    </div>
{/if}

<!-- Stale-data case only: when the load failed with nothing to show, the panel
     above says so in full and this would just repeat it. -->
{#if journal.status === "error" && journal.error !== null && !loadFailed}
    <div class="toast toast-end z-30">
        <div class="alert alert-error">
            <span class="max-w-xs truncate" title={journal.error}>{journal.error}</span>
            <button type="button" class="btn btn-sm" onclick={() => void journal.refresh({force: true})}>Retry</button>
        </div>
    </div>
{/if}
