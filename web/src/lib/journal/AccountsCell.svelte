<script lang="ts">
    // From→to account chips (WP-03): `source → dest` for simple two-sided txns,
    // degrading to a wrapped account list for N-way splits. Long names truncate
    // with the full name in a hover tooltip (native title).
    //
    // Editing: when the native engine is connected (editing.canEdit), each chip
    // is a button; clicking it swaps the cell for an autocomplete input that
    // recategorizes EVERY posting on that account (usually one) via a surgical
    // PATCH {postings:[{index, account}]}. Enter/blur commits, Escape cancels.
    import {accountPatch} from "$lib/api/editMapping";
    import type {Transaction} from "$lib/domain/types";
    import {editing} from "$lib/stores/editing.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import AccountInput from "./edit/AccountInput.svelte";
    import {accountFlow} from "./rowModel";

    let {txn}: {txn: Transaction} = $props();

    const flow = $derived(accountFlow(txn));
    const canEdit = $derived(editing.canEdit);

    // The account currently being edited (null = not editing), plus its draft value.
    let editingAccount = $state<string | null>(null);
    let draft = $state("");

    function startEdit(account: string): void {
        if (!canEdit) return;
        editingAccount = account;
        draft = account;
    }

    async function commit(): Promise<void> {
        const previous = editingAccount;
        if (previous === null) return;
        const next = draft.trim();
        editingAccount = null;
        if (next === "" || next === previous) return;
        const patch = accountPatch(txn, previous, next);
        if (patch.postings === undefined || patch.postings.length === 0) return;
        const result = await editing.patch(txn.index, patch);
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }

    function cancel(): void {
        editingAccount = null;
    }
</script>

{#if editingAccount !== null}
    <AccountInput bind:value={draft} accountNames={journal.accountNames} size="xs" autofocus onCommit={commit} onCancel={cancel} />
{:else if flow.kind === "flow"}
    <div class="flex min-w-0 items-center gap-1">
        {#if canEdit}
            <button
                type="button"
                class="badge badge-ghost badge-sm hover:badge-outline min-w-0 max-w-[45%] cursor-pointer"
                title="Edit category · {flow.source}"
                onclick={() => startEdit(flow.source)}><span class="truncate">{flow.source}</span></button
            >
            <span class="text-base-content/50 shrink-0" aria-label="to">&rarr;</span>
            <button
                type="button"
                class="badge badge-ghost badge-sm hover:badge-outline min-w-0 max-w-[45%] cursor-pointer"
                title="Edit category · {flow.dest}"
                onclick={() => startEdit(flow.dest)}><span class="truncate">{flow.dest}</span></button
            >
        {:else}
            <span class="badge badge-ghost badge-sm min-w-0 max-w-[45%]" title={flow.source}><span class="truncate">{flow.source}</span></span>
            <span class="text-base-content/50 shrink-0" aria-label="to">&rarr;</span>
            <span class="badge badge-ghost badge-sm min-w-0 max-w-[45%]" title={flow.dest}><span class="truncate">{flow.dest}</span></span>
        {/if}
    </div>
{:else}
    <div class="flex min-w-0 flex-wrap gap-1">
        {#each flow.accounts as account (account)}
            {#if canEdit}
                <button
                    type="button"
                    class="badge badge-ghost badge-sm hover:badge-outline max-w-44 min-w-0 cursor-pointer"
                    title="Edit category · {account}"
                    onclick={() => startEdit(account)}><span class="truncate">{account}</span></button
                >
            {:else}
                <span class="badge badge-ghost badge-sm max-w-44 min-w-0" title={account}><span class="truncate">{account}</span></span>
            {/if}
        {/each}
    </div>
{/if}
