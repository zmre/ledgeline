<!-- "Share them" (plan 43 §3.2): before a manual rules file's categorization
     is moved into a shared `<account>.categories.rules`, show what moves, what
     stays and why, both files' diffs, and the proof — hledger reads the manual
     file's last statement the same before and after. Nothing is written here:
     confirming only marks the account to share, and the wizard's Finish does
     the write. A failed check keeps Share them shut. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {dismissible} from "#ui/keys/dismissible.js";
    import {syncStore, type SyncFailure} from "../syncStore.svelte";
    import type {WireSharePreview} from "../types";

    let {
        manualRulesId,
        journalAccount,
        onConfirm,
        onCancel,
    }: {
        manualRulesId: string;
        journalAccount: string;
        onConfirm: () => void;
        onCancel: () => void;
    } = $props();

    const headingId = $props.id();
    let preview = $state.raw<WireSharePreview | null>(null);
    let failure = $state.raw<SyncFailure | null>(null);

    onMount(() => {
        void (async () => {
            const outcome = await syncStore.sharePreview(manualRulesId, journalAccount);
            if (outcome.ok) preview = outcome.value;
            else failure = outcome.failure;
        })();
    });

    const fileName = (id: string): string => id.split("/").at(-1) ?? id;
</script>

<div
    class="modal modal-open"
    role="dialog"
    aria-modal="true"
    aria-labelledby={headingId}
    tabindex="-1"
    data-testid="sync-share-dialog"
    use:dismissible={{trap: true, onDismiss: onCancel}}
>
    <div class="modal-box flex max-h-[90vh] max-w-3xl flex-col gap-3 overflow-y-auto">
        <h3 id={headingId} class="text-base font-semibold">Share categorization with sync</h3>
        {#if failure !== null}
            <p class="text-sm text-error" role="alert">{failure.message}</p>
        {:else if preview === null}
            <p class="flex items-center gap-2 text-sm text-muted" role="status">
                <span class="loading loading-xs loading-spinner"></span> Checking {fileName(manualRulesId)}…
            </p>
        {:else}
            <p class="text-sm">
                {preview.moved} rule{preview.moved === 1 ? "" : "s"} move from <code>{fileName(preview.manualRulesId)}</code> into
                <code>{fileName(preview.categoriesId)}</code>. Both the manual and the sync rules include it, so a rule you add on either side works for both.
            </p>
            {#if preview.kept.length > 0}
                <div class="text-sm">
                    <p class="font-medium">These stay in {fileName(preview.manualRulesId)}:</p>
                    <ul class="list-inside list-disc text-base-content/80" data-testid="sync-share-kept">
                        {#each preview.kept as kept, i (i)}
                            <li><code>{kept.header}</code> — {kept.reason}</li>
                        {/each}
                    </ul>
                </div>
            {/if}
            {#if preview.identical}
                <p class="text-sm text-success" data-testid="sync-share-identical">✓ hledger reads your last statement exactly the same before and after.</p>
            {:else}
                <div class="alert flex flex-col items-start alert-soft text-sm alert-error" role="alert" data-testid="sync-share-differs">
                    <p>Sharing would change how your last statement imports, so it's off.</p>
                    {#if preview.differences !== null}
                        <pre class="max-h-48 w-full overflow-auto text-xs">{preview.differences}</pre>
                    {/if}
                </div>
            {/if}
            <details class="text-sm">
                <summary class="cursor-pointer">Changes to {fileName(preview.manualRulesId)}</summary>
                <pre class="mt-1 max-h-64 overflow-auto rounded-box bg-base-200 p-2 text-xs">{preview.diffs.manual}</pre>
            </details>
            <details class="text-sm">
                <summary class="cursor-pointer">New {fileName(preview.categoriesId)}</summary>
                <pre class="mt-1 max-h-64 overflow-auto rounded-box bg-base-200 p-2 text-xs">{preview.diffs.categories}</pre>
            </details>
            <p class="text-xs text-muted">Nothing is written until you press Finish.</p>
        {/if}
        <div class="modal-action">
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel}>Cancel</button>
            <button
                type="button"
                class="btn btn-primary btn-sm"
                disabled={preview === null || !preview.identical}
                onclick={onConfirm}
                data-testid="sync-share-confirm"
            >
                Share them
            </button>
        </div>
    </div>
    <button type="button" class="modal-backdrop" aria-label="Close" onclick={onCancel}></button>
</div>
