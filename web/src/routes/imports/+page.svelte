<script lang="ts">
    // The Imports route (WP-11): an outer subnav and nothing else.
    //
    // Two screens live under it — "New Transactions" (drop a statement, import
    // it) and "Edit Rules" (the structured `*.rules` editor, which WAS this
    // route until the subnav arrived and now lives in `EditRulesPanel`). The
    // split is a straight move: this file owns the tab and the URL, the panel
    // owns everything it always owned.
    //
    // The tab lives in the query string via the same three pieces the reports
    // route uses — `searchToParams` once on mount behind a `restored` latch, a
    // debounced `replaceState` mirror the other way, and `paramsToSearch` read
    // BEFORE the latch is checked so the effect depends on the params even on
    // the run where it declines to write. Flat route, `?tab=`, no nested
    // directory: this codebase has none, and Reports does not use one either.
    import {onMount} from "svelte";
    import ErrorToast from "$lib/components/ErrorToast.svelte";
    import {defaultImportParams, paramsToSearch, searchToParams, type ImportParams} from "$lib/imports/params";
    import {rulesStore} from "$lib/imports/rulesStore.svelte";
    import EditRulesPanel from "$lib/imports/ui/EditRulesPanel.svelte";
    import ImportTabs from "$lib/imports/ui/ImportTabs.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {loadJournalWhenReady, onServerReady} from "$lib/stores/serverWatch.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {searchMirror} from "$lib/url/searchSync";

    // The account autocomplete is the only thing this screen wants from the
    // journal feed, and `AccountInput` already copes with an empty list. Both
    // loads stay on the HOST rather than moving into the panel with everything
    // else: the host mounts once per visit, so switching tabs cannot turn into a
    // journal refetch — `refresh()` only dedupes calls while a round is still in
    // flight, and a remounted panel would ask again every time.
    loadJournalWhenReady();
    onServerReady((url) => void rulesStore.ensureIndex(url, settings.serverNonce));

    let params = $state<ImportParams>(defaultImportParams());
    let restored = $state(false);

    // Restore params from the URL exactly once, at startup.
    onMount(() => {
        if (window.location.search !== "") Object.assign(params, searchToParams(window.location.search, defaultImportParams()));
        restored = true;
        return () => mirror.stop();
    });

    // Mirror params → URL, debounced, replaceState (no history entries, no loops).
    // Reading `params` before the `restored` guard is deliberate: the effect has
    // to depend on them even on the run where it declines to write.
    const mirror = searchMirror();
    $effect(() => {
        const search = paramsToSearch(params);
        if (!restored) return;
        mirror.write(search);
    });
</script>

<svelte:head><title>Ledgeline — Imports</title></svelte:head>

<div class="flex flex-col gap-3">
    <ImportTabs bind:tab={params.tab} />

    {#if params.tab === "new"}
        <!-- Placeholder. The real drop target, preview, candidate ranking and
             dry-run land in the next WP-11 lane; this exists so the subnav is
             navigable and its default tab is not a blank page. -->
        <div class="card bg-base-200" data-testid="imports-new-placeholder">
            <div class="card-body items-center py-16 text-center">
                <h2 class="card-title">Importing statements is coming</h2>
                <p class="text-base-content/60 max-w-lg">
                    This is where you will drop a statement — CSV, TSV, OFX/QFX or a spreadsheet — and Ledgeline will offer the rules file that fits it, show
                    the transactions it proposes, and reconcile them against your statement balance before anything is written.
                </p>
                <p class="text-base-content/50 max-w-lg text-xs">
                    Until then, <strong>Edit Rules</strong> maintains the <code>*.rules</code> files an import reads a CSV through.
                </p>
            </div>
        </div>
    {:else}
        <EditRulesPanel />
    {/if}
</div>

<ErrorToast message={journal.status === "error" ? journal.error : null} onRetry={() => void journal.refresh({force: true})} />
