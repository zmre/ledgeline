<!-- Top gainers / losers (WP-10): two compact lists (≤5 each) of symbol,
     gain %, gain $ — green/red per sign. Each list holds only holdings with
     that gain sign, so an empty list is hidden individually; the whole
     component is hidden when fewer than two holdings are priced (a
     single-entry "top 5" is noise, per plans/10). -->
<script lang="ts">
    import {toNumber, type Dec} from "$lib/domain/money";
    import type {Holding, HoldingsReport} from "$lib/holdings/types";
    import {formatGainPct} from "./view";

    let {report, format}: {report: HoldingsReport; format: (v: Dec) => string} = $props();

    const pricedCount = $derived(report.holdings.filter((h) => h.marketValue !== null).length);
    const visible = $derived(pricedCount >= 2 && (report.topGainers.length > 0 || report.topLosers.length > 0));
</script>

{#snippet list(title: string, entries: Holding[], testid: string)}
    <div class="min-w-0 flex-1" data-testid={testid}>
        <h3 class="text-base-content/60 mb-1 text-xs font-semibold tracking-wide uppercase">{title}</h3>
        <ul class="flex flex-col gap-1">
            {#each entries as h (h.symbol)}
                {@const negative = (h.gainPct ?? 0) < 0}
                <li class="flex items-baseline gap-2 text-sm">
                    <span class="font-medium">{h.symbol}</span>
                    <span class={negative ? "text-error" : "text-success"}>{formatGainPct(h.gainPct)}</span>
                    {#if h.gain !== null}
                        <span class="ml-auto font-mono text-xs tabular-nums {toNumber(h.gain) < 0 ? 'text-error' : 'text-success'}">{format(h.gain)}</span>
                    {/if}
                </li>
            {/each}
        </ul>
    </div>
{/snippet}

{#if visible}
    <div class="flex flex-col gap-4 sm:flex-row" data-testid="gainers-losers">
        {#if report.topGainers.length > 0}
            {@render list("Top gainers", report.topGainers, "top-gainers")}
        {/if}
        {#if report.topLosers.length > 0}
            {@render list("Top losers", report.topLosers, "top-losers")}
        {/if}
    </div>
{/if}
