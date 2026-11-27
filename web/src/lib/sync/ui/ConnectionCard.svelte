<!-- One connection on the Accounts tab (plan 43 §4.2): "SimpleFIN Bridge ●
     Connected", who sees the data, the ⋯ menu, its problems, and every account
     it reaches — linked rows first, then the ones not linked yet with Link….

     Disconnect asks first, with the plan's wording: it removes the access key
     from this computer only, and says how to stop SimpleFIN itself. -->
<script lang="ts">
    import {dismissible} from "#ui/keys/dismissible.js";
    import type {ProblemEffect} from "../problemAction";
    import {syncStore} from "../syncStore.svelte";
    import type {WireConnection, WireLinkedAccount, WireRemoteAccount} from "../types";
    import DisconnectConfirm from "./DisconnectConfirm.svelte";
    import LinkedAccountRow from "./LinkedAccountRow.svelte";
    import ProblemNotice from "./ProblemNotice.svelte";
    import WhoSeesData from "./WhoSeesData.svelte";

    let {
        connection,
        accounts,
        unlinked,
        now,
        onOpenAccount,
        onLink,
        onReplaceToken,
        onEffect,
    }: {
        connection: WireConnection;
        accounts: readonly WireLinkedAccount[];
        unlinked: readonly WireRemoteAccount[];
        now: number;
        onOpenAccount: (key: string) => void;
        /** Open the wizard's link step over these accounts. */
        onLink: (accounts: readonly WireRemoteAccount[]) => void;
        onReplaceToken: (connectionId: string) => void;
        onEffect: (effect: ProblemEffect) => void;
    } = $props();

    const STATE = {
        ok: {label: "Connected", dot: "status-success"},
        syncing: {label: "Syncing", dot: "status-info"},
        attention: {label: "Needs attention", dot: "status-warning"},
        broken: {label: "Not working", dot: "status-error"},
        "not-connected": {label: "Not connected on this computer", dot: "status-neutral"},
        elsewhere: {label: "Syncs on another computer", dot: "status-neutral"},
    } as const;
    const elsewhere = $derived(connection.state === "elsewhere");

    let menuOpen = $state(false);
    let confirmDisconnect = $state(false);
    let busy = $state(false);
    let note = $state<{text: string; error: boolean} | null>(null);

    function closeMenu(): void {
        menuOpen = false;
    }

    async function syncThis(): Promise<void> {
        closeMenu();
        await syncStore.syncNow({connectionId: connection.id});
    }

    async function recheck(): Promise<void> {
        closeMenu();
        busy = true;
        note = null;
        const outcome = await syncStore.recheckAccounts(connection.id);
        busy = false;
        if (!outcome.ok) {
            note = {text: outcome.failure.message, error: true};
            return;
        }
        const linkedIds = new Set(accounts.map((a) => a.remoteId));
        const fresh = outcome.value.filter((r) => !linkedIds.has(r.remoteId));
        if (fresh.length === 0) note = {text: "No new accounts at SimpleFIN.", error: false};
        else onLink(fresh);
    }

    async function copyDiagnostics(): Promise<void> {
        closeMenu();
        note = null;
        const outcome = await syncStore.diagnostics(connection.id);
        if (!outcome.ok) {
            note = {text: outcome.failure.message, error: true};
            return;
        }
        try {
            await navigator.clipboard.writeText(outcome.value);
            note = {text: "Diagnostic report copied. Your access key isn't in it.", error: false};
        } catch {
            note = {text: "Couldn't copy to the clipboard.", error: true};
        }
    }
</script>

<section class="flex flex-col gap-2" aria-label={connection.name} data-testid="sync-connection">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-1">
        <h3 class="font-semibold">{connection.name}</h3>
        <span class="flex items-center gap-1.5 text-sm text-muted">
            <span class="status {STATE[connection.state].dot}" aria-hidden="true"></span>
            {STATE[connection.state].label}
        </span>
        <div class="ml-auto flex items-center gap-2">
            <WhoSeesData summary="who sees your data" />
            <details class="dropdown dropdown-end" bind:open={menuOpen} use:dismissible={{onDismiss: closeMenu, active: menuOpen, outside: true}}>
                <summary class="btn btn-square btn-ghost btn-xs" aria-label="{connection.name} actions">⋯</summary>
                <ul class="menu dropdown-content z-20 w-56 rounded-box bg-base-200 p-1 text-sm shadow">
                    <li>
                        <button type="button" disabled={syncStore.running || !connection.connected} onclick={() => void syncThis()}
                            >Sync this connection now</button
                        >
                    </li>
                    <li><button type="button" disabled={busy || !connection.connected} onclick={() => void recheck()}>Re-check accounts</button></li>
                    <li>
                        <button
                            type="button"
                            onclick={() => {
                                closeMenu();
                                onReplaceToken(connection.id);
                            }}>{connection.connected ? "Replace token…" : elsewhere ? "Sync from this computer too…" : "Connect on this computer…"}</button
                        >
                    </li>
                    <li><button type="button" onclick={() => void copyDiagnostics()}>Copy diagnostic report</button></li>
                    {#if connection.connected}
                        <li>
                            <button
                                type="button"
                                class="text-error-text"
                                onclick={() => {
                                    closeMenu();
                                    confirmDisconnect = true;
                                }}>Disconnect…</button
                            >
                        </li>
                    {/if}
                </ul>
            </details>
        </div>
    </header>

    {#if confirmDisconnect}
        <DisconnectConfirm {connection} onDone={() => (confirmDisconnect = false)} />
    {/if}

    {#if elsewhere}
        <div class="flex flex-wrap items-center justify-between gap-2 text-sm text-muted" data-testid="sync-elsewhere">
            <p>Another computer syncs these accounts. New transactions arrive here when the ledger's files do.</p>
            <button type="button" class="btn btn-ghost btn-xs" onclick={() => onReplaceToken(connection.id)}>Sync from this computer too…</button>
        </div>
    {/if}

    {#each connection.problems.filter((p) => p.severity !== "info") as problem, i (i)}
        <ProblemNotice {problem} scope={{connectionId: connection.id, accountKey: null}} {onEffect} busy={syncStore.running} />
    {/each}

    {#if note !== null}
        <p class="text-xs {note.error ? 'text-error-text' : 'text-muted'}" role={note.error ? "alert" : "status"}>{note.text}</p>
    {/if}

    <ul class="rounded-box border border-base-content/10">
        {#each accounts as account (account.key)}
            <LinkedAccountRow {account} {now} connected={connection.connected} {elsewhere} onOpen={onOpenAccount} {onEffect} />
        {/each}
        {#each unlinked as remote (remote.remoteId)}
            <li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-base-content/5 px-3 py-2 last:border-b-0" data-testid="sync-unlinked-row">
                <span class="font-medium">{remote.label}</span>
                <span class="text-xs text-muted">→ not linked</span>
                <button type="button" class="btn ml-auto btn-xs" onclick={() => onLink([remote])}>Link…</button>
            </li>
        {/each}
        {#if accounts.length === 0 && unlinked.length === 0}
            <li class="px-3 py-2 text-sm text-muted">No accounts linked through this connection.</li>
        {/if}
    </ul>
</section>
