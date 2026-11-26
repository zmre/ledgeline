<!-- Top Payees (Box 11): who took the most spending this period, each against
     the previous period. Rows are real links into the journal, filtered to the
     payee over the current period, so middle-click and Tab+Enter both work. A
     payee with no previous spending says "new" rather than a fake +100%. -->
<script lang="ts">
    import {resolve} from "$app/paths";
    import InfoTip from "#ui/components/InfoTip.svelte";
    import {journalSearch} from "#lib/filters/journalTarget.js";
    import type {AmountStyle, ISODate} from "#lib/domain/types.js";
    import type {PayeeRow} from "#lib/reports/insightsTypes.js";
    import {fmt, payeeChange} from "./format";
    import {METRIC_HELP} from "./metricHelp";

    let {
        rows,
        base,
        styles,
        from,
        to,
        testid,
    }: {
        rows: PayeeRow[];
        base: string;
        styles: ReadonlyMap<string, AmountStyle>;
        /** The current period, which each row's journal link is filtered to. */
        from: ISODate;
        to: ISODate;
        testid?: string;
    } = $props();

    function journalLink(payee: string): string {
        return `${resolve("/")}?${journalSearch({query: payee, from, to})}`;
    }
</script>

<div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
    <div class="card-body gap-2 p-4">
        <div class="flex items-center gap-1">
            <div class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">Top Payees</div>
            <InfoTip {...METRIC_HELP.toppayees} placement="bottom-start" />
        </div>
        {#if rows.length === 0}
            <div class="text-sm text-base-content/50">No spending this period</div>
        {:else}
            <ul class="flex flex-col">
                {#each rows as row (row.payee)}
                    {@const change = payeeChange(row, base, styles)}
                    <li data-testid="top-payee-row">
                        <!-- `journalLink` already builds its path through
                             `resolve()`; the rule cannot see through the call. -->
                        <!-- eslint-disable svelte/no-navigation-without-resolve -->
                        <a
                            href={journalLink(row.payee)}
                            class="-mx-2 flex items-center justify-between gap-3 rounded px-2 py-1.5 text-sm transition-colors hover:bg-base-300/60 focus-visible:bg-base-300/60"
                            title="Show {row.payee} in the journal for this period"
                        >
                            <span class="min-w-0">
                                <span class="block truncate font-medium">{row.payee}</span>
                                <span class="text-xs text-base-content/50">
                                    {row.previous === null ? "new" : `prev ${fmt(base, row.previous, styles)}`}
                                </span>
                            </span>
                            <span class="text-right whitespace-nowrap">
                                <span class="block font-mono tabular-nums">{fmt(base, row.current, styles)}</span>
                                {#if change !== null}
                                    <span class="font-mono text-xs tabular-nums {change.klass}">{change.arrow} {change.text}</span>
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
