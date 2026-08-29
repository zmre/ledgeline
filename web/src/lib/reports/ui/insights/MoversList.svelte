<!-- Biggest Movers (Box 8): the top stocks by percent-move magnitude over the
     current period, with the windowed dollar gain alongside.

     A move is measured from the position's MARKET VALUE at the start of the
     period. When the journal has no `P` price directive before that date, the
     engine falls back to the purchase cost, which turns the "move" into the
     all-time gain since purchase — those rows are marked `startEstimated` and
     flagged here rather than silently passed off as a period return. -->
<script lang="ts">
    import type {AmountStyle} from "$lib/domain/types";
    import type {MoverRow} from "$lib/reports/insightsTypes";
    import {fmtSignedAmount, fmtSignedPct, signClass} from "./format";

    let {
        rows,
        base,
        styles,
        periodStart,
        testid,
    }: {rows: MoverRow[]; base: string; styles: ReadonlyMap<string, AmountStyle>; periodStart: string; testid?: string} = $props();

    const anyEstimated = $derived(rows.some((row) => row.startEstimated));
</script>

<div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
    <div class="card-body gap-2 p-4">
        <div class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">Biggest Movers</div>
        {#if rows.length === 0}
            <div class="text-sm text-base-content/50">No priced holdings</div>
        {:else}
            <ul class="flex flex-col gap-1.5">
                {#each rows as row (row.symbol)}
                    <li class="flex items-center justify-between gap-3 text-sm">
                        <span class="truncate font-medium" title={row.name}>
                            {row.symbol}{#if row.startEstimated}<span
                                    class="ml-1 text-warning"
                                    title="No market price before {periodStart} — measured from purchase cost">*</span
                                >{/if}
                        </span>
                        <span class="flex items-center gap-2 whitespace-nowrap">
                            <span class="font-mono text-base-content/70 tabular-nums">{fmtSignedAmount(row.gain, base, styles)}</span>
                            <span class="{signClass(row.gainPct)} w-16 text-right font-medium">{fmtSignedPct(row.gainPct)}</span>
                        </span>
                    </li>
                {/each}
            </ul>
            {#if anyEstimated}
                <div class="mt-1 text-xs text-warning/80" data-testid="movers-estimated-note">
                    * No market price before {periodStart} — measured from purchase cost, so this is closer to an all-time gain. Add <code>P</code> price directives
                    for true period returns.
                </div>
            {/if}
        {/if}
    </div>
</div>
