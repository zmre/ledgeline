<script lang="ts">
    // One side's whole transaction, expanded in place (plan 45 Phase 3b):
    // fetched on first show and cached per generation (`txnDetails`), a
    // one-line skeleton while it loads, Retry if it failed, then the journal's
    // own read-only body with the matched posting highlighted, and Open in
    // editor when the host can open one.
    import TxnDetailBody from "#lib/journal/TxnDetailBody.svelte";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {txnDetails} from "../details.svelte";

    let {
        index,
        highlight = null,
        onEdit,
    }: {
        /** The transaction's tindex. */
        index: number;
        /** The posting the card matched, or null. */
        highlight?: number | null;
        /** Open the transaction in the review editor; absent, no link. */
        onEdit?: (index: number) => void;
    } = $props();

    // Ask on mount, and again whenever the journal moves on (the cache drops
    // a stale generation, so the transaction is read afresh).
    $effect(() => {
        void journal.generation;
        txnDetails.request(index);
    });

    const detail = $derived(txnDetails.get(index));
</script>

<div class="mt-2 min-w-0 rounded-box border border-base-content/10 bg-base-200/40 p-3" data-testid="txn-details" data-index={index}>
    {#if detail === null || detail.state === "loading"}
        <div class="h-4 w-full skeleton opacity-50" aria-label="Loading the transaction" data-testid="txn-details-loading"></div>
    {:else if detail.state === "failed"}
        <p class="flex flex-wrap items-center gap-2 text-sm text-error" role="alert" data-testid="txn-details-failed">
            Couldn't load transaction #{index}: {detail.error}
            <button type="button" class="btn btn-ghost btn-xs" onclick={() => txnDetails.retry(index)} data-testid="txn-details-retry">Retry</button>
        </p>
    {:else}
        <TxnDetailBody txn={detail.txn} {highlight} compact />
        {#if onEdit !== undefined}
            <p class="mt-2 text-right">
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => onEdit(index)} data-testid="txn-details-edit">
                    <ReviewIcon name="pencil" class="h-3.5 w-3.5" /> Open in editor
                </button>
            </p>
        {/if}
    {/if}
</div>
