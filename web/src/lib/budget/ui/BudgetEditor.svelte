<!-- The goals themselves: what the journal says the plan is, grouped by how often
     it recurs.

     Grouped by PERIOD rather than by rule or by file. A user thinks "my monthly
     budget" and "what I expect to earn in a year"; which `~` block a goal is
     written in, and which file that block sits in, are facts about storage. They
     are shown (as the small line under each group, and in a goal's tooltip)
     because a plain-text ledger's whole promise is that you can go and look —
     but they are not the organising idea.

     A locked goal or rule is rendered, always, with the engine's own sentence
     for why it cannot be edited here. That is `aliases.rs`'s "when in doubt,
     opaque" carried to the screen: a rule Ledgeline will not rewrite is still a
     rule the user has, and hiding it would be a worse lie than showing it
     read-only. -->
<script lang="ts">
    import {joinableRule} from "$lib/budget/target";
    import {BUDGET_PERIODS, type BudgetFile, type BudgetGoal, type BudgetListing, type BudgetRule} from "$lib/budget/types";
    import type {MixedAmount} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import {formatTotals} from "$lib/journal/rowModel";

    let {
        listing,
        styles,
        busy,
        removing,
        onAdd,
        onEdit,
        onRemove,
        onRemoveCancel,
        onRemoveConfirm,
        onCreateFile,
    }: {
        listing: BudgetListing;
        styles: ReadonlyMap<string, AmountStyle>;
        busy: boolean;
        /** The goal a Remove click has armed, or null. Owned by the page, so a
         *  reload of the listing cannot leave a row armed for a goal that moved. */
        removing: {journalId: string; goal: BudgetGoal} | null;
        /** Add a goal. `rule` null means "in a new rule of this period". */
        onAdd: (journalId: string, rule: BudgetRule | null) => void;
        onEdit: (journalId: string, rule: BudgetRule, goal: BudgetGoal) => void;
        /** Arm the two-step removal for this goal. */
        onRemove: (journalId: string, goal: BudgetGoal) => void;
        onRemoveCancel: () => void;
        onRemoveConfirm: (journalId: string, goal: BudgetGoal) => void;
        onCreateFile: () => void;
    } = $props();

    /** Whether this row is the one awaiting a removal confirmation. */
    function armed(row: Row): boolean {
        return removing !== null && removing.journalId === row.file.journalId && removing.goal.index === row.goal.index;
    }

    /** One goal, with the file and rule it belongs to carried alongside. */
    interface Row {
        file: BudgetFile;
        rule: BudgetRule;
        goal: BudgetGoal;
    }

    /** One period's worth of goals, and the rules they came from. */
    interface Group {
        period: string;
        label: string;
        rows: Row[];
        /** The rule a new goal of this period joins, and the file it is in. */
        target: {file: BudgetFile; rule: BudgetRule} | null;
    }

    const rows = $derived<Row[]>(listing.files.flatMap((file) => file.rules.flatMap((rule) => rule.goals.map((goal) => ({file, rule, goal})))));

    /**
     * The goals, grouped by period, in the order the editor offers periods.
     *
     * A period with no goals is omitted rather than shown empty: the "Add a
     * budget goal" button offers every period, so an empty group would be a
     * second, worse way to reach the same place.
     */
    const groups = $derived.by((): Group[] =>
        BUDGET_PERIODS.map(({id, label}) => ({
            period: id,
            label,
            rows: rows.filter((row) => row.rule.period === id),
            // The rule a new goal joins. Asked of `joinableRule` rather than
            // worked out here, because the page asks the same question again when
            // the modal comes back — and two Add paths answering it differently
            // is how the tab came to write a fresh rule per goal.
            target: joinableRule(listing, id),
        })).filter((group) => group.rows.length > 0)
    );

    /** Goals in a period the editor does not offer (a `~ daily` rule, say) still have to appear. */
    const otherRows = $derived(rows.filter((row) => !BUDGET_PERIODS.some((p) => p.id === row.rule.period)));

    /** The file a brand-new rule goes in: the engine's own default target. */
    const defaultFile = $derived(listing.files.find((file) => file.journalId === listing.defaultTarget) ?? null);

    function fmt(amount: MixedAmount | null): string {
        if (amount === null) return "—";
        const parts = formatTotals(amount, styles).map((line) => line.text);
        return parts.length > 0 ? parts.join(", ") : "—";
    }

    /** The magnitude to show: the entry when there is one, else the raw amount. */
    function shown(goal: BudgetGoal): string {
        if (goal.entry === null) return fmt(goal.amount);
        return fmt(new Map([[goal.entry.commodity, goal.entry.value]]));
    }

    /** Whether this goal can be changed here at all. */
    function editable(row: Row): boolean {
        return listing.editable && row.file.writable && row.rule.locked === null && row.goal.locked === null;
    }

    /** Where this goal is written, for the tooltip — the promise plain text makes. */
    function whereabouts(row: Row): string {
        const named = row.rule.description === "" ? "" : ` — “${row.rule.description}”`;
        return `${row.file.label} line ${row.goal.line}${named}`;
    }
</script>

<section class="flex flex-col gap-4" data-testid="budget-editor">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 class="text-base font-semibold">Budget goals</h2>
        {#if listing.editable && defaultFile !== null}
            <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => onAdd(defaultFile.journalId, null)} data-testid="add-goal">
                Add a budget goal
            </button>
        {/if}
    </div>

    {#if !listing.editable}
        <div class="alert py-2 text-sm alert-info" role="status" data-testid="budget-readonly">
            <span>This journal is open read-only, so its budget can be viewed but not changed.</span>
        </div>
    {/if}

    {#if rows.length === 0}
        <div class="rounded-box border border-base-content/10 px-4 py-8 text-center" data-testid="no-goals">
            <p class="font-medium">No budget goals yet.</p>
            <p class="mx-auto mt-1 max-w-prose text-sm text-base-content/60">
                A budget is a set of <code class="rounded bg-base-200 px-1">~</code> rules in your journal saying what you plan to spend or earn each period.
            </p>
            {#if listing.canCreateFile}
                <p class="mx-auto mt-3 max-w-prose text-sm text-base-content/60">
                    Ledgeline can start one for you: it will create
                    <code class="rounded bg-base-200 px-1">{listing.createFileName}</code>
                    beside your journal and add an <code class="rounded bg-base-200 px-1">include</code> line at the end of the main file. Nothing else changes.
                </p>
                <button type="button" class="btn mt-4 btn-primary btn-sm" disabled={busy} onclick={onCreateFile} data-testid="create-budget-file">
                    Create {listing.createFileName}
                </button>
            {:else if listing.editable && defaultFile !== null}
                <button type="button" class="btn mt-4 btn-primary btn-sm" disabled={busy} onclick={() => onAdd(defaultFile.journalId, null)}>
                    Add your first goal
                </button>
            {/if}
        </div>
    {:else}
        {#each groups as group (group.period)}
            <div class="flex flex-col gap-1">
                <div class="flex flex-wrap items-baseline justify-between gap-x-3">
                    <h3 class="text-sm font-semibold">{group.label}</h3>
                    {#if listing.editable && group.target !== null}
                        <button
                            type="button"
                            class="btn btn-ghost btn-xs"
                            disabled={busy}
                            onclick={() => onAdd(group.target!.file.journalId, group.target!.rule)}
                            data-testid="add-goal-{group.period}"
                        >
                            + Add {group.label.toLowerCase()} goal
                        </button>
                    {/if}
                </div>

                <ul class="flex flex-col">
                    {#each group.rows as row (`${row.file.journalId}#${row.goal.index}`)}
                        {@const locked = row.rule.locked ?? row.goal.locked}
                        <li class="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-base-content/5 px-1 py-2 last:border-b-0">
                            <span class="min-w-0 grow truncate font-medium" title={whereabouts(row)}>{row.goal.account}</span>
                            <span class="font-mono text-sm tabular-nums">{shown(row.goal)}</span>
                            {#if editable(row)}
                                {#if armed(row)}
                                    <!-- An inline two-step, like the transaction
                                         popup's delete: the question is asked in
                                         place, and answering it is one click. -->
                                    <span class="flex shrink-0 items-center gap-1">
                                        <span class="text-xs">Remove it?</span>
                                        <button
                                            type="button"
                                            class="btn btn-error btn-xs"
                                            disabled={busy}
                                            onclick={() => onRemoveConfirm(row.file.journalId, row.goal)}
                                            data-testid="confirm-remove"
                                        >
                                            Confirm
                                        </button>
                                        <button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={onRemoveCancel}>Keep</button>
                                    </span>
                                {:else}
                                    <span class="flex shrink-0 gap-1">
                                        <button
                                            type="button"
                                            class="btn btn-ghost btn-xs"
                                            disabled={busy}
                                            onclick={() => onEdit(row.file.journalId, row.rule, row.goal)}
                                        >
                                            Edit
                                        </button>
                                        <button
                                            type="button"
                                            class="btn btn-ghost text-error btn-xs"
                                            disabled={busy}
                                            onclick={() => onRemove(row.file.journalId, row.goal)}
                                        >
                                            Remove
                                        </button>
                                    </span>
                                {/if}
                            {:else if locked !== null}
                                <!-- The engine's own sentence, verbatim. It says
                                     what stopped it, which is the only thing that
                                     makes a read-only row actionable. -->
                                <span class="shrink-0 text-xs text-base-content/50" title={locked} data-testid="goal-locked">🔒 read-only</span>
                            {:else if !row.file.writable}
                                <span class="shrink-0 text-xs text-base-content/50" title="This file is not a regular file inside the journal's directory.">
                                    🔒 read-only
                                </span>
                            {/if}
                        </li>
                    {/each}
                </ul>
            </div>
        {/each}

        {#if otherRows.length > 0}
            <div class="flex flex-col gap-1">
                <h3 class="text-sm font-semibold">Other periods</h3>
                <ul class="flex flex-col">
                    {#each otherRows as row (`${row.file.journalId}#${row.goal.index}`)}
                        <li class="flex flex-wrap items-center gap-x-3 border-b border-base-content/5 px-1 py-2 last:border-b-0">
                            <span class="min-w-0 grow truncate font-medium" title={whereabouts(row)}>{row.goal.account}</span>
                            <span class="badge badge-ghost badge-sm">{row.rule.period}</span>
                            <span class="font-mono text-sm tabular-nums">{shown(row.goal)}</span>
                        </li>
                    {/each}
                </ul>
            </div>
        {/if}
    {/if}
</section>
