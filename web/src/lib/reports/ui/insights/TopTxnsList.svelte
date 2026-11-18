<!-- Top Transactions (Box 10): the largest transactions in the current period by
     base-commodity money moved. A month of posted recognition entries is ONE
     line (plan 45 Q7): "Recognition, Nov · $41,250 (37 entries)", opening
     that month in the journal with its group row expanded. -->
<script lang="ts">
    import {openRecognitionMonth} from "#lib/journal/openJournal.js";
    import InfoTip from "#lib/components/InfoTip.svelte";
    import type {AmountStyle} from "#lib/domain/types.js";
    import type {TopTxn} from "#lib/reports/insightsTypes.js";
    import {fmt} from "./format";
    import {METRIC_HELP} from "./metricHelp";

    let {rows, base, styles, testid}: {rows: TopTxn[]; base: string; styles: ReadonlyMap<string, AmountStyle>; testid?: string} = $props();

    const SHORT_MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    /** `2026-11` → "Nov". */
    const shortMonth = (month: string): string => SHORT_MONTHS[Number(month.slice(5, 7)) - 1] ?? month;
</script>

<div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
    <div class="card-body gap-2 p-4">
        <div class="flex items-center gap-1">
            <div class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">Top Transactions</div>
            <InfoTip {...METRIC_HELP.toptxns} placement="bottom-start" />
        </div>
        {#if rows.length === 0}
            <div class="text-sm text-base-content/50">No transactions</div>
        {:else}
            <ul class="flex flex-col gap-1.5">
                {#each rows as row (row.index)}
                    <li class="flex items-center justify-between gap-3 text-sm">
                        {#if row.recognition !== null}
                            {@const recognition = row.recognition}
                            <!-- A button, not a link: the journal page may already be mounted
                                 (this panel lives on it), so the filter is set in the store. -->
                            <button
                                type="button"
                                class="min-w-0 link cursor-pointer text-left link-hover"
                                onclick={() => void openRecognitionMonth(recognition.month)}
                                title="Show {recognition.count} recognition entries in the journal"
                                data-testid="top-txn-recognition"
                            >
                                <span class="block truncate">Recognition, {shortMonth(recognition.month)}</span>
                                <span class="text-xs text-base-content/50"
                                    >{recognition.count} {recognition.count === 1 ? "entry" : "entries"} · {recognition.month}</span
                                >
                            </button>
                        {:else}
                            <span class="min-w-0">
                                <span class="block truncate" title={row.description}>{row.description || "(no description)"}</span>
                                <span class="text-xs text-base-content/50">{row.date}</span>
                            </span>
                        {/if}
                        <span class="font-mono whitespace-nowrap tabular-nums">{fmt(base, row.amount, styles)}</span>
                    </li>
                {/each}
            </ul>
        {/if}
    </div>
</div>
