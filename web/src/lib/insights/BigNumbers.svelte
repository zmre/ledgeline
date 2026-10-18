<!-- Big numbers (WP-05): Income / Expenses / Net for the filtered period.
     Primary (most-used) commodity prominent; other commodities listed small.
     Every figure is the engine's exact Dec (`/api/journal/summary`, sign-adjusted
     per the journal's detected conventions), formatted with formatAmount — no
     float accumulation anywhere. -->
<script lang="ts">
    import {formatAmount, toNumber, type Dec} from "#lib/domain/money.js";
    import {DEFAULT_AMOUNT_STYLE, ZERO} from "#lib/format/amounts.js";
    import {signClass} from "#lib/format/sign.js";
    import type {CommoditySummary, JournalSummary} from "#lib/journalView/types.js";

    let {summary}: {summary: JournalSummary | null} = $props();

    /** An empty (or still-loading) view reads as zeros in dollars, exactly as an empty journal always has. */
    const EMPTY: CommoditySummary = {commodity: "$", style: DEFAULT_AMOUNT_STYLE, income: ZERO, expenses: ZERO, net: ZERO, categories: []};

    const primary = $derived(summary?.perCommodity[0] ?? EMPTY);
    const others = $derived(summary?.perCommodity.slice(1) ?? []);

    const fmt = (entry: CommoditySummary, qty: Dec): string => formatAmount({commodity: entry.commodity, qty, style: entry.style});

    interface Stat {
        label: string;
        value: string;
        valueClass: string;
        extras: string[];
    }

    const stats: Stat[] = $derived.by(() => {
        const extras = (pick: (entry: CommoditySummary) => Dec): string[] =>
            others.filter((entry) => pick(entry).m !== 0n).map((entry) => fmt(entry, pick(entry)));
        return [
            {label: "Income", value: fmt(primary, primary.income), valueClass: "", extras: extras((n) => n.income)},
            {label: "Expenses", value: fmt(primary, primary.expenses), valueClass: "", extras: extras((n) => n.expenses)},
            {label: "Net", value: fmt(primary, primary.net), valueClass: signClass(toNumber(primary.net)), extras: extras((n) => n.net)},
        ];
    });
</script>

<div class="stats w-full stats-vertical bg-base-200 shadow-none sm:stats-horizontal">
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
