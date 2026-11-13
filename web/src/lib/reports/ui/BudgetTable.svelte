<!-- The Budget tab's table: one per section, each goal on the same row as its
     progress (plan 41 WP-B, after Banktivity's budget screen).

         Account | Budgeted | Actual ………… Remaining
                              [=========|=======    ]

     All the bars are drawn here, at every width. Rows are built in
     `budgetBars.ts`, so a bar's words and amounts come from one place.

     # One layout, folded by a container query

     From `@2xl` (42rem of the table's OWN width) up a row is the four columns
     above. Below it the same row folds onto two lines, the progress cell
     spanning the row under the other two:

         expenses:food                       ✎ $300.00
         $130.00 …………………………………………… $170.00 left
         [=========|=======                         ]

     A container query and not a script switch between two components, so
     there is exactly one of every element and test id at any width. The
     `@container` is the table's root; a rem in a container query is the
     LIVE root size, so the fold moves with the interface scale (plan 39).

     # Budgeted is where you edit

     The figure is the span total of the goals behind the row, and the row is
     where a user looks when they want to change it. So the figure IS the edit
     control, in the primary colour (Banktivity's blue), with a pencil BEFORE
     it that appears when the row is hovered or focused (always, dimmed, on a
     device that cannot hover). The pencil is inside the same button and keeps
     its width while invisible, so the figures never shift. Which goals are
     behind it is `rowGoals.ts`'s question, and what the cell offers is its
     `budgetedCell` answer —

       - one editable goal: the amount opens the goal modal for it;
       - several: the amount opens `GoalsPopover`, listing each with Edit/Remove
         (a figure that is a sum cannot honestly be one edit button);
       - none editable: plain text, and the lock's reason as the tooltip;
       - none found (the listing is loading or failed): plain text.

     The Total row closes each section, under a rule. It is a sum of sums and
     is never editable.

     # The bar's colour is a gradient fixed to the TRACK

     `barGradient` lays the stops out across the whole track, keyed to the goal
     marker, and the fill only reveals it (`fillClip`): a line at 40 % shows
     green, at 85 % it has reached amber, at its goal it ends in orange, and
     past the marker it is red. Income ramps info → success and never turns red,
     because earning more than the target is not an overspend (plans/20).

     The goal marker is NOT inside the clipped, rounded track: it is a sibling
     standing a few px proud of it above and below, in base-content with a
     base-100 ring, so it still reads where the fill has run past it in red,
     orange or green. The pace tick stays inside the track, quieter.

     The Remaining text carries the pace verdict instead of the bar: the
     engine's `health` and label, coloured success when on the right side of
     pace, warning when not, and error only for an expense past its goal.

     # Why the layout is a CSS grid with ARIA table roles

     A `<table>` cannot put the bar under the Actual and Remaining cells while
     keeping each a cell, nor fold a row onto two lines. A grid with explicit
     `role`s keeps the screen-reader reading — four columns, a header row, one
     row per account, the Total last — and lets the bar span the two figures it
     belongs to, as in the reference. -->
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
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
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

    /**
     * One template for every row of a section, so the columns line up without
     * a `<table>`. Two columns below the breakpoint — the account and its
     * Budgeted figure, with the progress cell wrapping under both (it spans
     * them) — and the four-column reading from `@2xl` (42rem of the TABLE's
     * own width) up. See the header for why a container query.
     */
    const GRID =
        "grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1.5 @2xl:grid-cols-[minmax(0,1fr)_minmax(7rem,auto)_minmax(16rem,2fr)] @2xl:gap-x-6";
    /** The progress cell: its own line under the other two when narrow, the third column when wide. */
    const PROGRESS_CELL = "col-span-2 @2xl:col-span-1";

    function openInJournal(account: string): void {
        void openJournal({accounts: [account], from, to});
    }
</script>

{#snippet bar(b: BudgetBarView, kind: BudgetSectionKind, big: boolean)}
    {#if b.geom !== null}
        {@const g = b.geom}
        <div class="relative w-full {big ? 'h-2.5' : 'h-2'}" aria-hidden="true">
            <!-- Only the TRACK clips: the fill and the pace tick are inside it, the goal marker is not. -->
            <div class="absolute inset-0 overflow-hidden rounded-full bg-base-300" data-testid="budget-bar-track">
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
            </div>
            <!-- Taller than the track and ringed in the page colour, so it reads over any fill in any theme. -->
            <div
                class="absolute -inset-y-1 w-[3px] -translate-x-1/2 rounded-full bg-base-content shadow-[0_0_0_1px_var(--color-base-100)]"
                style="left: {g.markerPct}%"
                data-testid="budget-goal-marker"
            ></div>
        </div>
    {/if}
{/snippet}

{#snippet progress(b: BudgetBarView, kind: BudgetSectionKind, big: boolean)}
    <div class="flex min-w-0 flex-col gap-2 {PROGRESS_CELL}">
        <div class="flex items-baseline justify-between gap-3 whitespace-nowrap">
            <span role="cell" class="font-mono tabular-nums">{b.spentText}</span>
            <span role="cell" class="truncate text-sm font-medium {remainingClass(b)}" data-testid="budget-remaining">{b.remainderText}</span>
        </div>
        {@render bar(b, kind, big)}
    </div>
{/snippet}

{#snippet pencil()}
    <!-- Invisible but still laid out, so the figure does not move when it appears. -->
    <span
        class="inline-flex opacity-0 transition-opacity group-focus-within/row:opacity-100 group-hover/row:opacity-100 [@media(hover:none)]:opacity-60"
        data-testid="budgeted-pencil"
    >
        <ReviewIcon name="pencil" class="h-3.5 w-3.5" />
    </span>
{/snippet}

{#snippet budgeted(b: BudgetBarView)}
    {@const cell = listing === null ? ({kind: "none"} as const) : budgetedCell(listing, goalsByAccount.get(b.account))}
    <span role="cell" class="justify-self-end text-right whitespace-nowrap">
        {#if cell.kind === "single"}
            {@const ref = cell.ref}
            <button
                type="button"
                class="inline-flex cursor-pointer items-center gap-1.5 font-mono text-primary tabular-nums hover:underline"
                disabled={busy}
                title="Edit this goal ({ref.goal.account})"
                aria-label="Edit budget for {b.account}"
                data-testid="budgeted-edit"
                onclick={() => onEdit(ref.file.journalId, ref.rule, ref.goal)}
            >
                {@render pencil()}{b.budgetText}
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
                {pencil}
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

<div class="@container flex flex-col gap-8" data-testid="budget-summary">
    {#if sections.length === 0}
        <BudgetEmpty />
    {:else}
        {#if periodLabel !== ""}<span class="-mb-6 text-xs text-base-content/50">{periodLabel}</span>{/if}
        {#each sections as section (section.kind)}
            <div role="table" aria-label="Budgeted {section.title.toLowerCase()}" class="flex flex-col" data-testid="budget-table-{section.kind}">
                <div role="row" class="{GRID} border-b border-base-content/15 pb-1 text-xs font-medium tracking-wide text-base-content/60 uppercase">
                    <span role="columnheader">Budgeted {section.title.toLowerCase()}</span>
                    <span role="columnheader" class="justify-self-end">Budgeted</span>
                    <!-- Folded, the figures above each bar explain themselves ("$170.00 left"): no header line for them. -->
                    <span class="hidden justify-between @2xl:flex">
                        <span role="columnheader">Actual</span>
                        <span role="columnheader">Remaining</span>
                    </span>
                </div>

                <div role="rowgroup" class="flex flex-col divide-y divide-base-content/5">
                    {#each section.bars as b (b.account)}
                        <div role="row" class="{GRID} group/row py-2.5" data-testid="budget-row">
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

                <div role="row" class="{GRID} border-t border-base-content/20 py-2.5 font-semibold" data-testid="budget-total-row">
                    <span role="rowheader">Total</span>
                    <span role="cell" class="justify-self-end font-mono tabular-nums">{section.overall.budgetText}</span>
                    {@render progress(section.overall, section.kind, true)}
                </div>
            </div>
        {/each}
    {/if}
</div>
