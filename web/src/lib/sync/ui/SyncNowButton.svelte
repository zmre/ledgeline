<!-- "Sync now" (plan 43 §4.5): the twin of Holdings' Update prices — a
     primary button with a spinner, the click is the consent and writes, and a
     toast says what happened: "Synced 3 accounts · 9 new transactions ·
     Review 4". Review opens Journal → Review Recents over what came in.

     Past SimpleFIN's comfortable daily count the line under the button warns,
     and at the hard stop the button is off with the reason. -->
<script lang="ts">
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
    import {quotaNote, runToast} from "../statusLine";
    import {syncStore} from "../syncStore.svelte";

    let {disabled = false}: {disabled?: boolean} = $props();

    const connections = $derived(syncStore.status?.connections ?? []);
    const quota = $derived(quotaNote(connections));
    const toast = $derived(syncStore.lastRun === null ? null : runToast(syncStore.lastRun));
    const hasLinks = $derived((syncStore.status?.accounts.length ?? 0) > 0);

    async function review(): Promise<void> {
        syncStore.dismissRun();
        reviewLaunch.open("recents");
        await goto(resolve("/"));
    }
</script>

{#if hasLinks}
    <div class="flex flex-col items-end gap-1">
        <button
            type="button"
            class="btn btn-primary btn-sm"
            disabled={disabled || syncStore.running || quota?.blocked === true}
            onclick={() => void syncStore.syncNow()}
            data-testid="sync-now"
        >
            {#if syncStore.running}<span class="loading loading-xs loading-spinner"></span>{/if}
            Sync now
        </button>
        {#if quota !== null}
            <span class="max-w-xs text-right text-xs {quota.blocked ? 'text-error' : 'text-warning'}" data-testid="sync-quota">{quota.text}</span>
        {/if}
    </div>
{/if}

{#if toast !== null}
    <div class="toast toast-end z-30" data-testid="sync-toast">
        <div class="alert {toast.failed ? 'alert-error' : 'alert-success'}" role="status">
            <span>{toast.text}</span>
            {#if toast.review > 0}
                <button type="button" class="btn btn-sm" onclick={() => void review()}>Review {toast.review}</button>
            {/if}
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => syncStore.dismissRun()}>Dismiss</button>
        </div>
    </div>
{:else if syncStore.lastRunError !== null}
    <div class="toast toast-end z-30" data-testid="sync-toast">
        <div class="alert alert-error" role="alert">
            <span class="max-w-xs">Sync failed: {syncStore.lastRunError.message}</span>
            <button type="button" class="btn btn-sm" onclick={() => syncStore.dismissRun()}>Dismiss</button>
        </div>
    </div>
{/if}
