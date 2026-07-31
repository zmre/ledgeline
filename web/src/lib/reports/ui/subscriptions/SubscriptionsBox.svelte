<!-- One cadence's recurring charges (annual or monthly).

     The header carries BOTH totals — per month and per year — because they
     answer different questions ("what is this costing me right now" vs "what
     would cancelling save"), and neither is obvious from the other once annual
     and monthly charges sit side by side. Both derive from the annualized cost,
     so the arithmetic stays consistent across cadences.

     Rows link into the journal, filtered to that payee over all dates, so a
     suspicious charge is one click from its actual transactions. Monthly rows
     also show the yearly total beside the price — that is the number worth
     reacting to. Both lists are ordered by yearly cost, so the most valuable
     thing to cancel is always first. -->
<script lang="ts">
    import {resolve} from "$app/paths";
    import {add, type Dec} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import type {Cadence, Subscription} from "$lib/reports/insightsTypes";
    import {fmt, monthlyAverage} from "../insights/format";

    let {
        title,
        cadence,
        rows,
        base,
        styles,
        lookbackStart,
        testid,
    }: {
        title: string;
        cadence: Cadence;
        rows: Subscription[];
        base: string;
        styles: ReadonlyMap<string, AmountStyle>;
        lookbackStart: string | null;
        testid?: string;
    } = $props();

    // Exact yearly cost of these charges (never float-accumulated); the monthly
    // figure is that spread across the year, which for monthly subscriptions is
    // exactly the sum of their individual prices.
    const perYear = $derived(rows.reduce<Dec>((sum, row) => add(sum, row.annualizedCost), {m: 0n, p: 0}));
    const perMonth = $derived(monthlyAverage(perYear, 12));

    /** The journal, filtered to this payee across all dates. */
    function journalLink(payee: string): string {
        return `${resolve("/")}?preset=all&q=${encodeURIComponent(payee)}`;
    }
</script>

<div class="card bg-base-200 border-base-content/5 border shadow-sm" data-testid={testid}>
    <div class="card-body gap-2 p-4">
        <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <span class="text-base-content/60 text-xs font-semibold tracking-wide uppercase">{title}</span>
            {#if rows.length > 0}
                <span class="font-mono text-xs tabular-nums" data-testid={testid ? `${testid}-total` : undefined}>
                    <span class="text-base-content/80">{fmt(base, perMonth, styles)}/mo</span>
                    <span class="text-base-content/40 mx-1">·</span>
                    <span class="text-base-content/80">{fmt(base, perYear, styles)}/yr</span>
                </span>
            {/if}
        </div>

        {#if rows.length === 0}
            <div class="text-base-content/50 text-sm">
                {lookbackStart === null ? "No recurring charges found." : `No recurring charges found since ${lookbackStart}.`}
            </div>
        {:else}
            <ul class="flex flex-col">
                {#each rows as row (`${row.payee}@${row.typicalAmount.m}`)}
                    <li>
                        <!-- `journalLink` already builds its path through
                             `resolve()`; the rule cannot see through the call. -->
                        <!-- eslint-disable svelte/no-navigation-without-resolve -->
                        <a
                            href={journalLink(row.payee)}
                            class="hover:bg-base-300/60 -mx-2 flex items-center justify-between gap-3 rounded px-2 py-1.5 text-sm transition-colors"
                            title="{row.occurrences} charges since {row.firstSeen} — show them in the journal"
                        >
                            <span class="min-w-0">
                                <span class="block truncate font-medium">
                                    {row.payee}{#if row.manual}<span
                                            class="badge badge-ghost badge-xs ml-1.5 align-middle"
                                            title="Added by a subscription:true tag">tagged</span
                                        >{/if}
                                </span>
                                <span class="text-base-content/50 text-xs">next {row.nextExpected}</span>
                            </span>
                            <span class="text-right whitespace-nowrap">
                                <span class="block font-mono tabular-nums">{fmt(base, row.typicalAmount, styles)}</span>
                                {#if cadence === "monthly"}
                                    <span class="text-base-content/50 font-mono text-xs tabular-nums">{fmt(base, row.annualizedCost, styles)}/yr</span>
                                {/if}
                            </span>
                        </a>
                        <!-- eslint-enable svelte/no-navigation-without-resolve -->
                    </li>
                {/each}
            </ul>
        {/if}
    </div>
</div>
