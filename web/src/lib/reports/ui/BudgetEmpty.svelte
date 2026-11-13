<!-- What the bars say when there are none: shared by the stacked list
     (`BudgetSummary`) and the wide table (`BudgetTable`), which carry the same
     `budget-empty` test id so the e2e suite finds it at any width. -->
<div class="rounded-box border border-base-content/10 px-4 py-10 text-center" data-testid="budget-empty">
    <p class="font-medium">No income or expense budget goals for this period.</p>
    <p class="mt-1 text-sm text-base-content/60">
        Add periodic rules (lines starting with <code class="rounded bg-base-200 px-1">~</code>) to your journal. Accounts that aren't named
        <code class="rounded bg-base-200 px-1">expenses…</code>/<code class="rounded bg-base-200 px-1">income…</code> need an
        <code class="rounded bg-base-200 px-1">account … ; type: X</code> directive to be classified.
    </p>
</div>
