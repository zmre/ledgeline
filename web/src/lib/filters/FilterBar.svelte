<script lang="ts">
    // Top filter bar (WP-04): date range + account tree + search in a wrapping
    // row (single row on desktop; controls wrap/stack at 375px, with the
    // account tree in a compact dropdown popover). Removable chips appear when
    // filters differ from the default (current month, all accounts, no query).
    import {defaultFilter, filters} from "$lib/stores/filters.svelte";
    import AccountTreeSelect from "./AccountTreeSelect.svelte";
    import DateRangePicker from "./DateRangePicker.svelte";
    import SearchInput from "./SearchInput.svelte";

    let {accountNames}: {accountNames: string[]} = $props();

    const dflt = defaultFilter();
    const rangeChanged = $derived(filters.value.from !== dflt.from || filters.value.to !== dflt.to);
    const hasChips = $derived(rangeChanged || filters.value.accounts.size > 0 || filters.value.query !== "");
    const rangeLabel = $derived(
        filters.value.from === null && filters.value.to === null ? "All dates" : `${filters.value.from ?? "…"} → ${filters.value.to ?? "…"}`
    );
    const selectedAccounts = $derived([...filters.value.accounts].sort());
</script>

<div class="bg-base-200 rounded-box flex flex-col gap-2 p-2">
    <div class="flex flex-wrap items-center gap-2">
        <DateRangePicker />
        <AccountTreeSelect {accountNames} />
        <div class="w-full min-w-48 sm:ml-auto sm:w-64">
            <SearchInput />
        </div>
    </div>
    {#if hasChips}
        <div class="flex flex-wrap items-center gap-1">
            {#if rangeChanged}
                <span class="badge badge-outline gap-1">
                    {rangeLabel}
                    <button type="button" class="cursor-pointer" aria-label="Reset date range" onclick={() => filters.setRange(dflt.from, dflt.to)}>✕</button>
                </span>
            {/if}
            {#each selectedAccounts as name (name)}
                <span class="badge badge-outline max-w-full gap-1">
                    <span class="truncate">{name}</span>
                    <button type="button" class="cursor-pointer" aria-label="Remove account filter {name}" onclick={() => filters.toggleAccount(name)}>✕</button
                    >
                </span>
            {/each}
            {#if filters.value.query !== ""}
                <span class="badge badge-outline max-w-full gap-1">
                    <span class="truncate">“{filters.value.query}”</span>
                    <button type="button" class="cursor-pointer" aria-label="Clear search filter" onclick={() => filters.setQuery("")}>✕</button>
                </span>
            {/if}
            <button type="button" class="btn btn-ghost btn-xs" onclick={() => filters.reset()}>Reset all</button>
        </div>
    {/if}
</div>
