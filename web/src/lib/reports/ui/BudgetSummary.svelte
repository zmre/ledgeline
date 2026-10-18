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
     high-contrast line; the pace mark is a second, quieter tick. -->
<script lang="ts">
    import {openJournal} from "#lib/journal/openJournal.js";
    import type {MixedAmount} from "#lib/domain/money.js";
    import type {AmountStyle, ISODate} from "#lib/domain/types.js";
    import {sortBudgetLines, type BudgetSortKey, type SortDir} from "#lib/budget/sort.js";
    import {formatTotals} from "#lib/journal/rowModel.js";
    import {barGeometry, primaryValue, type BarGeometry} from "#lib/reports/budgetSummary.js";
    import type {BudgetBar as WireBar, BudgetHealth, BudgetReport} from "#lib/reports/types.js";

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

    /** One MixedAmount → a compact string ("$352", or "$1, €2" multi-commodity; "0" when empty). */
    function fmt(ma: MixedAmount): string {
        const parts = formatTotals(ma, styles).map((l) => l.text);
        return parts.length > 0 ? parts.join(", ") : "0";
    }

    interface BudgetBar {
        account: string;
        spentText: string;
        budgetText: string;
        remainderText: string;
        /** null for a multi-commodity line, which has no single comparison to make. */
        health: BudgetHealth | null;
        pct: number | null;
        geom: BarGeometry | null;
        /** The goal in magnitude, so the sort control can order bars by it. */
        goal: MixedAmount;
    }

    /** The words under a bar, in the vocabulary plans/20 fixes. The engine picks the sentence and its amount. */
    function labelText(label: WireBar["label"]): string {
        const amount = label.amount === null ? "" : fmt(label.amount);
        switch (label.kind) {
            case "onPlan":
                return "on plan";
            case "behindPace":
                return `${amount} behind pace`;
            case "aboveTarget":
                return `${amount} above target`;
            case "toGo":
                return `${amount} to go`;
            case "over":
                return `${amount} over`;
            case "aheadOfPace":
                return `${amount} ahead of pace`;
            case "left":
                return `${amount} left`;
        }
    }

    /** An engine bar → what the row renders. Geometry is the only arithmetic, and it is pixels. */
    function toBar(bar: WireBar): BudgetBar {
        // A bar with no health has no single magnitude to draw (multi-commodity):
        // the engine withheld the verdict, so no geometry either.
        const spent = primaryValue(bar.actual);
        const budget = primaryValue(bar.goal);
        const pace = primaryValue(bar.pace);
        const geom = bar.health !== null && spent !== null && budget !== null && pace !== null ? barGeometry(spent, budget, pace) : null;
        return {
            account: bar.account,
            spentText: fmt(bar.actual),
            budgetText: fmt(bar.goal),
            remainderText: labelText(bar.label),
            health: bar.health,
            pct: geom !== null && geom.ratio !== null ? Math.round(geom.ratio * 100) : null,
            geom,
            goal: bar.goal,
        };
    }

    interface Section {
        title: string;
        verbSpent: string; // "Earned" / "Spent"
        verbOf: string; // "target" / "budgeted"
        overall: BudgetBar;
        bars: BudgetBar[];
    }

    const WORDS = {revenue: {title: "Income", verbSpent: "Earned", verbOf: "target"}, expense: {title: "Expenses", verbSpent: "Spent", verbOf: "budgeted"}};

    // A bar's goal is ALREADY the span total, so it is comparable as it stands —
    // the editor below sorts by the engine's annualised figures instead.
    const sections = $derived<Section[]>(
        report.summary.sections.map((section) => ({
            ...WORDS[section.kind],
            overall: toBar(section.overall),
            bars: sortBudgetLines(section.bars.map(toBar), sort, dir, (bar) => ({account: bar.account, amount: bar.goal})),
        }))
    );

    // The exact dates, not just the month names: the bars cover whole months, so
    // when the controls asked for a partial one this is the only thing telling
    // the user the bar (and the journal link) is wider than what they typed.
    const periodLabel = $derived(
        report.buckets.length === 0
            ? ""
            : report.buckets.length === 1
              ? `${report.periods[0].label} (${from} – ${to})`
              : `${report.periods[0].label} – ${report.periods[report.periods.length - 1].label} (${from} – ${to})`
    );

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
        <div class="rounded-box border border-base-content/10 px-4 py-10 text-center" data-testid="budget-empty">
            <p class="font-medium">No income or expense budget goals for this period.</p>
            <p class="mt-1 text-sm text-base-content/60">
                Add periodic rules (lines starting with <code class="rounded bg-base-200 px-1">~</code>) to your journal. Accounts that aren't named
                <code class="rounded bg-base-200 px-1">expenses…</code>/<code class="rounded bg-base-200 px-1">income…</code> need an
                <code class="rounded bg-base-200 px-1">account … ; type: X</code> directive to be classified.
            </p>
        </div>
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
