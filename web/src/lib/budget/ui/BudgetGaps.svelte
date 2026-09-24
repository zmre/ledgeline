<!-- What the budget does NOT cover: the income and expense categories no `~` rule
     measures, over the same span and at the same depth as the bars above.

     Collapsed by default, in the house style (daisyUI `collapse collapse-arrow`
     driven by a checkbox, state persisted by the caller). Two reasons, and the
     first is the user's: it is fine to budget some things and not others — "I set
     budgets for clothing but not for insurance" — so this is a check on how
     encompassing the budget is, not a list of chores. The second is mechanical:
     the disclosure state GATES THE FETCH, so a section nobody opens costs no
     request on a page that already refetches on every control change.

     THE SHELL IS STABLE ACROSS LOAD STATES, which is why `AsyncSection` is inside
     this component rather than around it — the same correction `SankeyPanel`
     carries. Wrapped from outside there would be no header and no arrow until the
     data landed, so a user who had shut the section would get a spinner block
     sitting where a shut section belongs.

     The header says the count and the total while shut, because a collapsed row
     that says nothing is a row you have to open to learn whether it was worth
     opening. Each row links into the journal for the same span a bar does. -->
<script lang="ts">
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {maAdd, type MixedAmount} from "$lib/domain/money";
    import type {AmountStyle, ISODate} from "$lib/domain/types";
    import {openJournal} from "$lib/journal/openJournal";
    import {formatTotals} from "$lib/journal/rowModel";
    import {magnitudeAmount} from "$lib/reports/budgetSummary";
    import type {BudgetGaps, GapRow} from "$lib/reports/types";
    import type {DataView} from "$lib/stores/loadState";

    let {
        gaps,
        view,
        error,
        styles,
        from,
        to,
        open,
        onToggle,
        onRetry,
    }: {
        /** The held payload, or null before the first successful load. */
        gaps: BudgetGaps | null;
        view: DataView;
        error: Error | null;
        styles: ReadonlyMap<string, AmountStyle>;
        /** The bars' span, for the journal links. The engine echoes its own; these are the controls'. */
        from: ISODate;
        to: ISODate;
        open: boolean;
        onToggle: (open: boolean) => void;
        onRetry: () => void;
    } = $props();

    /** One MixedAmount → a compact string ("$352", or "$1, €2" multi-commodity; "0" when empty). */
    function fmt(ma: MixedAmount): string {
        const parts = formatTotals(ma, styles).map((line) => line.text);
        return parts.length > 0 ? parts.join(", ") : "0";
    }

    /** Sum a section's rows. They are disjoint subtrees at one depth, so this does not double count. */
    function sectionTotal(rows: readonly GapRow[]): MixedAmount {
        let total: MixedAmount = new Map();
        for (const row of rows) total = maAdd(total, row.total);
        return total;
    }

    /**
     * The two lists, in reading order — income first, because "what am I not
     * counting" is a smaller question about earnings than about spending and
     * makes a shorter list to get past.
     */
    function sectionsOf(current: BudgetGaps): {title: string; rows: GapRow[]}[] {
        return [
            {title: "Unbudgeted income", rows: current.revenue},
            {title: "Unbudgeted expenses", rows: current.expense},
        ];
    }

    const shown = $derived(view === "data" ? gaps : null);
    const rowCount = $derived(shown === null ? 0 : shown.revenue.length + shown.expense.length);

    /**
     * The headline while shut: how many categories, and how much they come to in
     * MAGNITUDE.
     *
     * The two sections are summed separately and their magnitudes added, never
     * the raw figures: revenue is credit-normal, so a straight sum would net
     * unbudgeted income against unbudgeted spending and report the difference as
     * "not budgeted" — a smaller, friendlier number that answers nothing.
     */
    const headlineTotal = $derived.by(() => {
        if (shown === null) return null;
        return maAdd(magnitudeAmount(sectionTotal(shown.revenue)), magnitudeAmount(sectionTotal(shown.expense)));
    });

    /** Open the journal filtered to `account` (and its subaccounts) for the budget's period. */
    function openInJournal(account: string): void {
        void openJournal({accounts: [account], from, to});
    }
</script>

<section class="collapse-arrow collapse bg-base-200" data-testid="budget-gaps">
    <input type="checkbox" checked={open} onchange={(e) => onToggle(e.currentTarget.checked)} aria-label="Toggle what is not budgeted" />
    <div class="collapse-title flex min-h-0 flex-wrap items-baseline justify-between gap-x-3 py-3 pr-10">
        <h2 class="text-sm font-semibold tracking-tight">Not budgeted</h2>
        {#if rowCount > 0 && headlineTotal !== null}
            <span class="text-sm text-base-content/70" data-testid="budget-gaps-headline">
                {rowCount}
                {rowCount === 1 ? "category" : "categories"},
                <span class="font-mono font-semibold tabular-nums">{fmt(headlineTotal)}</span> not budgeted
            </span>
        {/if}
    </div>
    <div class="collapse-content flex flex-col gap-4">
        <p class="text-xs text-base-content/50">
            Income and expenses over the same period as the bars, at the same depth, that no <code class="rounded bg-base-300 px-1">~</code> rule mentions. Not a
            problem in itself — budgeting some categories and not others is a choice — but it is what your bars above are silent about.
        </p>

        <AsyncSection
            {view}
            value={gaps}
            {error}
            testid="budget-gaps-error"
            label="the unbudgeted categories"
            loadingLabel="Loading unbudgeted categories"
            {onRetry}
        >
            {#snippet children(current)}
                {#if current.revenue.length === 0 && current.expense.length === 0}
                    <p class="py-6 text-center text-sm text-base-content/60" data-testid="budget-gaps-empty">
                        Every income and expense category with activity in {current.from} – {current.to} is covered by a goal.
                    </p>
                {:else}
                    {#each sectionsOf(current) as section (section.title)}
                        {#if section.rows.length > 0}
                            <div class="flex flex-col gap-1">
                                <div class="flex flex-wrap items-baseline justify-between gap-x-3">
                                    <h3 class="text-sm font-semibold">{section.title}</h3>
                                    <span class="font-mono text-sm font-medium tabular-nums">{fmt(magnitudeAmount(sectionTotal(section.rows)))}</span>
                                </div>
                                <ul class="flex flex-col">
                                    {#each section.rows as row (row.account)}
                                        <li class="border-b border-base-content/5 last:border-b-0">
                                            <button
                                                type="button"
                                                class="flex w-full cursor-pointer flex-wrap items-baseline justify-between gap-x-3 rounded px-1 py-2 text-left transition-colors hover:bg-base-300/60"
                                                title="View {row.account} in the journal"
                                                onclick={() => openInJournal(row.account)}
                                            >
                                                <span class="min-w-0 truncate font-medium">{row.account}</span>
                                                <span class="font-mono text-sm tabular-nums">{fmt(magnitudeAmount(row.total))}</span>
                                            </button>
                                        </li>
                                    {/each}
                                </ul>
                            </div>
                        {/if}
                    {/each}
                {/if}
            {/snippet}
        </AsyncSection>
    </div>
</section>
