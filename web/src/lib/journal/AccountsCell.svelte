<script lang="ts">
    // From→to account chips (WP-03): `source → dest` for simple two-sided txns,
    // degrading to a wrapped account list for N-way splits. Long names truncate
    // with the full name in a hover tooltip (native title).
    import type {Transaction} from "$lib/domain/types";
    import {accountFlow} from "./rowModel";

    let {txn}: {txn: Transaction} = $props();

    const flow = $derived(accountFlow(txn));
</script>

{#if flow.kind === "flow"}
    <div class="flex min-w-0 items-center gap-1">
        <span class="badge badge-ghost badge-sm min-w-0 max-w-[45%]" title={flow.source}><span class="truncate">{flow.source}</span></span>
        <span class="text-base-content/50 shrink-0" aria-label="to">&rarr;</span>
        <span class="badge badge-ghost badge-sm min-w-0 max-w-[45%]" title={flow.dest}><span class="truncate">{flow.dest}</span></span>
    </div>
{:else}
    <div class="flex min-w-0 flex-wrap gap-1">
        {#each flow.accounts as account (account)}
            <span class="badge badge-ghost badge-sm min-w-0 max-w-44" title={account}><span class="truncate">{account}</span></span>
        {/each}
    </div>
{/if}
