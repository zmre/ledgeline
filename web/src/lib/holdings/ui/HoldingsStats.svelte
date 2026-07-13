<!-- Holdings stat tiles (WP-10, style of WP-05 BigNumbers): Market value |
     Cost basis | Unrealized gain $ | Unrealized gain %. Totals are null when
     ANY in-scope holding is tainted or unpriced (the engine's honest-totals
     rule) — rendered as an em-dash; the inline warning explains why. -->
<script lang="ts">
    import {toNumber, type Dec} from "$lib/domain/money";
    import type {HoldingsReport} from "$lib/holdings/types";
    import {EM_DASH, formatGainPct} from "./view";

    let {totals, format}: {totals: HoldingsReport["totals"]; format: (v: Dec) => string} = $props();

    const signClass = (negative: boolean): string => (negative ? "text-error" : "text-success");

    interface Stat {
        label: string;
        value: string;
        valueClass: string;
    }
    const stats: Stat[] = $derived([
        {label: "Market value", value: format(totals.marketValue), valueClass: ""},
        {label: "Cost basis", value: totals.basis === null ? EM_DASH : format(totals.basis), valueClass: ""},
        {
            label: "Unrealized gain",
            value: totals.gain === null ? EM_DASH : format(totals.gain),
            valueClass: totals.gain === null ? "" : signClass(toNumber(totals.gain) < 0),
        },
        {
            label: "Unrealized gain %",
            value: formatGainPct(totals.gainPct),
            valueClass: totals.gainPct === null ? "" : signClass(totals.gainPct < 0),
        },
    ]);
</script>

<div class="stats stats-vertical sm:stats-horizontal bg-base-200 w-full shadow-none" data-testid="holdings-stats">
    {#each stats as stat (stat.label)}
        <div class="stat px-4 py-3">
            <div class="stat-title text-xs">{stat.label}</div>
            <div class="stat-value text-2xl md:text-3xl {stat.valueClass}">{stat.value}</div>
        </div>
    {/each}
</div>
