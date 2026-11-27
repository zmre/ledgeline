<!-- Holdings scope bar (WP-10): account tree (fed only accounts that ever hold
     a stock commodity), include/exclude mode toggle (mode change keeps the
     selection), the holdings filter (plan 47: which rows the chart and the
     stat tiles show — the tab's report rows, passed in), and the as-of date.
     All state lives in the holdings scope store. -->
<script lang="ts">
    import FoldedControls from "#ui/components/FoldedControls.svelte";
    import AccountTreeSelect from "#lib/filters/AccountTreeSelect.svelte";
    import type {GainPeriod} from "#lib/holdings/types.js";
    import {GAIN_PERIODS} from "#lib/holdings/ui/gainPeriod.js";
    import {holdingsScope} from "#lib/stores/holdings.svelte.js";
    import HoldingsSelect from "./HoldingsSelect.svelte";
    import type {HoldingOption} from "./selection";
    import {scopeSummary} from "./view";

    let {
        accountNames,
        holdingOptions = [],
        holdingNoun = "holdings",
    }: {
        accountNames: string[];
        /** The tab's report rows, as the Holdings dropdown lists them (empty before the report lands). */
        holdingOptions?: readonly HoldingOption[];
        /** What a row is on this tab, for the dropdown's wording. */
        holdingNoun?: string;
    } = $props();

    const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
    // type=date emits "" while clearing — ignore until a full valid ISO date.
    function setAsOf(value: string): void {
        if (ISO_DATE.test(value)) holdingsScope.setAsOf(value);
    }

    const mode = $derived(holdingsScope.value.mode);
    const gainPeriod = $derived(holdingsScope.value.gainPeriod);
    const selectedAccounts = $derived([...holdingsScope.value.accounts].sort());
    const selectedHoldings = $derived([...holdingsScope.value.holdings].sort());
    const holdingLabel = (key: string): string => holdingOptions.find((option) => option.key === key)?.label ?? key;
    const summary = $derived(scopeSummary(holdingsScope.value, GAIN_PERIODS.find((option) => option.value === gainPeriod)?.label ?? gainPeriod));
</script>

<!-- Below `sm` the bar folds behind a line saying what it is set to. -->
<FoldedControls id="holdings-scope" {summary} testid="holdings-scope-bar" class="flex-col gap-2 rounded-box bg-base-200 p-2">
    <!-- `relative`: below `md` the Accounts panel hangs from this row. -->
    <div class="relative flex flex-wrap items-center gap-2">
        <!-- `/` here: holdings has no free-text search box, so the account tree
             is the search on this page. -->
        <AccountTreeSelect
            {accountNames}
            selected={holdingsScope.value.accounts}
            onToggle={(name) => holdingsScope.toggleAccount(name)}
            onClear={() => holdingsScope.clear()}
            searchKey="/"
        />
        <div class="join" role="group" aria-label="Scope mode">
            <button
                type="button"
                class="btn join-item btn-sm {mode === 'include' ? 'btn-active' : ''}"
                aria-pressed={mode === "include"}
                title="Show only the selected accounts (empty selection = everything)"
                onclick={() => holdingsScope.setMode("include")}
            >
                Only
            </button>
            <button
                type="button"
                class="btn join-item btn-sm {mode === 'exclude' ? 'btn-active' : ''}"
                aria-pressed={mode === "exclude"}
                title="Show everything except the selected accounts"
                onclick={() => holdingsScope.setMode("exclude")}
            >
                All except
            </button>
        </div>
        <!-- `F`: "f" filters the chart to the table's cursor row (HoldingsTable), so its capital opens the full list. -->
        <HoldingsSelect
            options={holdingOptions}
            selected={holdingsScope.value.holdings}
            onToggle={(key) => holdingsScope.toggleHolding(key)}
            onClear={() => holdingsScope.clearHoldings()}
            noun={holdingNoun}
            openKey="F"
        />
        <!-- Below sm each of these takes a full line, labels in one column,
             rather than wrapping wherever the widths happen to break. -->
        <label class="flex w-full items-center gap-2 sm:ml-auto sm:w-auto">
            <span class="w-10 shrink-0 text-xs text-muted sm:w-auto">Period</span>
            <select
                class="select min-w-0 flex-1 select-sm sm:w-44 sm:flex-none"
                value={gainPeriod}
                onchange={(e) => holdingsScope.setGainPeriod(e.currentTarget.value as GainPeriod)}
                aria-label="Gain period"
                title="The window for the gain/loss figures and the value-over-time chart"
            >
                {#each GAIN_PERIODS as opt (opt.value)}
                    <option value={opt.value}>{opt.label}</option>
                {/each}
            </select>
        </label>
        <label class="flex w-full items-center gap-2 sm:w-auto">
            <span class="w-10 shrink-0 text-xs text-muted sm:w-auto">As of</span>
            <input
                type="date"
                class="input min-w-0 flex-1 input-sm sm:w-40 sm:flex-none"
                value={holdingsScope.value.asOf}
                onchange={(e) => setAsOf(e.currentTarget.value)}
                aria-label="As of date"
            />
        </label>
    </div>
    {#if selectedAccounts.length > 0}
        <div class="flex flex-wrap items-center gap-1">
            <span class="text-xs text-muted">{mode === "include" ? "Only:" : "All except:"}</span>
            {#each selectedAccounts as name (name)}
                <span class="badge max-w-full gap-1 badge-outline">
                    <span class="truncate">{name}</span>
                    <button type="button" class="cursor-pointer" aria-label="Remove account {name} from scope" onclick={() => holdingsScope.toggleAccount(name)}
                        >✕</button
                    >
                </span>
            {/each}
        </div>
    {/if}
    {#if selectedHoldings.length > 0}
        <div class="flex flex-wrap items-center gap-1" data-testid="holdings-filter-chips">
            <span class="text-xs text-muted">Chart &amp; totals:</span>
            {#each selectedHoldings as key (key)}
                <span class="badge max-w-full gap-1 badge-outline badge-primary">
                    <span class="truncate" title={key}>{holdingLabel(key)}</span>
                    <button type="button" class="cursor-pointer" aria-label="Stop charting {holdingLabel(key)}" onclick={() => holdingsScope.toggleHolding(key)}
                        >✕</button
                    >
                </span>
            {/each}
        </div>
    {/if}
</FoldedControls>
