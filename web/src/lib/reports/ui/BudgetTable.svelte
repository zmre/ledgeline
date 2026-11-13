<!-- The Budget tab on a wide layout: one table per section, each goal on the
     same row as its progress (plan 41 WP-B, after Banktivity's budget screen).

         Account | Budgeted | Actual ………… Remaining
                              [=========|=======    ]

     The narrow layout is `BudgetSummary.svelte`, untouched. Both build their
     rows from `budgetBars.ts`, so a bar's words and amounts are the same at
     every width; what differs is only where they sit and how the bar is
     coloured.

     # Budgeted is where you edit

     The figure is the span total of the goals behind the row, and the row is
     where a user looks when they want to change it. So the figure IS the edit
     control, in the primary colour (Banktivity's blue): which goals are behind
     it is `rowGoals.ts`'s question, and what the cell offers is its
     `budgetedCell` answer —

       - one editable goal: the amount opens the goal modal for it;
       - several: the amount opens `GoalsPopover`, listing each with Edit/Remove
         (a figure that is a sum cannot honestly be one edit button);
       - none editable: plain text, and the lock's reason as the tooltip;
       - none found (the listing is loading or failed): plain text.

     The Total row is a sum of sums and is never editable.

     # The bar's colour is a gradient fixed to the TRACK

     `barGradient` lays the stops out across the whole track, keyed to the goal
     marker, and the fill only reveals it (`fillClip`): a line at 40 % shows
     green, at 85 % it has reached amber, at its goal it ends in orange, and
     past the marker it is red. Income ramps info → success and never turns red,
     because earning more than the target is not an overspend (plans/20).

     The Remaining text carries the pace verdict instead of the bar: the
     engine's `health` and label, coloured success when on the right side of
     pace, warning when not, and error only for an expense past its goal.

     # Why the layout is a CSS grid with ARIA table roles

     A `<table>` cannot put the bar under the Actual and Remaining cells while
     keeping each a cell. A grid with explicit `role`s keeps the screen-reader
     reading — four columns, a header row, one row per account — and lets the
     bar span the two figures it belongs to, as in the reference. -->
<script lang="ts">
    import {openJournal} from "#lib/journal/openJournal.js";
    import type {AmountStyle, ISODate} from "#lib/domain/types.js";
    import type {BudgetSortKey, SortDir} from "#lib/budget/sort.js";
    import {budgetedCell, type GoalRef} from "#lib/budget/rowGoals.js";
    import type {BudgetGoal, BudgetListing, BudgetRule} from "#lib/budget/types.js";
    import GoalsPopover from "#lib/budget/ui/GoalsPopover.svelte";
    import {budgetPeriodLabel, budgetSections, type BudgetBarView, type BudgetSectionKind, type BudgetSectionView} from "#lib/reports/ui/budgetBars.js";
    import {barGradient, fillClip} from "#lib/reports/budgetSummary.js";
    import type {BudgetReport} from "#lib/reports/types.js";
    import BudgetEmpty from "./BudgetEmpty.svelte";

    let {
        report,
        styles,
        from,
        to,
        asOf,
        sort,
        dir,
        listing,
        goalsByAccount,
        busy,
        removing,
        onEdit,
        onRemove,
        onRemoveCancel,
        onRemoveConfirm,
    }: {
        report: BudgetReport;
        styles: ReadonlyMap<string, AmountStyle>;
        /** The budget period (inclusive), forwarded to the journal on an account click. */
        from: ISODate;
        to: ISODate;
        /** The day pace was measured at; shown on the pace tick. */
        asOf: ISODate;
        sort: BudgetSortKey;
        dir: SortDir;
        /** The goals listing, or null while it loads (every Budgeted cell is then plain text). */
        listing: BudgetListing | null;
        /** Row account → the goals behind it (`rowGoals`), computed by the page so the editor can leave them out. */
        goalsByAccount: ReadonlyMap<string, readonly GoalRef[]>;
        busy: boolean;
        removing: {journalId: string; goal: BudgetGoal} | null;
        onEdit: (journalId: string, rule: BudgetRule, goal: BudgetGoal) => void;
        onRemove: (journalId: string, goal: BudgetGoal) => void;
        onRemoveCancel: () => void;
        onRemoveConfirm: (journalId: string, goal: BudgetGoal) => void;
    } = $props();

    const sections = $derived<BudgetSectionView[]>(budgetSections(report, styles, sort, dir));
    const periodLabel = $derived(budgetPeriodLabel(report, from, to));

    /**
     * The Remaining text's colour: the pace verdict, in three steps. Error is
     * reserved for an expense actually past its goal (the engine's `over`
     * label), so "a little ahead of pace" and "over budget" do not shout alike.
     * Income has no `over` label, so it can never go red.
     */
    function remainingClass(bar: BudgetBarView): string {
        if (bar.health === null) return "text-base-content/60";
        if (bar.labelKind === "over") return "text-error";
        return bar.health === "healthy" ? "text-success" : "text-warning";
    }

    /** One template for every row of a section, so the columns line up without a `<table>`. */
    const GRID = "grid grid-cols-[minmax(0,1fr)_minmax(7rem,auto)_minmax(16rem,2fr)] items-center gap-x-6";

    function openInJournal(account: string): void {
        void openJournal({accounts: [account], from, to});
    }
</script>

{#snippet bar(b: BudgetBarView, kind: BudgetSectionKind, big: boolean)}
    {#if b.geom !== null}
        {@const g = b.geom}
        <div class="relative w-full overflow-hidden rounded-full bg-base-300 {big ? 'h-2.5' : 'h-2'}" aria-hidden="true">
            <!-- The gradient spans the whole track; the clip reveals the fill. -->
            <div
                class="absolute inset-0"
                style="background-image: {barGradient(kind, g)}; clip-path: {fillClip(g.fillPct)}"
                data-testid="budget-bar-fill"
                data-kind={kind}
            ></div>
            {#if g.pacePct !== null}
                <div class="absolute top-1/4 h-1/2 w-0.5 bg-base-content/35" style="left: {g.pacePct}%" title="Where this should be by {asOf}"></div>
            {/if}
            <div class="absolute inset-y-0 w-0.5 bg-base-content/70" style="left: {g.markerPct}%"></div>
        </div>
    {/if}
{/snippet}

{#snippet progress(b: BudgetBarView, kind: BudgetSectionKind, big: boolean)}
    <div class="flex min-w-0 flex-col gap-1">
        <div class="flex items-baseline justify-between gap-3 whitespace-nowrap">
            <span role="cell" class="font-mono tabular-nums">{b.spentText}</span>
            <span role="cell" class="text-sm font-medium {remainingClass(b)}" data-testid="budget-remaining">{b.remainderText}</span>
        </div>
        {@render bar(b, kind, big)}
    </div>
{/snippet}

{#snippet budgeted(b: BudgetBarView)}
    {@const cell = listing === null ? ({kind: "none"} as const) : budgetedCell(listing, goalsByAccount.get(b.account))}
    <span role="cell" class="justify-self-end text-right whitespace-nowrap">
        {#if cell.kind === "single"}
            {@const ref = cell.ref}
            <button
                type="button"
                class="cursor-pointer font-mono text-primary tabular-nums hover:underline"
                disabled={busy}
                title="Edit this goal ({ref.goal.account})"
                aria-label="Edit the budget goal for {b.account}: {b.budgetText}"
                data-testid="budgeted-edit"
                onclick={() => onEdit(ref.file.journalId, ref.rule, ref.goal)}
            >
                {b.budgetText}
            </button>
        {:else if cell.kind === "several" && listing !== null}
            <GoalsPopover
                account={b.account}
                amountText={b.budgetText}
                refs={cell.refs}
                {listing}
                {styles}
                {busy}
                {removing}
                {onEdit}
                {onRemove}
                {onRemoveCancel}
                {onRemoveConfirm}
            />
        {:else if cell.kind === "locked"}
            <span class="font-mono tabular-nums" title={cell.reason} data-testid="budgeted-locked">
                <span class="mr-1 text-xs" aria-label="read-only">🔒</span>{b.budgetText}
            </span>
        {:else}
            <span class="font-mono tabular-nums">{b.budgetText}</span>
        {/if}
    </span>
{/snippet}

<div class="flex flex-col gap-8" data-testid="budget-summary">
    {#if sections.length === 0}
        <BudgetEmpty />
    {:else}
        {#if periodLabel !== ""}<span class="-mb-6 text-xs text-base-content/50">{periodLabel}</span>{/if}
        {#each sections as section (section.kind)}
            <div role="table" aria-label="Budgeted {section.title.toLowerCase()}" class="flex flex-col" data-testid="budget-table-{section.kind}">
                <div role="row" class="{GRID} border-b border-base-content/15 pb-1 text-xs font-medium tracking-wide text-base-content/60 uppercase">
                    <span role="columnheader">Budgeted {section.title.toLowerCase()}</span>
                    <span role="columnheader" class="justify-self-end">Budgeted</span>
                    <span class="flex justify-between">
                        <span role="columnheader">Actual</span>
                        <span role="columnheader">Remaining</span>
                    </span>
                </div>

                <div role="row" class="{GRID} border-b border-base-content/10 py-2.5 font-semibold" data-testid="budget-total-row">
                    <span role="rowheader">Total</span>
                    <span role="cell" class="justify-self-end font-mono tabular-nums">{section.overall.budgetText}</span>
                    {@render progress(section.overall, section.kind, true)}
                </div>

                {#each section.bars as b (b.account)}
                    <div role="row" class="{GRID} border-b border-base-content/5 py-2.5 last:border-b-0" data-testid="budget-row">
                        <span role="rowheader" class="min-w-0">
                            <button
                                type="button"
                                class="max-w-full cursor-pointer truncate text-left font-medium hover:underline"
                                title="View {b.account} in the journal"
                                onclick={() => openInJournal(b.account)}
                            >
                                {b.account}
                            </button>
                        </span>
                        {@render budgeted(b)}
                        {@render progress(b, section.kind, false)}
                    </div>
                {/each}
            </div>
        {/each}
    {/if}
</div>
