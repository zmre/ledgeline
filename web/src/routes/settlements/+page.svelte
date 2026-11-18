<script lang="ts">
    // The Invoices page (plan 45 Q9): invoice ↔ payment matching on a page of
    // its own, the same panel the Recognition overlay's Invoices tab shows.
    // Shown in the nav only when the ledger has accrual accounts (D-gate);
    // reached directly without them, it says when it would appear.
    import {onMount} from "svelte";
    import {accrual} from "#lib/accrual/store.svelte.js";
    import SettlementsPanel from "#lib/accrual/ui/SettlementsPanel.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {loadJournalWhenReady} from "#lib/stores/serverWatch.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";

    const DOCS = "https://zmre.github.io/ledgeline/journal/invoice-matching/";

    loadJournalWhenReady();

    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        if (serverUrl === null || generation === null) return;
        void accrual.sync(serverUrl, generation);
    });

    // The page's list is read on mount and after every write; the gate opening
    // later (the status arriving after mount) reads it then.
    onMount(() => {
        accrual.setPageOpen(true);
        return () => accrual.setPageOpen(false);
    });
    $effect(() => {
        if (accrual.enabled && accrual.settlements === null) void accrual.reload();
    });
</script>

<svelte:head><title>Ledgeline — Invoices</title></svelte:head>

<div class="mx-auto flex w-full max-w-5xl flex-col gap-4" data-testid="settlements-page">
    <h1 class="text-lg font-semibold">Invoices</h1>
    {#if accrual.enabled}
        <SettlementsPanel />
    {:else}
        <p class="text-sm text-base-content/70" data-testid="settlements-off">
            Invoice matching appears when the ledger has a receivable or payable account.
            <a class="link" href={DOCS} target="_blank" rel="noopener noreferrer">How it works</a>
        </p>
    {/if}
</div>
