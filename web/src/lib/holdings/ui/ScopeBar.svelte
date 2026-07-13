<!-- Holdings scope bar (WP-10): account tree (fed only accounts that ever hold
     a stock commodity), include/exclude mode toggle (mode change keeps the
     selection), and the as-of date. All state lives in the holdings scope
     store; every change recomputes the derived report, no refetch. -->
<script lang="ts">
    import AccountTreeSelect from "$lib/filters/AccountTreeSelect.svelte";
    import {holdingsScope} from "$lib/stores/holdings.svelte";

    let {accountNames}: {accountNames: string[]} = $props();

    const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
    // type=date emits "" while clearing — ignore until a full valid ISO date.
    function setAsOf(value: string): void {
        if (ISO_DATE.test(value)) holdingsScope.setAsOf(value);
    }

    const mode = $derived(holdingsScope.value.mode);
    const selectedAccounts = $derived([...holdingsScope.value.accounts].sort());
</script>

<div class="bg-base-200 rounded-box flex flex-col gap-2 p-2" data-testid="holdings-scope-bar">
    <div class="flex flex-wrap items-center gap-2">
        <AccountTreeSelect
            {accountNames}
            selected={holdingsScope.value.accounts}
            onToggle={(name) => holdingsScope.toggleAccount(name)}
            onClear={() => holdingsScope.clear()}
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
        <label class="ml-auto flex items-center gap-2">
            <span class="text-base-content/70 text-xs">As of</span>
            <input
                type="date"
                class="input input-sm w-40"
                value={holdingsScope.value.asOf}
                onchange={(e) => setAsOf(e.currentTarget.value)}
                aria-label="As of date"
            />
        </label>
    </div>
    {#if selectedAccounts.length > 0}
        <div class="flex flex-wrap items-center gap-1">
            <span class="text-base-content/60 text-xs">{mode === "include" ? "Only:" : "All except:"}</span>
            {#each selectedAccounts as name (name)}
                <span class="badge badge-outline max-w-full gap-1">
                    <span class="truncate">{name}</span>
                    <button type="button" class="cursor-pointer" aria-label="Remove account {name} from scope" onclick={() => holdingsScope.toggleAccount(name)}
                        >✕</button
                    >
                </span>
            {/each}
        </div>
    {/if}
</div>
