<!-- Budget summary (primary budget view): per-category bullet bars grouped into
     Income (revenue) and Expenses, à la Goodbudget / YNAB envelopes. The ENGINE
     computes everything a reader is told (`reports/budget_summary.rs`, served as
     `BudgetReport.summary`, plan 26 B6): one {actual, goal} per leaf budget over
     the span, in magnitudes, split by the accounts' resolved type, with each
     bar's pace, health and label. This component turns that into words and
     pixels and nothing else. Clicking a category opens the journal filtered to
     that account subtree for the same period.

     # Pace, and why a bar is one colour

     A bar is judged against where it SHOULD be by now, not against the whole
     period's goal: the goal is summed over every bucket in the span while the
     actual can only run to today, so in June a year-to-date view read every
     income line as a shortfall and every expense line as a large, reassuring
     underspend. The engine prorates the goal by days elapsed (exactly) and
     decides health from the same three numbers — at or above pace for revenue,
     at or below it for expense.

     The fill is therefore ONE colour. It used to be green up to the goal marker
     and red past it, which is precisely what made "earned more than target"
     render as a red overspend on an income line. The goal marker stays as the
     high-contrast line; the pace mark is a second, quieter tick.

     # Narrow only, since plan 41

     This is the NARROW layout. From `@4xl` (56rem) up the page draws
     `BudgetTable.svelte` instead — goals on the same row as their progress,
     with track-fixed gradient bars. Both build rows from `budgetBars.ts`, so
     they cannot word a bar differently. -->
<script lang="ts">
    import {openJournal} from "#lib/journal/openJournal.js";
    import type {AmountStyle, ISODate} from "#lib/domain/types.js";
    import type {BudgetSortKey, SortDir} from "#lib/budget/sort.js";
    // The rows are built in `budgetBars.ts`, shared with the wide table
    // (`BudgetTable.svelte`), so the two layouts can never word a bar apart.
    import {budgetPeriodLabel, budgetSections, type BudgetBarView as BudgetBar, type BudgetSectionView} from "#lib/reports/ui/budgetBars.js";
    import type {BudgetHealth, BudgetReport} from "#lib/reports/types.js";
    import BudgetEmpty from "./BudgetEmpty.svelte";

    let {
        report,
        styles,
        from,
        to,
        asOf,
        sort,
        dir,
    }: {
        report: BudgetReport;
        styles: ReadonlyMap<string, AmountStyle>;
        /** The budget period (inclusive), forwarded to the journal on a category click. */
        from: ISODate;
        to: ISODate;
        /** The day pace was measured at (the page sent it with the query); shown on the pace tick. */
        asOf: ISODate;
        sort: BudgetSortKey;
        dir: SortDir;
    } = $props();

    const sections = $derived<BudgetSectionView[]>(budgetSections(report, styles, sort, dir));
    const periodLabel = $derived(budgetPeriodLabel(report, from, to));

    /** Green when the line is on the right side of pace, red otherwise; neutral when there is no comparison. */
    const healthText: Record<BudgetHealth, string> = {healthy: "text-success", behind: "text-error", over: "text-error"};
    const labelClass = (health: BudgetHealth | null): string => (health === null ? "text-base-content/60" : healthText[health]);
    const fillClass = (health: BudgetHealth | null): string => (health === "healthy" ? "bg-success" : "bg-error");

    /** Open the journal filtered to `account` (and its subaccounts) for the budget's period. */
    function openInJournal(account: string): void {
        void openJournal({accounts: [account], from, to});
    }
</script>

{#snippet bar(b: BudgetBar, big = false)}
    {#if b.geom !== null}
        {@const g = b.geom}
        <!-- Each segment is a DIRECT child of the sized track, so its %-width/left resolves against the full width. -->
        <div class="relative w-full overflow-hidden rounded-full bg-base-300 {big ? 'h-3.5' : 'h-2.5'}" aria-hidden="true">
            <!-- ONE fill, one colour — see the header comment. -->
            <div class="absolute inset-y-0 left-0 {fillClass(b.health)}" style="width: {g.fillPct}%"></div>
            <!-- Pace mark: quieter and half height, and titled, because an
                 unlabelled second tick on a bar is a puzzle. Dropped by
                 `barGeometry` when it would land on top of the goal marker. -->
            {#if g.pacePct !== null}
                <div class="absolute top-1/4 h-1/2 w-0.5 bg-base-content/35" style="left: {g.pacePct}%" title="Where this should be by {asOf}"></div>
            {/if}
            <!-- goal marker -->
            <div class="absolute inset-y-0 w-0.5 bg-base-content/70" style="left: {g.markerPct}%"></div>
        </div>
    {/if}
{/snippet}

<div class="flex flex-col gap-6" data-testid="budget-summary">
    {#if sections.length === 0}
        <BudgetEmpty />
    {:else}
        {#if periodLabel !== ""}<span class="-mb-2 text-xs text-base-content/50">{periodLabel}</span>{/if}
        {#each sections as section (section.title)}
            <div class="flex flex-col gap-2">
                <div class="flex flex-col gap-2 rounded-box bg-base-200 px-4 py-3">
                    <div class="flex flex-wrap items-baseline justify-between gap-x-3">
                        <span class="text-sm font-semibold">
                            {section.title} · {section.verbSpent}
                            <span class="font-mono tabular-nums">{section.overall.spentText}</span> of
                            <span class="font-mono tabular-nums">{section.overall.budgetText}</span>
                            {section.verbOf}
                        </span>
                        <span class="text-sm font-medium {labelClass(section.overall.health)}">{section.overall.remainderText}</span>
                    </div>
                    {@render bar(section.overall, true)}
                </div>

                <div class="flex flex-col">
                    {#each section.bars as b (b.account)}
                        <button
                            type="button"
                            class="flex w-full cursor-pointer flex-col gap-1.5 rounded border-b border-base-content/5 px-1 py-2.5 text-left transition-colors last:border-b-0 hover:bg-base-200/60"
                            title="View {b.account} in the journal"
                            onclick={() => openInJournal(b.account)}
                        >
                            <div class="flex flex-wrap items-baseline justify-between gap-x-3">
                                <span class="truncate font-medium">{b.account}</span>
                                <span class="flex items-baseline gap-2 text-sm whitespace-nowrap">
                                    <span class="font-mono tabular-nums">{b.spentText} / {b.budgetText}</span>
                                    <span class="font-medium {labelClass(b.health)}">{b.remainderText}</span>
                                    {#if b.pct !== null}<span class="w-10 text-right text-base-content/40 tabular-nums">{b.pct}%</span>{/if}
                                </span>
                            </div>
                            {@render bar(b, false)}
                        </button>
                    {/each}
                </div>
            </div>
        {/each}
    {/if}
</div>
