<!-- Budget summary (primary budget view): per-category bullet bars grouped into
     Income (revenue) and Expenses, à la Goodbudget / YNAB envelopes. The engine
     returns per-bucket cells; summarizeBudget folds them into one {actual, goal}
     per account. Only revenue & expense accounts are shown (assets/liabilities
     and the synthetic <unbudgeted> cash offset are excluded); revenue budgets
     are entered NEGATIVE per hledger's credit convention, so they read in
     magnitude as "earned $X of $Y target". Clicking a category opens the journal
     filtered to that account subtree for the same period. -->
<script lang="ts">
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {resolveAccountType, type AccountType} from "$lib/domain/accountTypes";
    import {maAdd, maNeg, type MixedAmount} from "$lib/domain/money";
    import type {AmountStyle, ISODate} from "$lib/domain/types";
    import {filterToSearch} from "$lib/filters/urlCodec";
    import {formatTotals} from "$lib/journal/rowModel";
    import {bucketLabel} from "$lib/reports/periods";
    import {barGeometry, budgetLeaves, budgetTotals, magnitudeAmount, primaryValue, summarizeBudget, type BarGeometry, type BudgetLine} from "$lib/reports/budgetSummary";
    import type {BudgetReport} from "$lib/reports/types";
    import {defaultFilter, filters, type JournalFilter} from "$lib/stores/filters.svelte";

    let {report, styles, declared, from, to}: {
        report: BudgetReport;
        styles: ReadonlyMap<string, AmountStyle>;
        /** Declared account types (from journal.accountDecls) → effective-type resolution. */
        declared: ReadonlyMap<string, AccountType>;
        /** The budget period (inclusive), forwarded to the journal on a category click. */
        from: ISODate;
        to: ISODate;
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
        state: "under" | "over" | "onplan";
        pct: number | null;
        geom: BarGeometry | null;
    }

    /** Build a bar view-model from a budgeted line (goal is non-null here). Works in magnitudes so
     *  income budgets (entered NEGATIVE per hledger's credit convention) read "earned $X of $Y". */
    function toBar(line: BudgetLine): BudgetBar {
        const rawGoal = line.goal ?? new Map();
        const income = (primaryValue(rawGoal) ?? 0) < 0; // credit-normal (revenue) budgets are negative
        const goal = magnitudeAmount(rawGoal);
        const actual = magnitudeAmount(line.actual);
        const remainder = maAdd(goal, maNeg(actual)); // |budget| − |spent|
        const remValue = primaryValue(remainder);
        const spent = primaryValue(actual);
        const budget = primaryValue(goal);
        const geom = spent !== null && budget !== null ? barGeometry(spent, budget) : null;

        const state: BudgetBar["state"] = remValue !== null && remValue > 0 ? "under" : remValue !== null && remValue < 0 ? "over" : "onplan";
        const remainderText =
            state === "onplan" ? "on plan" : state === "under" ? `${fmt(remainder)} ${income ? "to go" : "left"}` : `${fmt(maNeg(remainder))} over`;

        return {
            account: line.account,
            spentText: fmt(actual),
            budgetText: fmt(goal),
            remainderText,
            state,
            pct: geom !== null && geom.ratio !== null ? Math.round(geom.ratio * 100) : null,
            geom,
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
        return {title, verbSpent, verbOf, overall: toBar({account: "", depth: 0, actual: totals.actual, goal: totals.goal}), bars: secLines.map(toBar)};
    }

    const leaves = $derived(budgetLeaves(summarizeBudget(report)));
    const typeOf = (account: string): AccountType | null => resolveAccountType(account, declared);
    const income = $derived(leaves.filter((l) => typeOf(l.account) === "revenue"));
    const expenses = $derived(leaves.filter((l) => typeOf(l.account) === "expense"));
    const sections = $derived(
        [buildSection("Income", "Earned", "target", income), buildSection("Expenses", "Spent", "budgeted", expenses)].filter((s): s is Section => s !== null)
    );

    const periodLabel = $derived(
        report.buckets.length === 0
            ? ""
            : report.buckets.length === 1
              ? bucketLabel(report.buckets[0])
              : `${bucketLabel(report.buckets[0])} – ${bucketLabel(report.buckets[report.buckets.length - 1])}`
    );

    const stateText: Record<BudgetBar["state"], string> = {under: "text-success", over: "text-error", onplan: "text-base-content/60"};

    /** Open the journal filtered to `account` (and its subaccounts) for the budget's period. */
    function openInJournal(account: string): void {
        const filter: JournalFilter = {from, to, accounts: new Set([account]), query: "", preset: null};
        filters.replace(filter);
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/") IS the route id; the query string is appended
        void goto(`${resolve("/")}?${filterToSearch(filter, defaultFilter())}`);
    }
</script>

{#snippet bar(b: BudgetBar, big = false)}
    {#if b.geom !== null}
        {@const g = b.geom}
        <!-- Each segment is a DIRECT child of the sized track, so its %-width/left resolves against the full width. -->
        <div class="bg-base-300 relative w-full overflow-hidden rounded-full {big ? 'h-3.5' : 'h-2.5'}" aria-hidden="true">
            <div class="bg-success absolute inset-y-0 left-0" style="width: {g.underPct}%"></div>
            <div class="bg-error absolute inset-y-0" style="left: {g.underPct}%; width: {g.overPct}%"></div>
            <!-- goal marker -->
            <div class="bg-base-content/70 absolute inset-y-0 w-0.5" style="left: {g.markerPct}%"></div>
        </div>
    {/if}
{/snippet}

<div class="flex flex-col gap-6" data-testid="budget-summary">
    {#if sections.length === 0}
        <div class="border-base-content/10 rounded-box border px-4 py-10 text-center" data-testid="budget-empty">
            <p class="font-medium">No income or expense budget goals for this period.</p>
            <p class="text-base-content/60 mt-1 text-sm">
                Add periodic rules (lines starting with <code class="bg-base-200 rounded px-1">~</code>) to your journal. Accounts that aren't named
                <code class="bg-base-200 rounded px-1">expenses…</code>/<code class="bg-base-200 rounded px-1">income…</code> need an
                <code class="bg-base-200 rounded px-1">account … ; type: X</code> directive to be classified.
            </p>
        </div>
    {:else}
        {#if periodLabel !== ""}<span class="text-base-content/50 -mb-2 text-xs">{periodLabel}</span>{/if}
        {#each sections as section (section.title)}
            <div class="flex flex-col gap-2">
                <div class="bg-base-200 rounded-box flex flex-col gap-2 px-4 py-3">
                    <div class="flex flex-wrap items-baseline justify-between gap-x-3">
                        <span class="text-sm font-semibold">
                            {section.title} · {section.verbSpent}
                            <span class="font-mono tabular-nums">{section.overall.spentText}</span> of
                            <span class="font-mono tabular-nums">{section.overall.budgetText}</span>
                            {section.verbOf}
                        </span>
                        <span class="text-sm font-medium {stateText[section.overall.state]}">{section.overall.remainderText}</span>
                    </div>
                    {@render bar(section.overall, true)}
                </div>

                <div class="flex flex-col">
                    {#each section.bars as b (b.account)}
                        <button
                            type="button"
                            class="border-base-content/5 hover:bg-base-200/60 flex w-full cursor-pointer flex-col gap-1.5 rounded border-b px-1 py-2.5 text-left transition-colors last:border-b-0"
                            title="View {b.account} in the journal"
                            onclick={() => openInJournal(b.account)}
                        >
                            <div class="flex flex-wrap items-baseline justify-between gap-x-3">
                                <span class="truncate font-medium">{b.account}</span>
                                <span class="flex items-baseline gap-2 text-sm whitespace-nowrap">
                                    <span class="font-mono tabular-nums">{b.spentText} / {b.budgetText}</span>
                                    <span class="font-medium {stateText[b.state]}">{b.remainderText}</span>
                                    {#if b.pct !== null}<span class="text-base-content/40 w-10 text-right tabular-nums">{b.pct}%</span>{/if}
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
