<!-- Spreadsheet-style report table (WP-07): sticky header row + sticky account
     column (daisyUI table-pin-rows/pin-cols), zebra rows, depth-indented
     accounts with single-child chain compression, right-aligned exact amounts
     (negatives in text-error), emphasized subtotal/total rows. Handles both
     SectionedReport (bs/is — engine rows arrive pre-sign-flipped, rendered
     as-is) and PeriodReport (cf/nw — one column per bucket, horizontal scroll
     on mobile). -->
<script lang="ts">
    import type {MixedAmount} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import {formatTotals} from "$lib/journal/rowModel";
    import {bucketLabel} from "$lib/reports/periods";
    import type {PeriodReport, SectionedReport} from "$lib/reports/types";
    import {compressPeriodRows, compressSectionRows} from "./displayRows";

    let {report, styles}: {report: SectionedReport | PeriodReport; styles: ReadonlyMap<string, AmountStyle>} = $props();

    const sectioned = $derived("sections" in report ? report : null);
    const period = $derived("sections" in report ? null : report);
</script>

{#snippet amount(ma: MixedAmount)}
    {@const lines = formatTotals(ma, styles)}
    {#if lines.length === 0}
        <span class="text-base-content/40">0</span>
    {:else}
        {#each lines as line (line.text)}
            <div class={line.negative ? "text-error" : ""}>{line.text}</div>
        {/each}
    {/if}
{/snippet}

<div class="border-base-content/10 rounded-box max-h-[70vh] overflow-auto border">
    <table class="table-zebra table-pin-rows table-pin-cols table-sm table">
        {#if sectioned !== null}
            <thead>
                <tr>
                    <th class="w-full">Account</th>
                    <td class="text-right">Amount</td>
                </tr>
            </thead>
            {#each sectioned.sections as section (section.title)}
                <tbody>
                    <tr>
                        <th class="text-base-content/60 pt-3 text-xs font-semibold tracking-wide uppercase">{section.title}</th>
                        <td></td>
                    </tr>
                    {#each compressSectionRows(section.rows) as display (display.row.account)}
                        <tr>
                            <th class="font-normal whitespace-nowrap">
                                <span style="padding-left: {display.indent}rem">{display.label}</span>
                            </th>
                            <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(display.row.inclusive)}</td>
                        </tr>
                    {/each}
                    <tr class="border-base-content/20 border-t font-semibold">
                        <th>Total {section.title}</th>
                        <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(section.total)}</td>
                    </tr>
                </tbody>
            {/each}
            <tbody>
                <tr class="border-base-content/40 border-t-2 text-base font-bold">
                    <th>Net</th>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(sectioned.grandTotal)}</td>
                </tr>
            </tbody>
        {:else if period !== null}
            <thead>
                <tr>
                    <th>Account</th>
                    {#each period.buckets as bucket (bucket)}
                        <td class="min-w-24 text-right whitespace-nowrap">{bucketLabel(bucket)}</td>
                    {/each}
                </tr>
            </thead>
            <tbody>
                {#each compressPeriodRows(period.rows) as display (display.row.account)}
                    <tr>
                        <th class="font-normal whitespace-nowrap">
                            <span style="padding-left: {display.indent}rem">{display.label}</span>
                        </th>
                        {#each display.row.values as value, i (period.buckets[i])}
                            <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(value)}</td>
                        {/each}
                    </tr>
                {/each}
                {#if period.rows.length === 0}
                    <tr>
                        <th class="text-base-content/50 font-normal">No matching accounts</th>
                        {#each period.buckets as bucket (bucket)}
                            <td></td>
                        {/each}
                    </tr>
                {/if}
                <tr class="border-base-content/40 border-t-2 font-bold">
                    <th>Net</th>
                    {#each period.totals as total, i (period.buckets[i])}
                        <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(total)}</td>
                    {/each}
                </tr>
            </tbody>
        {/if}
    </table>
</div>
