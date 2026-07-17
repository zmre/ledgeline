<script lang="ts">
    // Pinned totals footer (WP-03): MixedAmount totals from getFilteredTotals(),
    // transaction count, and the active period label.
    import type {MixedAmount} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import {formatTotals} from "./rowModel";

    let {
        totals,
        styles,
        count,
        period,
    }: {
        totals: MixedAmount;
        styles: ReadonlyMap<string, AmountStyle>;
        count: number;
        period: string;
    } = $props();

    const lines = $derived(formatTotals(totals, styles));
</script>

<footer class="bg-base-200 rounded-box flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-4 py-2">
    <div class="text-base-content/70 flex items-center gap-3 text-sm">
        <span class="whitespace-nowrap"><span class="text-base-content font-medium">{count}</span> {count === 1 ? "transaction" : "transactions"}</span>
        <span class="whitespace-nowrap">{period}</span>
    </div>
    <div
        class="flex flex-wrap items-center gap-x-3 font-mono text-sm tabular-nums"
        title="Balance of the selected accounts: asset/liability/equity carry their opening balance; income/expense show the period total"
    >
        {#if lines.length === 0}
            <span class="text-base-content/50">0 (balanced)</span>
        {:else}
            {#each lines as line (line.text)}
                <span class={["whitespace-nowrap", line.negative && "text-error"]}>{line.text}</span>
            {/each}
        {/if}
    </div>
</footer>
