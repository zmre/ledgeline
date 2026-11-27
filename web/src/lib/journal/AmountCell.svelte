<script lang="ts">
    // Right-aligned amounts (WP-03): one line per commodity, negatives in text-error-text.
    import {formatAmount} from "#lib/domain/money.js";
    import type {Amount} from "#lib/domain/types.js";

    let {amounts}: {amounts: Amount[]} = $props();
</script>

<div class="text-right font-mono text-sm tabular-nums">
    {#each amounts as amount (amount.commodity)}
        <div class={["whitespace-nowrap", amount.qty.m < 0n && "text-error-text"]}>{formatAmount(amount)}</div>
    {:else}
        <div class="text-muted">&mdash;</div>
    {/each}
</div>
