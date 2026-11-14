<!-- Holdings scope bar (WP-10): account tree (fed only accounts that ever hold
     a stock commodity), include/exclude mode toggle (mode change keeps the
     selection), and the as-of date. All state lives in the holdings scope
     store; every change recomputes the derived report, no refetch. -->
<script lang="ts">
    import AccountTreeSelect from "#lib/filters/AccountTreeSelect.svelte";
    import type {GainPeriod} from "#lib/holdings/types.js";
    import {GAIN_PERIODS} from "#lib/holdings/ui/gainPeriod.js";
    import {holdingsScope} from "#lib/stores/holdings.svelte.js";

    let {accountNames}: {accountNames: string[]} = $props();

    const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
    // type=date emits "" while clearing — ignore until a full valid ISO date.
    function setAsOf(value: string): void {
        if (ISO_DATE.test(value)) holdingsScope.setAsOf(value);
    }

    const mode = $derived(holdingsScope.value.mode);
    const gainPeriod = $derived(holdingsScope.value.gainPeriod);
    const selectedAccounts = $derived([...holdingsScope.value.accounts].sort());
</script>

<div class="flex flex-col gap-2 rounded-box bg-base-200 p-2" data-testid="holdings-scope-bar">
    <div class="flex flex-wrap items-center gap-2">
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
        <!-- Below sm each of these takes a full line, labels in one column,
             rather than wrapping wherever the widths happen to break. -->
        <label class="flex w-full items-center gap-2 sm:ml-auto sm:w-auto">
            <span class="w-10 shrink-0 text-xs text-base-content/70 sm:w-auto">Period</span>
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
            <span class="w-10 shrink-0 text-xs text-base-content/70 sm:w-auto">As of</span>
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
            <span class="text-xs text-base-content/60">{mode === "include" ? "Only:" : "All except:"}</span>
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
</div>
