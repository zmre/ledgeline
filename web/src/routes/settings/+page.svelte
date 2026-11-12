<script lang="ts">
    // Settings route: an outer subnav, same shape as Imports (WP-11) and
    // Reports. Five screens live under it — "Accounts" (the chart-of-accounts
    // editor), "Account Aliases" (moved here from Imports; the panel and
    // its store are unchanged, only the tab that hosts it moved), "hledger
    // config" (the `hledger.conf` in force: --auto and report depths),
    // "Automatic postings" (the journal's `=` rules) and "Projections" (which
    // scenario the Projections tab opens by default, plan 33). The tab
    // lives in the query string via the same three pieces the Imports route
    // uses — `searchToParams` after each navigation behind a `restored` latch,
    // a debounced `replaceState` mirror the other way.
    import {onMount} from "svelte";
    import {afterNavigate} from "$app/navigation";
    import ErrorToast from "#lib/components/ErrorToast.svelte";
    import AccountsPanel from "#lib/accounts/ui/AccountsPanel.svelte";
    import AliasPanel from "#lib/imports/ui/AliasPanel.svelte";
    import HledgerConfPanel from "#lib/settings/ui/HledgerConfPanel.svelte";
    import AutoRulesPanel from "#lib/autoRules/ui/AutoRulesPanel.svelte";
    import ProjectionSettingsPanel from "#lib/projections/ui/ProjectionSettingsPanel.svelte";
    import {defaultSettingsParams, paramsToSearch, searchToParams, type SettingsParams} from "#lib/settings/params.js";
    import SettingsTabs from "#lib/settings/ui/SettingsTabs.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {loadJournalWhenReady} from "#lib/stores/serverWatch.svelte.js";
    import {searchMirror} from "#lib/url/searchSync.js";

    // The account tree's autocomplete data — the same feed
    // `AccountInput`/`AccountTreeSelect` read — is the only thing the
    // Accounts tab wants from the journal. Loaded on the host, not the panel,
    // so switching tabs cannot turn into a refetch.
    loadJournalWhenReady();

    let params = $state<SettingsParams>(defaultSettingsParams());
    let restored = $state(false);

    // Read the tab from the address after EVERY navigation, not once on mount:
    // a link from one Settings tab to another (the Automatic postings notice
    // to hledger config, a generated posting's "open its rule") stays on this
    // route, so a mount-only read left the old tab showing under the new URL.
    // The mirror below writes with shallow `replaceState`, which is not a
    // navigation, so this never echoes our own writes back.
    //
    // ALSO read on mount: the layout holds every page until the view settings
    // load (plan 38 WP-5), so on a cold start this page can mount AFTER the
    // first navigation has finished, and `afterNavigate` never fires for it.
    // Restoring twice is harmless — it is the same parse of the same address.
    function restore(): void {
        Object.assign(params, searchToParams(window.location.search, defaultSettingsParams()));
        restored = true;
    }
    afterNavigate(restore);
    onMount(() => {
        restore();
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
    {:else if params.tab === "hledgerConf"}
        <HledgerConfPanel />
    {:else if params.tab === "auto"}
        <!-- Plan 30 Phase 5; `rule` is the rule a generated posting linked to. -->
        <AutoRulesPanel focus={params.rule} />
    {:else if params.tab === "projections"}
        <ProjectionSettingsPanel />
    {:else}
        <AccountsPanel accountNames={journal.accountNames} />
    {/if}
</div>

<ErrorToast message={journal.status === "error" ? journal.error : null} onRetry={() => void journal.refresh({force: true})} />
