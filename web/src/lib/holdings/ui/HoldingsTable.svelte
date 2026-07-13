<!-- Holdings details table (WP-10): one row per held stock, already sorted by
     the engine (market value desc, unpriced last). Sticky header
     (table-pin-rows), horizontal scroll at 375px, right-aligned numerics via
     the exact domain formatters (2dp display cap), em-dash for null cells,
     negatives in text-error, "inferred" badge when the price came from a cost
     annotation instead of a P directive. -->
<script lang="ts">
    import {toNumber, type Dec} from "$lib/domain/money";
    import type {Holding} from "$lib/holdings/types";
    import {EM_DASH, formatGainPct, formatShares} from "./view";

    let {holdings, format}: {holdings: Holding[]; format: (v: Dec) => string} = $props();
</script>

{#snippet money(v: Dec | null)}
    {#if v === null}
        <span class="text-base-content/40">{EM_DASH}</span>
    {:else}
        <span class={toNumber(v) < 0 ? "text-error" : ""}>{format(v)}</span>
    {/if}
{/snippet}

<div class="border-base-content/10 rounded-box max-h-[70vh] overflow-auto border">
    <table class="table-zebra table-pin-rows table-sm table" data-testid="holdings-table">
        <thead>
            <tr>
                <th>Name</th>
                <th>Symbol</th>
                <td class="text-right">Shares</td>
                <td class="text-right">Basis</td>
                <td class="text-right">Price</td>
                <td>Price date</td>
                <td class="text-right">Market value</td>
                <td class="text-right">Gain %</td>
            </tr>
        </thead>
        <tbody>
            {#each holdings as h (h.symbol)}
                <tr data-testid="holding-{h.symbol}">
                    <th class="font-normal whitespace-nowrap" title={h.accounts.join(", ")}>{h.name}</th>
                    <th class="font-medium">{h.symbol}</th>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums" data-testid="shares-{h.symbol}">{formatShares(h.shares)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(h.basis)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">
                        {#if h.price === null}
                            <span class="text-base-content/40">{EM_DASH}</span>
                        {:else}
                            {format(h.price.qty)}
                            {#if h.price.source === "cost"}
                                <span class="badge badge-ghost badge-xs align-middle" title="No P price directive — inferred from the latest cost annotation"
                                    >inferred</span
                                >
                            {/if}
                        {/if}
                    </td>
                    <td class="whitespace-nowrap">
                        {#if h.price === null}
                            <span class="text-base-content/40">{EM_DASH}</span>
                        {:else}
                            {h.price.date}
                        {/if}
                    </td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(h.marketValue)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">
                        <span class={h.gainPct === null ? "text-base-content/40" : h.gainPct < 0 ? "text-error" : ""}>{formatGainPct(h.gainPct)}</span>
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
</div>
