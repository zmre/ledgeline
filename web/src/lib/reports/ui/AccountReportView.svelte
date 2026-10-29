<!-- Every account, per period, each section read the way that kind of account
     is read (2026-10-28) — Reports › Accounts and the Projections Accounts tab.

     Assets and liabilities show each period's closing BALANCE, and their last
     column is the Δ over the whole window, every figure in it prefixed "Δ" so
     a reader skimming the column never takes it for a balance. Equity,
     revenue and expenses show what MOVED in each period, with a Total. The
     help text above the table says so in words.

     Amounts are as written, in natural signs (revenue and liabilities
     negative), one line per commodity: `hledger balance` without `-V`. The
     columns are the statements' own template (statementGrid.ts), so a
     projection's Actual/Projected band and divider fall exactly as they do on
     the statements; on the Reports tab `projectedFrom` is null and there is
     neither. -->
<script lang="ts">
    import type {MixedAmount} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {formatTotals} from "#lib/journal/rowModel.js";
    import type {AccountReport} from "#lib/reports/types.js";
    import {accountDisplaySections, DELTA, READING_HELP, summaryHeading} from "./accountReportRows";
    import FigureColgroup from "./FigureColgroup.svelte";
    import ProjectedHeadRow from "./ProjectedHeadRow.svelte";
    import {projectedEdge} from "./projectedColumns";
    import {statementMinWidth} from "./statementGrid";

    let {
        report,
        styles,
        projectedFrom = null,
    }: {
        report: AccountReport;
        styles: ReadonlyMap<string, AmountStyle>;
        projectedFrom?: number | null;
    } = $props();

    const sections = $derived(accountDisplaySections(report));
    const periods = $derived(report.periods.length);
    const readings = $derived([...new Set(sections.map((section) => section.reading))]);
</script>

{#snippet amount(ma: MixedAmount, delta = false)}
    {@const lines = formatTotals(ma, styles)}
    {#if lines.length === 0}
        <span class="text-base-content/30">{delta ? `${DELTA} 0` : "0"}</span>
    {:else}
        {#each lines as line (line.text)}
            <div class={line.negative ? "text-error" : ""}>{delta ? `${DELTA} ${line.text}` : line.text}</div>
        {/each}
    {/if}
{/snippet}

<div class="flex flex-col gap-2" data-testid="account-report">
    <div class="flex flex-col gap-1 text-xs text-base-content/70" data-testid="account-report-help">
        {#each readings as reading (reading)}
            <p>{READING_HELP[reading]}</p>
        {/each}
        <p>Amounts are as written: revenue and liabilities are negative, as in hledger.</p>
    </div>
    <div class="overflow-x-auto" data-testid="accounts-scroller">
        <section
            class="overflow-hidden rounded-box border border-base-content/10"
            style:min-width={statementMinWidth(periods, true)}
            data-testid="accounts-table"
        >
            <table class="table table-fixed table-sm">
                <FigureColgroup {periods} total={true} />
                <thead>
                    <ProjectedHeadRow columns={periods} {projectedFrom} trailing={1} fixed />
                    <tr class="text-[0.65rem] text-base-content/40">
                        <th class="truncate font-normal">Account</th>
                        {#each report.periods as period, i (period.key)}
                            <th class="{projectedEdge(i, projectedFrom)} text-right font-normal" title="{period.start} to {period.end}">{period.label}</th>
                        {/each}
                        <th class="text-right font-normal"></th>
                    </tr>
                </thead>
                {#each sections as section (section.kind)}
                    <tbody data-testid="account-section" data-section={section.kind} data-reading={section.reading}>
                        <tr class="bg-base-200/60 text-xs font-semibold">
                            <th class="font-semibold">
                                {section.title}
                                <span class="ml-1 font-normal text-base-content/50"
                                    >{section.reading === "balance" ? "balance at period end" : "change in period"}</span
                                >
                            </th>
                            <td colspan={periods}></td>
                            <td class="text-right font-semibold" data-column="summary-heading">{summaryHeading(section.reading)}</td>
                        </tr>
                        {#each section.rows as display (display.row.account)}
                            <tr class="hover:bg-base-200/60" data-account={display.row.account}>
                                <th class="truncate font-normal" title={display.row.account}>
                                    <span class="whitespace-nowrap" style="padding-left: {display.indent + 0.5}rem">{display.label}</span>
                                </th>
                                {#each display.row.values as value, i (i)}
                                    <td class="{projectedEdge(i, projectedFrom)} text-right align-top font-mono whitespace-nowrap tabular-nums"
                                        >{@render amount(value)}</td
                                    >
                                {/each}
                                <td class="text-right align-top font-mono whitespace-nowrap tabular-nums" data-column="summary">
                                    {@render amount(display.row.summary, section.reading === "balance")}
                                </td>
                            </tr>
                        {/each}
                        <tr class="border-t border-base-content/20 font-semibold" data-testid="account-section-total">
                            <th class="font-semibold">Total {section.title.toLowerCase()}</th>
                            {#each section.totals as value, i (i)}
                                <td class="{projectedEdge(i, projectedFrom)} text-right align-top font-mono whitespace-nowrap tabular-nums"
                                    >{@render amount(value)}</td
                                >
                            {/each}
                            <td class="text-right align-top font-mono whitespace-nowrap tabular-nums" data-column="summary">
                                {@render amount(section.summary, section.reading === "balance")}
                            </td>
                        </tr>
                    </tbody>
                {:else}
                    <tbody>
                        <tr>
                            <th class="font-normal text-base-content/50">No account has anything in these periods</th>
                            <td colspan={periods + 1}></td>
                        </tr>
                    </tbody>
                {/each}
            </table>
        </section>
    </div>
</div>
