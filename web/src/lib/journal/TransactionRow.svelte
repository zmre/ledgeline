<script lang="ts">
    // One transaction (WP-03): a fixed-height table row on desktop, a compact
    // card at narrow widths. Renders only the columns enabled in settings.
    // WP-08: `flags` (background-check problems for this txn) render as a
    // severity dot with a tooltip; `pulse` briefly highlights the row after
    // problems-drawer navigation.
    //
    // Editing (native engine only): clicking the description edits it inline
    // (PATCH {description}); a hover pencil opens the whole-transaction popup
    // (edit-all → PUT). Account/category inline editing lives in AccountsCell.
    import {descriptionPatch} from "$lib/api/editMapping";
    import {maxSeverity, type Problem} from "$lib/checks/engine";
    import type {Transaction} from "$lib/domain/types";
    import {editing} from "$lib/stores/editing.svelte";
    import type {ColumnConfig} from "$lib/stores/settings.svelte";
    import AccountsCell from "./AccountsCell.svelte";
    import AmountCell from "./AmountCell.svelte";
    import CommentIndicator from "./CommentIndicator.svelte";
    import StatusCell from "./StatusCell.svelte";
    import {txnModal} from "./edit/modalState.svelte";
    import {txnFlowAmounts} from "./rowModel";

    let {
        txn,
        columns,
        mode = "row",
        flags = [],
        pulse = false,
    }: {txn: Transaction; columns: ColumnConfig; mode?: "row" | "card"; flags?: Problem[]; pulse?: boolean} = $props();

    const amounts = $derived(txnFlowAmounts(txn));
    const flagTip = $derived(flags.map((problem) => problem.message).join("; "));
    const dotClass = $derived(maxSeverity(flags) === "error" ? "bg-error" : maxSeverity(flags) === "warning" ? "bg-warning" : "bg-info");
    const pulseClass = "bg-primary/15 animate-pulse";
    const canEdit = $derived(editing.canEdit);

    let editingDesc = $state(false);
    let draft = $state("");

    function startEditDesc(): void {
        if (!canEdit) return;
        draft = txn.description;
        editingDesc = true;
    }

    async function commitDesc(): Promise<void> {
        if (!editingDesc) return;
        const next = draft.trim();
        editingDesc = false;
        if (next === txn.description) return;
        const result = await editing.patch(txn.index, descriptionPatch(next));
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }

    function onDescKeydown(event: KeyboardEvent): void {
        if (event.key === "Enter") {
            event.preventDefault();
            void commitDesc();
        } else if (event.key === "Escape") {
            event.preventDefault();
            editingDesc = false;
        }
    }

    function descAutofocus(node: HTMLInputElement): void {
        node.focus();
        node.select();
    }
</script>

{#snippet descriptionContent()}
    <div class="group/desc flex min-w-0 items-center gap-1.5">
        {#if flags.length > 0}
            <span class="tooltip tooltip-right shrink-0" data-tip={flagTip}>
                <span class="block h-2 w-2 rounded-full {dotClass}" role="img" aria-label={flagTip}></span>
            </span>
        {/if}
        {#if editingDesc}
            <input
                type="text"
                class="input input-xs w-full min-w-0"
                bind:value={draft}
                aria-label="Edit description"
                autocomplete="off"
                onkeydown={onDescKeydown}
                onblur={commitDesc}
                use:descAutofocus
            />
        {:else if canEdit}
            <button type="button" class="min-w-0 truncate text-left" title="Click to edit · {txn.description}" onclick={startEditDesc}>
                {txn.description || "(no description)"}
            </button>
            <CommentIndicator {txn} />
            <button
                type="button"
                class="btn btn-ghost btn-xs btn-square shrink-0 opacity-0 group-hover/desc:opacity-100 focus:opacity-100"
                title="Edit whole transaction"
                aria-label="Edit whole transaction"
                onclick={() => txnModal.openEdit(txn)}
            >
                ✎
            </button>
        {:else}
            <span class="truncate" title={txn.description}>{txn.description}</span>
            <CommentIndicator {txn} />
        {/if}
    </div>
{/snippet}

{#if mode === "row"}
    <tr class="hover:bg-base-200/60 h-10 {pulse ? pulseClass : ''}">
        {#if columns.date}
            <td class="text-base-content/80 py-1 font-mono text-xs whitespace-nowrap">{txn.date}</td>
        {/if}
        {#if columns.status}
            <td class="py-1"><StatusCell {txn} /></td>
        {/if}
        {#if columns.description}
            <td class="max-w-0 py-1">{@render descriptionContent()}</td>
        {/if}
        {#if columns.accounts}
            <td class="max-w-0 py-1"><AccountsCell {txn} /></td>
        {/if}
        {#if columns.amount}
            <td class="py-1"><AmountCell {amounts} /></td>
        {/if}
    </tr>
{:else}
    <article class="card bg-base-200 mb-2 h-24 overflow-hidden {pulse ? pulseClass : ''}">
        <div class="card-body gap-1 p-3">
            <div class="flex items-center justify-between gap-2">
                <div class="flex min-w-0 items-center gap-1.5">
                    {#if columns.status}<StatusCell {txn} />{/if}
                    {@render descriptionContent()}
                </div>
                {#if columns.amount}<AmountCell {amounts} />{/if}
            </div>
            <div class="flex items-center justify-between gap-2">
                {#if columns.accounts}
                    <div class="min-w-0 overflow-hidden"><AccountsCell {txn} /></div>
                {/if}
                {#if columns.date}
                    <span class="text-base-content/60 shrink-0 font-mono text-xs whitespace-nowrap">{txn.date}</span>
                {/if}
            </div>
        </div>
    </article>
{/if}
