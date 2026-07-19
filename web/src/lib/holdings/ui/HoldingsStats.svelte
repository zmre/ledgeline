<!-- Holdings stat tiles (WP-10, style of WP-05 BigNumbers): Market value |
     Cost basis | Unrealized gain $ | Unrealized gain %. Cost basis & gain are
     PARTIAL totals — summed over the holdings that have the inputs, so a single
     cost-less/unpriced row no longer blanks the portfolio. A muted note names how
     many shown holdings were left out; a tile shows an em-dash only when its total
     is genuinely null (every shown holding excluded). -->
<script lang="ts">
    import {toNumber, type Dec} from "$lib/domain/money";
    import type {GainPeriod, HoldingsReport} from "$lib/holdings/types";
    import {gainWindowSuffix} from "./gainPeriod";
    import {EM_DASH, formatGainPct, untotaledBasisCount} from "./view";

    let {totals, holdings, format, gainPeriod = "all"}: {totals: HoldingsReport["totals"]; holdings: HoldingsReport["holdings"]; format: (v: Dec) => string; gainPeriod?: GainPeriod} = $props();

    // Displayed holdings excluded from the partial basis/gain totals (no recorded basis) → the muted note; 0 hides it.
    const excludedCount = $derived(untotaledBasisCount(holdings));
    const excludedNote = $derived(excludedCount > 0 ? `Cost basis & gain exclude ${excludedCount} holding${excludedCount === 1 ? "" : "s"} with no recorded basis.` : null);

    const signClass = (negative: boolean): string => (negative ? "text-error" : "text-success");
    // "Unrealized" only reads true for the all-time window; a windowed gain gets the window tag instead.
    const gainSuffix = $derived(gainWindowSuffix(gainPeriod));
    const gainLabel = $derived(gainPeriod === "all" ? "Unrealized gain" : `Gain${gainSuffix}`);
    const gainPctLabel = $derived(gainPeriod === "all" ? "Unrealized gain %" : `Gain %${gainSuffix}`);

    interface Stat {
        label: string;
        value: string;
        valueClass: string;
    }
    const stats: Stat[] = $derived([
        {label: "Market value", value: format(totals.marketValue), valueClass: ""},
        {label: "Cost basis", value: totals.basis === null ? EM_DASH : format(totals.basis), valueClass: ""},
        {
            label: gainLabel,
            value: totals.gain === null ? EM_DASH : format(totals.gain),
            valueClass: totals.gain === null ? "" : signClass(toNumber(totals.gain) < 0),
        },
        {
            label: gainPctLabel,
            value: formatGainPct(totals.gainPct),
            valueClass: totals.gainPct === null ? "" : signClass(totals.gainPct < 0),
        },
    ]);
</script>

<div class="flex flex-col gap-1" data-testid="holdings-stats-block">
    <div class="stats stats-vertical sm:stats-horizontal bg-base-200 w-full shadow-none" data-testid="holdings-stats">
        {#each stats as stat (stat.label)}
            <div class="stat px-4 py-3">
                <div class="stat-title text-xs">{stat.label}</div>
                <div class="stat-value text-2xl md:text-3xl {stat.valueClass}">{stat.value}</div>
            </div>
        {/each}
    </div>
    {#if excludedNote !== null}
        <p class="text-base-content/60 px-4 text-xs" data-testid="holdings-basis-note">{excludedNote}</p>
    {/if}
</div>
