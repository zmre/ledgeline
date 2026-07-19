<script lang="ts">
    // Inline status toggle (native engine only): clicking a transaction's
    // cleared/pending indicator CYCLES its status unmarked → pending → cleared →
    // unmarked, committing each step with a surgical PATCH {status} via
    // editing.patch (which refetches the journal on success). Mirrors the
    // description/account inline edits: gated by editing.canEdit (a read-only
    // StatusBadge otherwise) and reporting non-conflict failures as a toast.
    // A cycle button has no text draft, so there is no Esc/blur state to manage —
    // Enter/Space activate it natively; the store's 409/refetch handling is unchanged.
    import {statusPatch} from "$lib/api/editMapping";
    import type {Transaction, TxnStatus} from "$lib/domain/types";
    import {editing} from "$lib/stores/editing.svelte";
    import StatusBadge from "./StatusBadge.svelte";

    let {txn}: {txn: Transaction} = $props();

    const canEdit = $derived(editing.canEdit);

    const NEXT: Record<TxnStatus, TxnStatus> = {unmarked: "pending", pending: "cleared", cleared: "unmarked"};

    let busy = $state(false);

    async function cycle(): Promise<void> {
        if (!canEdit || busy) return;
        busy = true;
        const result = await editing.patch(txn.index, statusPatch(NEXT[txn.status]));
        busy = false;
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }
</script>

{#if canEdit}
    <button
        type="button"
        class="cursor-pointer disabled:cursor-wait"
        onclick={cycle}
        disabled={busy}
        title="Status: {txn.status} · click to cycle"
        aria-label="Cycle status (currently {txn.status})"
    >
        <StatusBadge status={txn.status} />
    </button>
{:else}
    <StatusBadge status={txn.status} />
{/if}
