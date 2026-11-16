<!-- Imports → Accounts (plan 43 §4.2): every account the journal gets
     transactions for, and how — SimpleFIN connections with their linked
     accounts, then the accounts imported by hand — with Sync now, the status
     line, and + Connect a bank.

     The wizard and the account drawer open from here. A fix pressed somewhere
     else (the Problems drawer) arrives as `syncStore.intent` and is taken
     when this panel mounts. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {resolve} from "$app/paths";
    import type {ProblemEffect} from "../problemAction";
    import {statusLine} from "../statusLine";
    import {syncStore, type PanelIntent} from "../syncStore.svelte";
    import type {WireRemoteAccount} from "../types";
    import type {WizardStep} from "../wizardModel";
    import AccountDrawer from "./AccountDrawer.svelte";
    import ConnectionCard from "./ConnectionCard.svelte";
    import ConnectWizard from "./ConnectWizard.svelte";
    import {runEffect} from "./effects";
    import LinkedAccountRow from "./LinkedAccountRow.svelte";
    import ManualAccountRow from "./ManualAccountRow.svelte";
    import SyncNowButton from "./SyncNowButton.svelte";

    const status = $derived(syncStore.status);

    // The wall clock a minute at a time, for "last synced 7:12 AM".
    let now = $state(Math.floor(Date.now() / 1000));
    $effect(() => {
        const timer = setInterval(() => (now = Math.floor(Date.now() / 1000)), 60_000);
        return () => clearInterval(timer);
    });

    let wizard = $state.raw<{start: WizardStep; connectionId: string | null; accounts: readonly WireRemoteAccount[]} | null>(null);
    let drawer = $state.raw<{key: string; focus: "held" | "mapping" | null} | null>(null);
    let highlightManual = $state(false);
    let manualSection: HTMLElement | undefined = $state();

    const drawerAccount = $derived(drawer === null ? null : (status?.accounts.find((a) => a.key === drawer?.key) ?? null));
    const connectionIds = $derived(new Set(status?.connections.map((c) => c.id) ?? []));
    const orphans = $derived(status?.accounts.filter((a) => !connectionIds.has(a.connectionId)) ?? []);
    const line = $derived(status === null ? null : statusLine(status, now));
    const nothingYet = $derived(status !== null && status.connections.length === 0 && status.accounts.length === 0);

    function open(intent: PanelIntent): void {
        switch (intent.type) {
            case "reconnect":
                wizard = {start: intent.connectionId === null ? "choose" : "simplefin", connectionId: intent.connectionId, accounts: []};
                return;
            case "connect":
                wizard = {start: "choose", connectionId: null, accounts: []};
                return;
            case "review-held":
            case "choose-file":
                if (intent.accountKey !== null) drawer = {key: intent.accountKey, focus: intent.type === "review-held" ? "held" : "mapping"};
                return;
        }
    }

    const onEffect = (effect: ProblemEffect): void => void runEffect(effect, open);

    function closeWizard(result: {highlightManual: boolean}): void {
        wizard = null;
        if (result.highlightManual) {
            highlightManual = true;
            manualSection?.scrollIntoView({block: "start", behavior: "smooth"});
        }
    }

    onMount(() => void syncStore.loadManual());

    // A request from elsewhere — taken on mount, and when one arrives while this panel shows (the Problems drawer over the Accounts tab).
    $effect(() => {
        if (syncStore.intent === null) return;
        untrack(() => {
            const pending = syncStore.takeIntent();
            if (pending !== null) open(pending);
        });
    });
</script>

