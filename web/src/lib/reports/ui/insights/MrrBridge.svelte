<!-- The MRR bridge for the current half (plans/37): start MRR, each category
     that moved it (with its customer count), Other when non-zero, end MRR.

     A category with customers behind it is a button: it opens that category's
     largest customers, each a link into the journal searching for the
     customer over all dates. The link stays a real `<a href>` (middle-click,
     open in a new tab), built through `journalSearch` like Subscriptions'.

     The journal search is a substring match over a transaction's text, not
     hledger's query language, so the link searches for the customer's NAME:
     that finds the `customer:` lines however the tag is spaced, and the
     payee-inferred customers, whose only trace is the description. -->
<script lang="ts">
    import {resolve} from "$app/paths";
    import InfoTip from "#lib/components/InfoTip.svelte";
    import type {Dec} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {journalSearch} from "#lib/filters/journalTarget.js";
    import type {Bridge, BridgeCategory, BridgeCustomers} from "#lib/reports/saasTypes.js";
    import {fmt, fmtSignedAmount} from "./format";
    import {METRIC_HELP} from "./metricHelp";
    import {bridgeModel} from "./startup";

    let {
        bridge,
        end,
        customers,
        base,
        styles,
        periodLabel,
        testid = "startup-bridge",
    }: {
        bridge: Bridge;
        /** MRR at the half's last complete month. */
        end: Dec;
        customers: BridgeCustomers | null;
        base: string;
        styles: ReadonlyMap<string, AmountStyle>;
        /** e.g. "2025-07 → 2026-06". */
        periodLabel: string;
        testid?: string;
    } = $props();

    const model = $derived(bridgeModel(bridge, end));
    let open = $state<BridgeCategory | null>(null);

    function journalLink(name: string): string {
        return `${resolve("/")}?${journalSearch({query: name, preset: "all"})}`;
    }

    function toggle(category: BridgeCategory): void {
        open = open === category ? null : category;
    }
</script>

<div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
    <div class="card-body gap-2 p-4">
        <div class="flex flex-wrap items-baseline justify-between gap-x-3">
            <span class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">MRR bridge</span>
            <span class="text-xs text-base-content/50">{periodLabel}</span>
        </div>
        <ul class="flex flex-col text-sm">
            <li class="flex justify-between gap-3 py-1 font-medium" data-testid="{testid}-start">
                <span>Start MRR</span>
                <span class="font-mono tabular-nums">{fmt(base, model.start, styles)}</span>
            </li>
            {#each model.lines as line (line.category)}
                {@const category = line.category}
                {@const rows = category === "other" || customers === null ? [] : customers[category]}
                <li data-testid="{testid}-{category}">
                    {#if category !== "other" && rows.length > 0}
                        <!-- The toggle covers the label only: the (i) beside it is a
                             button of its own, and buttons don't nest. -->
                        <div class="-mx-2 flex items-center justify-between gap-3 rounded px-2 py-1 transition-colors hover:bg-base-300/60">
                            <span class="flex items-center gap-1">
                                <button type="button" class="cursor-pointer text-left" aria-expanded={open === category} onclick={() => toggle(category)}>
                                    <span class="mr-1 inline-block w-3 text-base-content/50">{open === category ? "▾" : "▸"}</span
                                    >{line.label}{#if line.count !== null}<span class="ml-1 text-base-content/50">({line.count})</span>{/if}
                                </button>
                                <InfoTip {...METRIC_HELP[`bridge-${category}`]} placement="bottom-start" />
                            </span>
                            <span class="font-mono tabular-nums">{fmtSignedAmount(line.amount, base, styles)}</span>
                        </div>
                        {#if open === category}
                            <ul class="mb-1 ml-4 flex flex-col border-l border-base-content/10 pl-2" data-testid="{testid}-{category}-customers">
                                {#each rows as row (row.customer)}
                                    <li>
                                        <!-- `journalLink` builds its path through `resolve()`; the rule cannot see through the call. -->
                                        <!-- eslint-disable svelte/no-navigation-without-resolve -->
                                        <a
                                            href={journalLink(row.name)}
                                            class="-mx-2 flex items-center justify-between gap-3 rounded px-2 py-0.5 text-xs transition-colors hover:bg-base-300/60"
                                            title="Show {row.name}'s transactions in the journal"
                                            data-testid="{testid}-customer"
                                        >
                                            <span class="truncate">
                                                {row.name}{#if row.inferred}<span class="ml-0.5 text-warning" title="Inferred from the payee">*</span>{/if}
                                                {#if row.segment !== null}<span class="ml-1 text-base-content/50">{row.segment}</span>{/if}
                                            </span>
                                            <span class="font-mono tabular-nums">{fmt(base, row.amount, styles)}</span>
                                        </a>
                                        <!-- eslint-enable svelte/no-navigation-without-resolve -->
                                    </li>
                                {/each}
                            </ul>
                        {/if}
                    {:else}
                        <div class="flex items-center justify-between gap-3 py-1">
                            <span class="flex items-center gap-1 pl-4 {category === 'other' ? 'text-base-content/70' : ''}"
                                ><span
                                    >{line.label}{#if line.count !== null}<span class="ml-1 text-base-content/50">({line.count})</span>{/if}</span
                                ><InfoTip {...METRIC_HELP[`bridge-${category}`]} placement="bottom-start" /></span
                            >
                            <span class="font-mono tabular-nums">{fmtSignedAmount(line.amount, base, styles)}</span>
                        </div>
                    {/if}
                </li>
            {/each}
            <li class="flex justify-between gap-3 border-t border-base-content/10 py-1 font-medium" data-testid="{testid}-end">
                <span>End MRR</span>
                <span class="font-mono tabular-nums">{fmt(base, model.end, styles)}</span>
            </li>
        </ul>
    </div>
</div>
