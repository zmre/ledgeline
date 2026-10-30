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
     neither.

     A row with rows beneath it has a disclosure, and "Collapse all"/"Expand
     all" close or open every one. `collapsed` is bindable so the page holds it:
     a refetch unmounts this view, and the user's closed rows must survive it.
     Filtering to chosen accounts happens before the report gets here
     (`filterAccountReport`), so the export narrows with it. -->
<script lang="ts">
    import type {MixedAmount} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {formatTotals} from "#lib/journal/rowModel.js";
    import type {AccountReport} from "#lib/reports/types.js";
    import {accountDisplaySections, DELTA, parentAccounts, READING_HELP, summaryHeading, visibleAccountRows} from "./accountReportRows";
    import FigureColgroup from "./FigureColgroup.svelte";
    import ProjectedHeadRow from "./ProjectedHeadRow.svelte";
    import {projectedEdge} from "./projectedColumns";
    import {statementMinWidth} from "./statementGrid";

    let {
        report,
        styles,
        projectedFrom = null,
        collapsed = $bindable(new Set()),
        filtered = false,
    }: {
        report: AccountReport;
        styles: ReadonlyMap<string, AmountStyle>;
        projectedFrom?: number | null;
        /** Accounts whose rows beneath are hidden, by the account each row carries. */
        collapsed?: ReadonlySet<string>;
        /** Whether the report was narrowed to chosen accounts, so its totals are theirs alone. */
        filtered?: boolean;
    } = $props();

    const sections = $derived(accountDisplaySections(report));
    const periods = $derived(report.periods.length);
    const readings = $derived([...new Set(sections.map((section) => section.reading))]);
    const parents = $derived(parentAccounts(sections));
    const anyCollapsed = $derived([...parents].some((account) => collapsed.has(account)));

    function toggle(account: string): void {
        collapsed = collapsed.has(account) ? new Set([...collapsed].filter((name) => name !== account)) : new Set([...collapsed, account]);
    }
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
    <div class="flex items-start gap-4">
        <div class="flex flex-col gap-1 text-xs text-base-content/70" data-testid="account-report-help">
            {#each readings as reading (reading)}
                <p>{READING_HELP[reading]}</p>
            {/each}
            <p>Amounts are as written: revenue and liabilities are negative, as in hledger.</p>
        </div>
        {#if parents.size > 0}
            <div class="join ml-auto shrink-0">
                <button
                    type="button"
                    class="btn join-item btn-xs"
                    disabled={!anyCollapsed}
                    onclick={() => (collapsed = new Set())}
                    data-testid="accounts-expand-all">Expand all</button
                >
                <button
                    type="button"
                    class="btn join-item btn-xs"
                    disabled={[...parents].every((account) => collapsed.has(account))}
                    onclick={() => (collapsed = new Set([...collapsed, ...parents]))}
                    data-testid="accounts-collapse-all">Collapse all</button
                >
            </div>
        {/if}
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
                        {#each visibleAccountRows(section.rows, collapsed) as { display, parent, expanded } (display.row.account)}
                            <tr class="hover:bg-base-200/60" data-account={display.row.account}>
                                <th class="truncate font-normal" title={display.row.account}>
                                    <!-- Every label sits after a 0.75rem disclosure slot, empty on a
                                         leaf, so siblings line up whether or not they have children. -->
                                    <span class="flex items-center gap-1 whitespace-nowrap" style="padding-left: {display.indent + 0.5}rem">
                                        {#if parent}
                                            <button
                                                type="button"
                                                class="flex cursor-pointer items-center gap-1 text-left hover:text-primary"
                                                aria-expanded={expanded}
                                                onclick={() => toggle(display.row.account)}
                                            >
                                                <span
                                                    class="inline-block w-3 shrink-0 text-[0.65rem] text-base-content/40 transition-transform {expanded
                                                        ? 'rotate-90'
                                                        : ''}"
                                                    aria-hidden="true">▶</span
                                                >
                                                {display.label}
                                            </button>
                                        {:else}
                                            <span class="inline-block w-3 shrink-0" aria-hidden="true"></span>{display.label}
                                        {/if}
                                    </span>
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
                            <th class="font-semibold">Total {filtered ? "selected " : ""}{section.title.toLowerCase()}</th>
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
                            <th class="font-normal text-base-content/50"
                                >{filtered ? "None of the chosen accounts has anything in these periods" : "No account has anything in these periods"}</th
                            >
                            <td colspan={periods + 1}></td>
                        </tr>
                    </tbody>
                {/each}
            </table>
        </section>
    </div>
</div>
