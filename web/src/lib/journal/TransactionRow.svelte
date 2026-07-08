<script lang="ts">
    // One transaction (WP-03): a fixed-height table row on desktop, a compact
    // card at narrow widths. Renders only the columns enabled in settings.
    import type {Transaction} from "$lib/domain/types";
    import type {ColumnConfig} from "$lib/stores/settings.svelte";
    import AccountsCell from "./AccountsCell.svelte";
    import AmountCell from "./AmountCell.svelte";
    import CommentIndicator from "./CommentIndicator.svelte";
    import StatusBadge from "./StatusBadge.svelte";
    import {txnFlowAmounts} from "./rowModel";

    let {txn, columns, mode = "row"}: {txn: Transaction; columns: ColumnConfig; mode?: "row" | "card"} = $props();

    const amounts = $derived(txnFlowAmounts(txn));
</script>

{#if mode === "row"}
    <tr class="hover:bg-base-200/60 h-10">
        {#if columns.date}
            <td class="text-base-content/80 py-1 font-mono text-xs whitespace-nowrap">{txn.date}</td>
        {/if}
        {#if columns.status}
            <td class="py-1"><StatusBadge status={txn.status} /></td>
        {/if}
        {#if columns.description}
            <td class="max-w-0 py-1">
                <div class="flex items-center gap-1.5">
                    <span class="truncate" title={txn.description}>{txn.description}</span>
                    <CommentIndicator {txn} />
                </div>
            </td>
        {/if}
        {#if columns.accounts}
            <td class="max-w-0 py-1"><AccountsCell {txn} /></td>
        {/if}
        {#if columns.amount}
            <td class="py-1"><AmountCell {amounts} /></td>
        {/if}
    </tr>
{:else}
    <article class="card bg-base-200 mb-2 h-24 overflow-hidden">
        <div class="card-body gap-1 p-3">
            <div class="flex items-center justify-between gap-2">
                <div class="flex min-w-0 items-center gap-1.5">
                    {#if columns.status}<StatusBadge status={txn.status} />{/if}
                    <span class="truncate text-sm font-medium" title={txn.description}>{txn.description}</span>
                    <CommentIndicator {txn} />
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