<section class="flex max-w-4xl flex-col gap-4" data-testid="sync-accounts">
    {#if syncStore.unsupported}
        <div class="alert alert-soft text-sm alert-info" role="status">
            This Ledgeline engine can't sync with banks. Update Ledgeline to link accounts through SimpleFIN.
        </div>
    {:else if status === null}
        <p class="flex items-center gap-2 text-sm text-base-content/60" role="status">
            <span class="loading loading-xs loading-spinner"></span> Loading accounts…
        </p>
    {:else if !status.available}
        <div class="alert alert-soft text-sm alert-info" role="status">{status.unavailableReason ?? "Bank sync isn't available here."}</div>
    {:else}
        <div class="flex flex-wrap items-start justify-between gap-3">
            <div class="flex flex-col gap-1">
                {#if line !== null}<span class="text-sm" data-testid="sync-status-line">{line}</span>{/if}
                <a href="{resolve('/settings')}?tab=sync" class="link text-xs text-base-content/70 link-hover">⚙ Sync settings</a>
                {#if syncStore.pollError !== null}
                    <span class="text-xs text-warning" role="status">Couldn't refresh: {syncStore.pollError}</span>
                {/if}
            </div>
            {#if !status.connections.every((c) => c.state === "elsewhere") || status.connections.length === 0}<SyncNowButton />{/if}
        </div>

        {#if nothingYet}
            <p class="text-sm text-base-content/80">
                Link a bank account and Ledgeline brings in its new transactions for you to review. Or keep downloading statements yourself: the accounts below
                show when each was last imported.
            </p>
        {/if}

        {#each status.connections as connection (connection.id)}
            <ConnectionCard
                {connection}
                accounts={status.accounts.filter((a) => a.connectionId === connection.id)}
                unlinked={status.unlinked.filter((u) => u.connectionId === connection.id)}
                {now}
                onOpenAccount={(key) => (drawer = {key, focus: null})}
                onLink={(accounts) => (wizard = {start: "link", connectionId: connection.id, accounts})}
                onReplaceToken={(id) => (wizard = {start: "simplefin", connectionId: id, accounts: []})}
                {onEffect}
            />
        {/each}

        {#if orphans.length > 0}
            <section class="flex flex-col gap-2" aria-label="Not connected on this computer">
                <h3 class="font-semibold">Not connected on this computer</h3>
                <p class="text-xs text-base-content/70">These links are in this ledger's settings, but this computer has no access key for them.</p>
                <ul class="rounded-box border border-base-content/10">
                    {#each orphans as account (account.key)}
                        <LinkedAccountRow {account} {now} connected={false} onOpen={(key) => (drawer = {key, focus: null})} {onEffect} />
                    {/each}
                </ul>
            </section>
        {/if}

        <div>
            <button type="button" class="btn btn-sm" onclick={() => (wizard = {start: "choose", connectionId: null, accounts: []})} data-testid="sync-connect">
                + Connect a bank
            </button>
        </div>
    {/if}

    {#if !syncStore.unsupported}
        <section class="flex flex-col gap-2" aria-label="Manual downloads" bind:this={manualSection} data-testid="sync-manual">
            <h3 class="font-semibold">Manual downloads</h3>
            {#if syncStore.manualError !== null}
                <p class="text-sm text-error" role="alert">{syncStore.manualError}</p>
            {:else if syncStore.manual === null}
                <p class="text-sm text-base-content/60">Loading…</p>
            {:else if syncStore.manual.length === 0}
                <p class="text-sm text-base-content/60">Every bank and card account in your journal is linked.</p>
            {:else}
                <ul class="rounded-box border border-base-content/10">
                    {#each syncStore.manual as account (account.account)}
                        <ManualAccountRow {account} highlight={highlightManual} />
                    {/each}
                </ul>
            {/if}
        </section>
    {/if}
</section>

{#if wizard !== null}
    <ConnectWizard start={wizard.start} connectionId={wizard.connectionId} accounts={wizard.accounts} onClose={closeWizard} />
{/if}

{#if drawerAccount !== null && drawer !== null}
    <AccountDrawer account={drawerAccount} focus={drawer.focus} {now} onClose={() => (drawer = null)} {onEffect} />
{/if}
