<script lang="ts">
    // Right-aligned amounts (WP-03): one line per commodity, negatives in text-error.
    import {formatAmount} from "$lib/domain/money";
    import type {Amount} from "$lib/domain/types";

    let {amounts}: {amounts: Amount[]} = $props();
</script>

<div class="text-right font-mono text-sm tabular-nums">
    {#each amounts as amount (amount.commodity)}
        <div class={["whitespace-nowrap", amount.qty.m < 0n && "text-error"]}>{formatAmount(amount)}</div>
    {:else}
        <div class="text-base-content/40">&mdash;</div>
    {/each}
</div>
