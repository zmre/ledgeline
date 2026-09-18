<!-- Budget summary (primary budget view): per-category bullet bars grouped into
     Income (revenue) and Expenses, à la Goodbudget / YNAB envelopes. The engine
     returns per-bucket cells; summarizeBudget folds them into one {actual, goal}
     per account. Only revenue & expense accounts are shown (assets/liabilities
     and the synthetic <unbudgeted> cash offset are excluded); revenue budgets
     are entered NEGATIVE per hledger's credit convention, so they read in
     magnitude as "earned $X of $Y target". Clicking a category opens the journal
     filtered to that account subtree for the same period.

     # Pace, and why a bar is one colour

     A bar is judged against where it SHOULD be by now, not against the whole
     period's goal: `summarizeBudget` sums a goal over every bucket in the span
     while the actual can only run to today, so in June a year-to-date view read
     every income line as a shortfall and every expense line as a large,
     reassuring underspend. `elapsedFraction` prorates the goal by days elapsed
     and `budgetHealth` decides, from the same three numbers, which side of that
     mark the line is on — at or above it for revenue, at or below it for expense.

     The fill is therefore ONE colour. It used to be green up to the goal marker
     and red past it, which is precisely what made "earned more than target"
     render as a red overspend on an income line. The goal marker stays as the
     high-contrast line; the pace mark is a second, quieter tick. -->
