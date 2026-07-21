<script lang="ts">
    // Journal footer (WP-03): filtered transaction count and active period label
    // on the left; the right-aligned "Visible Journal Total" on the right — the
    // per-commodity net (income − expenses) of the filtered view (see visibleNet
    // in insights/series.ts). Expenses read negative, revenue positive, and equal
    // refunds offset, so filtering by e.g. a store shows the net spent there.
    import {formatAmount} from "$lib/domain/money";
    import type {Amount} from "$lib/domain/types";

    let {
        count,
        period,
        total,
    }: {
        count: number;
        period: string;
        total: Amount[];
    } = $props();
</script>

<footer class="bg-base-200 rounded-box flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-4 py-2">
    <div class="text-base-content/70 flex items-center gap-3 text-sm">
        <span class="whitespace-nowrap"><span class="text-base-content font-medium">{count}</span> {count === 1 ? "transaction" : "transactions"}</span>
        <span class="whitespace-nowrap">{period}</span>
    </div>
    <div class="ml-auto flex items-baseline gap-2 text-sm">
        <span class="text-base-content/60 whitespace-nowrap">Visible Journal Total</span>
        <span class="text-right font-mono font-semibold tabular-nums">
            {#each total as amount (amount.commodity)}
                <span class={["block whitespace-nowrap", amount.qty.m < 0n && "text-error"]}>{formatAmount(amount)}</span>
            {:else}
                <span class="text-base-content/40 block">&mdash;</span>
            {/each}
        </span>
    </div>
</footer>
