<!-- Top Transactions (Box 10): the largest transactions in the current period by
     base-commodity money moved. -->
<script lang="ts">
    import type {AmountStyle} from "$lib/domain/types";
    import type {TopTxn} from "$lib/reports/insightsTypes";
    import {fmt} from "./format";

    let {rows, base, styles, testid}: {rows: TopTxn[]; base: string; styles: ReadonlyMap<string, AmountStyle>; testid?: string} = $props();
</script>

<div class="card bg-base-200 border-base-content/5 border shadow-sm" data-testid={testid}>
    <div class="card-body gap-2 p-4">
        <div class="text-base-content/60 text-xs font-semibold tracking-wide uppercase">Top Transactions</div>
        {#if rows.length === 0}
            <div class="text-base-content/50 text-sm">No transactions</div>
        {:else}
            <ul class="flex flex-col gap-1.5">
                {#each rows as row (row.index)}
                    <li class="flex items-center justify-between gap-3 text-sm">
                        <span class="min-w-0">
                            <span class="block truncate" title={row.description}>{row.description || "(no description)"}</span>
                            <span class="text-base-content/50 text-xs">{row.date}</span>
                        </span>
                        <span class="font-mono whitespace-nowrap tabular-nums">{fmt(base, row.amount, styles)}</span>
                    </li>
                {/each}
            </ul>
        {/if}
    </div>
</div>