<script lang="ts">
    import {resolveAccountType, type AccountType} from "$lib/domain/accountTypes";
    import {openJournal} from "$lib/journal/openJournal";
    import {maAdd, maNeg, type MixedAmount} from "$lib/domain/money";
    import type {AmountStyle, ISODate} from "$lib/domain/types";
    import {sortBudgetLines, type BudgetSortKey, type SortDir} from "$lib/budget/sort";
    import {formatTotals} from "$lib/journal/rowModel";
    import {bucketLabel} from "$lib/reports/periods";
    import {
        barGeometry,
        budgetHealth,
        budgetLeaves,
        budgetTotals,
        elapsedFraction,
        magnitudeAmount,
        paceAmount,
        primaryValue,
        summarizeBudget,
        type BarGeometry,
        type BudgetHealth,
        type BudgetLine,
    } from "$lib/reports/budgetSummary";
    import type {BudgetReport} from "$lib/reports/types";

    let {
        report,
        styles,
        declared,
        from,
        to,
        asOf,
        sort,
        dir,
    }: {
        report: BudgetReport;
        styles: ReadonlyMap<string, AmountStyle>;
        /** Declared account types (from journal.accountDecls) → effective-type resolution. */
        declared: ReadonlyMap<string, AccountType>;
        /** The budget period (inclusive), forwarded to the journal on a category click. */
        from: ISODate;
        to: ISODate;
        /**
         * Today, as the page's local clock reads it. A PROP, not a clock read:
         * `budgetSummary.ts` is purity-guarded and may not read one, and a
         * component that took it from `today()` itself could not be tested at a
         * fixed date without mocking a module.
         */
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

    /** How much of the span has run. One number for the whole screen: both sections cover the same dates. */
    const elapsed = $derived(elapsedFraction(from, to, asOf));

    /**
     * The label under a bar, in the vocabulary plans/20 fixes.
     *
     * Six cases, three per column, and "on plan" for the exact-zero remainder in
     * either. Every amount here is a magnitude, so nothing needs a sign.
     */
    function labelFor(health: BudgetHealth | null, income: boolean, goal: MixedAmount, actual: MixedAmount, pace: MixedAmount): string {
        // A multi-commodity line has no bar and no health; it keeps the neutral
        // wording it has always had rather than gaining a colour it cannot earn.
        if (health === null) return "on plan";
        const remainder = maAdd(goal, maNeg(actual)); // |budget| − |spent|
        const left = primaryValue(remainder) ?? 0;
        if (income) {
            if (health === "behind") return `${fmt(maAdd(pace, maNeg(actual)))} behind pace`;
            if (left < 0) return `${fmt(maNeg(remainder))} above target`;
            return left === 0 ? "on plan" : `${fmt(remainder)} to go`;
        }
        if (health === "over") return `${fmt(maNeg(remainder))} over`;
        if (health === "behind") return `${fmt(maAdd(actual, maNeg(pace)))} ahead of pace`;
        return left === 0 ? "on plan" : `${fmt(remainder)} left`;
    }

    /** Build a bar view-model from a budgeted line (goal is non-null here). Works in magnitudes so
     *  income budgets (entered NEGATIVE per hledger's credit convention) read "earned $X of $Y". */
    function toBar(line: BudgetLine): BudgetBar {
        const rawGoal = line.goal ?? new Map();
        const income = (primaryValue(rawGoal) ?? 0) < 0; // credit-normal (revenue) budgets are negative
        const goal = magnitudeAmount(rawGoal);
        const actual = magnitudeAmount(line.actual);
        const pace = paceAmount(goal, elapsed);
        const spent = primaryValue(actual);
        const budget = primaryValue(goal);
        const paceValue = primaryValue(pace);

        // All three or none: the geometry and the health test read the same
        // numbers, so a line that cannot produce one cannot produce the other.
        // (A multi-commodity line produces neither — `primaryValue` refuses a
        // single magnitude for it, and there is no bar to draw.)
        const geom = spent !== null && budget !== null && paceValue !== null ? barGeometry(spent, budget, paceValue) : null;
        const health = spent !== null && budget !== null && paceValue !== null ? budgetHealth(spent, paceValue, budget, income) : null;

        return {
            account: line.account,
            spentText: fmt(actual),
            budgetText: fmt(goal),
            remainderText: labelFor(health, income, goal, actual, pace),
            health,
            pct: geom !== null && geom.ratio !== null ? Math.round(geom.ratio * 100) : null,
            geom,
            goal,
        };
    }

    interface Section {
        title: string;
        verbSpent: string; // "Earned" / "Spent"
        verbOf: string; // "target" / "budgeted"
        overall: BudgetBar;
        bars: BudgetBar[];
    }

    function buildSection(title: string, verbSpent: string, verbOf: string, secLines: BudgetLine[]): Section | null {
        if (secLines.length === 0) return null;
        const totals = budgetTotals(secLines);
        return {
            title,
            verbSpent,
            verbOf,
            overall: toBar({account: "", depth: 0, actual: totals.actual, goal: totals.goal}),
            // A bar's goal is ALREADY the span total — `summarizeBudget` summed
            // every bucket — so it needs no annualising, which is what passing
            // "yearly" (factor 1) says to the shared comparator. The editor below
            // sorts the same list by the per-period figures its rules state, and
            // that is where the factors earn their keep.
            bars: sortBudgetLines(secLines.map(toBar), sort, dir, (bar) => ({account: bar.account, amount: bar.goal, period: "yearly"})),
        };
    }

    const leaves = $derived(budgetLeaves(summarizeBudget(report)));
    const typeOf = (account: string): AccountType | null => resolveAccountType(account, declared);
    const income = $derived(leaves.filter((l) => typeOf(l.account) === "revenue"));
    const expenses = $derived(leaves.filter((l) => typeOf(l.account) === "expense"));
    const sections = $derived(
        [buildSection("Income", "Earned", "target", income), buildSection("Expenses", "Spent", "budgeted", expenses)].filter((s): s is Section => s !== null)
    );

    // The exact dates, not just the month names: the bars cover whole months, so
    // when the controls asked for a partial one this is the only thing telling
    // the user the bar (and the journal link) is wider than what they typed.
    const periodLabel = $derived(
        report.buckets.length === 0
            ? ""
            : report.buckets.length === 1
              ? `${bucketLabel(report.buckets[0])} (${from} – ${to})`
              : `${bucketLabel(report.buckets[0])} – ${bucketLabel(report.buckets[report.buckets.length - 1])} (${from} – ${to})`
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
