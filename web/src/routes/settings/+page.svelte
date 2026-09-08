<script lang="ts">
    // Settings route: an outer subnav, same shape as Imports (WP-11) and
    // Reports. Two screens live under it — "Accounts" (the chart-of-accounts
    // editor) and "Account Aliases" (moved here from Imports; the panel and
    // its store are unchanged, only the tab that hosts it moved). The tab
    // lives in the query string via the same three pieces the Imports route
    // uses — `searchToParams` once on mount behind a `restored` latch, a
    // debounced `replaceState` mirror the other way.
    import {onMount} from "svelte";
    import ErrorToast from "$lib/components/ErrorToast.svelte";
    import AccountsPanel from "$lib/accounts/ui/AccountsPanel.svelte";
    import AliasPanel from "$lib/imports/ui/AliasPanel.svelte";
    import {defaultSettingsParams, paramsToSearch, searchToParams, type SettingsParams} from "$lib/settings/params";
    import SettingsTabs from "$lib/settings/ui/SettingsTabs.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {loadJournalWhenReady} from "$lib/stores/serverWatch.svelte";
    import {searchMirror} from "$lib/url/searchSync";

    // The account tree's autocomplete data — the same feed
    // `AccountInput`/`AccountTreeSelect` read — is the only thing the
    // Accounts tab wants from the journal. Loaded on the host, not the panel,
    // so switching tabs cannot turn into a refetch.
    loadJournalWhenReady();

    let params = $state<SettingsParams>(defaultSettingsParams());
    let restored = $state(false);

    onMount(() => {
        if (window.location.search !== "") Object.assign(params, searchToParams(window.location.search, defaultSettingsParams()));
        restored = true;
        return () => mirror.stop();
    });

    const mirror = searchMirror();
    $effect(() => {
        const search = paramsToSearch(params);
        if (!restored) return;
        mirror.write(search);
    });
</script>

<svelte:head><title>Ledgeline — Settings</title></svelte:head>

<div class="flex flex-col gap-3">
    <SettingsTabs bind:tab={params.tab} />

    {#if params.tab === "aliases"}
        <AliasPanel />
    {:else}
        <AccountsPanel accountNames={journal.accountNames} />
    {/if}
</div>

<ErrorToast message={journal.status === "error" ? journal.error : null} onRetry={() => void journal.refresh({force: true})} />
