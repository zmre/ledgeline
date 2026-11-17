<!-- Disconnect, asked first (plan 43 §4.4), with the plan's wording: it
     removes the access key from this computer only, and says how to stop
     SimpleFIN itself. Used by the connection card's ⋯ menu and Settings → Sync. -->
<script lang="ts">
    import {SIMPLEFIN_URL, safeUrl} from "../problemAction";
    import {syncStore} from "../syncStore.svelte";
    import type {WireConnection} from "../types";

    let {connection, onDone}: {connection: WireConnection; onDone: () => void} = $props();

    let busy = $state(false);
    let error = $state<string | null>(null);

    async function disconnect(): Promise<void> {
        busy = true;
        error = null;
        const outcome = await syncStore.disconnect(connection.id);
        busy = false;
        if (outcome.ok) onDone();
        else error = outcome.failure.message;
    }
</script>

<div
    class="alert flex flex-col items-start gap-2 alert-soft text-sm alert-warning"
    role="alertdialog"
    aria-label="Disconnect {connection.name}"
    data-testid="sync-disconnect"
>
    <p>
        Removes the access key from this Mac. Your transactions and linked accounts stay, and other computers that open this ledger keep syncing. To stop
        SimpleFIN itself sharing data, also disable the 'Ledgeline' app on SimpleFIN.
    </p>
    <div class="flex flex-wrap gap-2">
        <button type="button" class="btn btn-error btn-xs" disabled={busy} onclick={() => void disconnect()}>Disconnect</button>
        <a class="btn btn-xs" href={safeUrl(connection.manageUrl) ?? SIMPLEFIN_URL} target="_blank" rel="noopener noreferrer external">Open SimpleFIN ↗</a>
        <button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={onDone}>Cancel</button>
    </div>
    {#if error !== null}<p class="text-error" role="alert">{error}</p>{/if}
</div>
