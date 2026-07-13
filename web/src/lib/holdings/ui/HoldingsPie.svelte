<!-- Holdings pie (WP-10): slice per symbol by toNumber(marketValue), unpriced
     holdings excluded (the inline warning covers them), tail folded into one
     "(other)" bucket.
     - colors: the dataviz reference DARK categorical palette, all 8 slots in
       fixed order (app theme is dark-only) + muted gray for the folded tail.
       Validated with the dataviz skill validator against the daisyUI dark
       surface: lightness band PASS, chroma PASS, contrast >=3:1 PASS, worst
       adjacent CVD dE 10.3 (floor band) — mitigated per the skill by the
       always-visible legend (symbol + % share, identity never color-alone),
       pad-angle gaps between slices, and full tooltips.
     - a 9th holding never gets a generated hue: it folds into "(other)"
       (dataviz non-negotiable), which is why the named-slice cap is 8. -->
<script lang="ts">
    import {PieChart, Tooltip} from "layerchart";
    import type {Dec} from "$lib/domain/money";
    import type {Holding} from "$lib/holdings/types";
    import {pieSlices, PIE_OTHER, type PieSlice} from "./view";

    let {holdings, format}: {holdings: Holding[]; format: (v: Dec) => string} = $props();

    // Dark-mode categorical slots 1..8 from the dataviz reference palette, fixed order, never cycled.
    const PALETTE = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767", "#d55181", "#d95926"];
    const OTHER_COLOR = "#898781"; // muted — the folded tail is context, not a series identity

    const slices = $derived(pieSlices(holdings, format, PALETTE.length));
    const colorOf = (slice: PieSlice, i: number): string => (slice.symbol === PIE_OTHER ? OTHER_COLOR : PALETTE[i]);
</script>

{#if slices.length === 0}
    <p class="text-base-content/60 py-10 text-center text-sm">No priced holdings to chart.</p>
{:else}
    <div class="h-56 w-full sm:h-64" data-testid="holdings-pie">
        <PieChart data={slices} key="symbol" label="symbol" value={(d) => d.value} cRange={slices.map(colorOf)} padAngle={0.02}>
            {#snippet tooltip()}
                <Tooltip.Root>
                    {#snippet children({data})}
                        {@const d = data as PieSlice}
                        <div class="flex items-center gap-2 text-xs">
                            <span class="inline-block h-2 w-2 rounded-full" style="background:{colorOf(d, slices.indexOf(d))}"></span>
                            <span class="text-base-content/70">{d.name}</span>
                            <span class="font-semibold">{d.formatted}</span>
                        </div>
                    {/snippet}
                </Tooltip.Root>
            {/snippet}
        </PieChart>
    </div>
    <!-- always-visible legend: symbol + % share (identity is never color-alone) -->
    <ul class="text-base-content/70 mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs" data-testid="holdings-pie-legend">
        {#each slices as slice, i (slice.symbol)}
            <li class="flex items-center gap-1" title="{slice.name} — {slice.formatted}">
                <span class="inline-block h-2 w-2 rounded-full" style="background:{colorOf(slice, i)}"></span>
                {slice.symbol}
                <span class="text-base-content/50">{slice.share.toFixed(1)}%</span>
            </li>
        {/each}
    </ul>
{/if}
