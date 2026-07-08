<!-- Big numbers (WP-05): Income / Expenses / Net for the filtered period.
     Primary (most-used) commodity prominent; other commodities listed small.
     Values are exact Dec strings via formatAmount — no float accumulation. -->
<script lang="ts">
    import {formatAmount, toNumber, type Dec} from "$lib/domain/money";
    import type {Transaction} from "$lib/domain/types";
    import {bigNumbers, commoditiesInUse, styleFor, type AccountSelection} from "./series";

    let {txns, accounts}: {txns: Transaction[]; accounts?: AccountSelection} = $props();

    const commodities = $derived(commoditiesInUse(txns, accounts));
    const primary = $derived(commodities[0] ?? "$");
    const others = $derived(commodities.slice(1));

    const fmt = (commodity: string, qty: Dec): string => formatAmount({commodity, qty, style: styleFor(txns, commodity)});

    interface Stat {
        label: string;
        value: string;
        valueClass: string;
        extras: string[];
    }

    const stats: Stat[] = $derived.by(() => {
        const primaryNums = bigNumbers(txns, primary, accounts);
        const otherNums = others.map((c) => ({commodity: c, nums: bigNumbers(txns, c, accounts)}));
        const extras = (pick: (nums: {income: Dec; expenses: Dec; net: Dec}) => Dec): string[] =>
            otherNums.filter(({nums}) => pick(nums).m !== 0n).map(({commodity, nums}) => fmt(commodity, pick(nums)));
        return [
            {label: "Income", value: fmt(primary, primaryNums.income), valueClass: "", extras: extras((n) => n.income)},
            {label: "Expenses", value: fmt(primary, primaryNums.expenses), valueClass: "", extras: extras((n) => n.expenses)},
            {
                label: "Net",
                value: fmt(primary, primaryNums.net),
                valueClass: toNumber(primaryNums.net) < 0 ? "text-error" : "text-success",
                extras: extras((n) => n.net),
            },
        ];
    });
</script>

<div class="stats stats-vertical sm:stats-horizontal bg-base-200 w-full shadow-none">
    {#each stats as stat (stat.label)}
        <div class="stat px-4 py-3">
            <div class="stat-title text-xs">{stat.label}</div>
            <div class="stat-value text-2xl md:text-3xl {stat.valueClass}">{stat.value}</div>
            {#if stat.extras.length > 0}
                <div class="stat-desc">{stat.extras.join(" · ")}</div>
            {/if}
        </div>
    {/each}
</div>
