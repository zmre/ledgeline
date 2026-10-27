<!-- Every account, per period, over the projection's columns: the Projections
     tab's `hledger balance -M --forecast` (plans/28, follow-ups 2026-10-26).

     The raw ledger the three statements are built from, so a reader can see
     what each account does before groups, sections and valuation shape it.
     Amounts are AS WRITTEN, in natural signs (revenue negative), one line per
     commodity: `hledger balance` without `-V`. The engine has already rolled
     the tree up to the page's depth; chain compression folds a parent into its
     only child where the two are the same in every column, as on the Reports
     tab's period tables.

     The columns are the statements' own template (statementGrid.ts): a
     fixed-layout table with the shared colgroup, the Actual/Projected band and
     the divider down the first projected column. Change carries a Total
     column, the sum of each row's periods; Balance is snapshots, which mean
     nothing summed, so it has none. -->
<script lang="ts">
    import type {MixedAmount} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {formatTotals} from "#lib/journal/rowModel.js";
    import type {PeriodReport} from "#lib/reports/types.js";
    import {compressPeriodRows} from "#lib/reports/ui/displayRows.js";
    import FigureColgroup from "#lib/reports/ui/FigureColgroup.svelte";
    import ProjectedHeadRow from "#lib/reports/ui/ProjectedHeadRow.svelte";
    import {projectedEdge} from "#lib/reports/ui/projectedColumns.js";
    import {statementMinWidth} from "#lib/reports/ui/statementGrid.js";

    let {
        report,
        styles,
        projectedFrom = null,
    }: {
        report: PeriodReport;
        styles: ReadonlyMap<string, AmountStyle>;
        projectedFrom?: number | null;
    } = $props();

    const rows = $derived(compressPeriodRows(report.rows));
    /** Change only: the report carries row totals exactly when it carries a grand total. */
    const total = $derived(report.grandTotal !== undefined);
    const periods = $derived(report.periods.length);
</script>

{#snippet amount(ma: MixedAmount)}
    {@const lines = formatTotals(ma, styles)}
    {#if lines.length === 0}
        <span class="text-base-content/30">0</span>
    {:else}
        {#each lines as line (line.text)}
            <div class={line.negative ? "text-error" : ""}>{line.text}</div>
        {/each}
    {/if}
{/snippet}

<div class="overflow-x-auto" data-testid="accounts-scroller">
    <section class="overflow-hidden rounded-box border border-base-content/10" style:min-width={statementMinWidth(periods, total)} data-testid="accounts-table">
        <table class="table table-fixed table-sm">
            <FigureColgroup {periods} {total} />
            <thead>
                <ProjectedHeadRow columns={periods} {projectedFrom} trailing={total ? 1 : 0} fixed />
                <tr class="text-[0.65rem] text-base-content/40">
                    <th class="truncate font-normal">Account</th>
                    {#each report.periods as period, i (period.key)}
                        <th class="{projectedEdge(i, projectedFrom)} text-right font-normal" title="{period.start} to {period.end}">{period.label}</th>
                    {/each}
                    {#if total}<th class="text-right font-normal">Total</th>{/if}
                </tr>
            </thead>
            <tbody>
                {#each rows as display (display.row.account)}
                    <tr class="hover:bg-base-200/60" data-account={display.row.account}>
                        <th class="truncate font-normal" title={display.row.account}>
                            <span class="whitespace-nowrap" style="padding-left: {display.indent}rem">{display.label}</span>
                        </th>
                        {#each display.row.values as value, i (i)}
                            <td class="{projectedEdge(i, projectedFrom)} text-right align-top font-mono whitespace-nowrap tabular-nums"
                                >{@render amount(value)}</td
                            >
                        {/each}
                        {#if total}
                            <td class="text-right align-top font-mono whitespace-nowrap tabular-nums" data-column="total">
                                {@render amount(display.row.total ?? new Map())}
                            </td>
                        {/if}
                    </tr>
                {:else}
                    <tr>
                        <th class="font-normal text-base-content/50">No account moves in these periods</th>
                        <td colspan={periods + (total ? 1 : 0)}></td>
                    </tr>
                {/each}
            </tbody>
            <tfoot>
                <tr class="border-t-2 border-base-content/20 bg-base-200 text-sm font-bold text-base-content">
                    <th class="font-bold">Total</th>
                    {#each report.totals as value, i (i)}
                        <td class="{projectedEdge(i, projectedFrom)} text-right align-top font-mono whitespace-nowrap tabular-nums">{@render amount(value)}</td>
                    {/each}
                    {#if total}
                        <td class="text-right align-top font-mono whitespace-nowrap tabular-nums">{@render amount(report.grandTotal ?? new Map())}</td>
                    {/if}
                </tr>
            </tfoot>
        </table>
    </section>
</div>
