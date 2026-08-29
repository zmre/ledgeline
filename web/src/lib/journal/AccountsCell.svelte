<script lang="ts">
    // From→to account chips (WP-03): `source → dest` for simple two-sided txns,
    // degrading to a wrapped account list for N-way splits. Names that do not
    // fit are shortened by AccountLabel from the LEFT, which is the end nobody
    // reads — the full name stays in the hover tooltip (native title).
    //
    // Editing: when the native engine is connected (editing.canEdit), each chip
    // is a button; clicking it swaps the cell for an autocomplete input that
    // recategorizes EVERY posting on that account (usually one) via a surgical
    // PATCH {postings:[{index, account}]}. Enter/blur commits, Escape cancels.
    import {accountPatch} from "$lib/api/editMapping";
    import AccountLabel from "$lib/components/AccountLabel.svelte";
    import {chipMeasurer} from "$lib/components/textWidth";
    import type {Transaction} from "$lib/domain/types";
    import {editing} from "$lib/stores/editing.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {accountColumn} from "./accountColumn.svelte";
    import {flowChipRooms, splitChipRooms} from "./chipGeometry";
    import AccountInput from "./edit/AccountInput.svelte";
    import {rowActions} from "./rowAction.svelte";
    import {accountFlow} from "./rowModel";

    let {txn}: {txn: Transaction} = $props();

    const flow = $derived(accountFlow(txn));
    const canEdit = $derived(editing.canEdit);

    // Room in px for each chip's text, or null when nothing has measured the
    // column (card layout, server render, first frame) — the labels then fall
    // back to their character budget. The chips are NOT width-capped in CSS any
    // more; these numbers only choose which string goes in, and flexbox still
    // does the laying out.
    const chipRooms = $derived.by(() => {
        const measure = chipMeasurer();
        const cell = accountColumn.width;
        if (measure === null || cell <= 0) return null;
        if (flow.kind === "flow") return flowChipRooms([flow.source, flow.dest], cell, measure);
        return splitChipRooms(flow.accounts, cell);
    });

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

    // Answer `c` from the keyboard. This cell owns `startEdit`, so it consumes
    // the request directly rather than routing it through TransactionRow.
    //
    // Targets the flow's DESTINATION — the category you actually recategorize —
    // falling back to the first account of an N-way split.
    $effect(() => {
        const request = rowActions.request;
        if (request === null || request.txnIndex !== txn.index || request.action !== "category") return;
        rowActions.consume(request.nonce);
        startEdit(flow.kind === "flow" ? flow.dest : (flow.accounts[0] ?? ""));
    });
</script>

{#if editingAccount !== null}
    <AccountInput bind:value={draft} accountNames={journal.accountNames} size="xs" autofocus onCommit={commit} onCancel={cancel} />
{:else if flow.kind === "flow"}
    <div class="flex min-w-0 items-center gap-1">
        <!-- `min-w-0` and no max-width: the two chips SHARE the cell by flex
             shrink instead of being capped at 45% each. A capped pair could not
             lend slack to one another, so a short source next to a long
             destination left the end of the column empty. -->
        {#if canEdit}
            <button
                type="button"
                class="badge min-w-0 cursor-pointer badge-ghost badge-sm hover:badge-outline"
                title="Edit category · {flow.source}"
                onclick={() => startEdit(flow.source)}
                ><AccountLabel name={flow.source} title="Edit category · {flow.source}" maxWidth={chipRooms?.[0]} /></button
            >
            <span class="shrink-0 text-base-content/50" aria-label="to">&rarr;</span>
            <button
                type="button"
                class="badge min-w-0 cursor-pointer badge-ghost badge-sm hover:badge-outline"
                title="Edit category · {flow.dest}"
                onclick={() => startEdit(flow.dest)}><AccountLabel name={flow.dest} title="Edit category · {flow.dest}" maxWidth={chipRooms?.[1]} /></button
            >
        {:else}
            <span class="badge min-w-0 badge-ghost badge-sm" title={flow.source}><AccountLabel name={flow.source} maxWidth={chipRooms?.[0]} /></span>
            <span class="shrink-0 text-base-content/50" aria-label="to">&rarr;</span>
            <span class="badge min-w-0 badge-ghost badge-sm" title={flow.dest}><AccountLabel name={flow.dest} maxWidth={chipRooms?.[1]} /></span>
        {/if}
    </div>
{:else}
    <!-- `max-w-full` rather than the old `max-w-44` (176px): this list wraps, so
         a chip too wide to share a line gets one to itself. Capping it at 176px
         inside a ~450px cell left the rest of that line blank for nothing. -->
    <div class="flex min-w-0 flex-wrap gap-1">
        {#each flow.accounts as account, index (account)}
            {#if canEdit}
                <button
                    type="button"
                    class="badge max-w-full min-w-0 cursor-pointer badge-ghost badge-sm hover:badge-outline"
                    title="Edit category · {account}"
                    onclick={() => startEdit(account)}><AccountLabel name={account} title="Edit category · {account}" maxWidth={chipRooms?.[index]} /></button
                >
            {:else}
                <span class="badge max-w-full min-w-0 badge-ghost badge-sm" title={account}><AccountLabel name={account} maxWidth={chipRooms?.[index]} /></span>
            {/if}
        {/each}
    </div>
{/if}
