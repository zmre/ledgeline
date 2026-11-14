<script lang="ts">
    // Top filter bar (WP-04): date range + account tree + search in a wrapping
    // row (single row on desktop; controls wrap/stack at 375px, with the
    // account tree in a compact dropdown popover). Removable chips appear when
    // filters differ from the default. A pinned running balance (plan 36) gets
    // its own chip, and each account chip a ⚖ button that pins that account.
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {defaultFilter, filters} from "#lib/stores/filters.svelte.js";
    import {toggleSubtreeRoot} from "./treeSelect";
    import AccountTreeSelect from "./AccountTreeSelect.svelte";
    import DateRangePicker from "./DateRangePicker.svelte";
    import RunningPinChip from "./RunningPinChip.svelte";
    import SearchInput from "./SearchInput.svelte";

    let {accountNames}: {accountNames: string[]} = $props();

    /**
     * Coalescing window for account checkbox clicks.
     *
     * Each toggle re-derives the whole journal view (279 ms at 150k transactions
     * in node under "All time", and this ships in WKWebView), so selecting three
     * accounts in a row paid for three. 200 ms — the shortest of the three
     * windows in this bar, because clicks are deliberate and spaced, so a longer
     * one would defer work without coalescing any more of it.
     *
     * The delay costs the user NOTHING they can see: `pending` below drives the
     * checkboxes and the chips, so both respond on the click. Only the charts,
     * table and footer wait.
     */
    const ACCOUNT_DEBOUNCE_MS = 200;

    /** Locally-applied selection awaiting commit, or null when there is nothing pending. */
    let pending = $state<ReadonlySet<string> | null>(null);
    let timer: ReturnType<typeof setTimeout> | null = null;

    /** What the UI shows: the optimistic set while one is pending, else the committed one. */
    const accounts = $derived(pending ?? filters.value.accounts);

    function commitAccounts(): void {
        timer = null;
        if (pending === null) return;
        const next = pending;
        pending = null;
        filters.setAccounts(next);
    }

    function toggleAccount(name: string): void {
        pending = toggleSubtreeRoot(accounts, name);
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(commitAccounts, ACCOUNT_DEBOUNCE_MS);
    }

    /** Drop a queued selection — for the actions that REPLACE it outright. */
    function cancelPending(): void {
        if (timer !== null) clearTimeout(timer);
        timer = null;
        pending = null;
    }

    function clearAccounts(): void {
        cancelPending();
        filters.clearAccounts();
    }

    function resetAll(): void {
        cancelPending();
        filters.reset();
    }

    // Navigating away with a selection still queued must not discard it: the
    // checkbox showed as ticked, so the filter has to end up ticked too.
    $effect(() => () => {
        if (timer !== null) {
            clearTimeout(timer);
            commitAccounts();
        }
    });

    const pin = $derived(filters.value.pin ?? null);

    /**
     * `b`: unpin when pinned; otherwise pin the one account the filter is on.
     * Off (and so absent from the help sheet) when neither applies — with two
     * accounts selected there is no telling which one was meant.
     */
    function togglePin(): void {
        if (pin !== null) filters.unpin();
        else if (accounts.size === 1) filters.pin([...accounts][0]);
    }

    registerKeys({
        id: "journal-running-pin",
        priority: PRIORITY.widget,
        bindings: [
            {
                keys: "b",
                label: "Pin or unpin the running balance",
                group: "Filters",
                enabled: () => pin !== null || accounts.size === 1,
                run: togglePin,
            },
        ],
    });

    const dflt = defaultFilter();
    const rangeChanged = $derived(filters.value.from !== dflt.from || filters.value.to !== dflt.to);
    const hasChips = $derived(rangeChanged || accounts.size > 0 || filters.value.query !== "" || pin !== null);
    const rangeLabel = $derived(
        filters.value.from === null && filters.value.to === null ? "All dates" : `${filters.value.from ?? "…"} → ${filters.value.to ?? "…"}`
    );
    const selectedAccounts = $derived([...accounts].sort());
</script>

<div class="flex flex-col gap-2 rounded-box bg-base-200 p-2">
    <div class="flex flex-wrap items-center gap-2">
        <DateRangePicker />
        <!-- `a`, not `/`: on this page `/` belongs to the free-text SearchInput
             below, which is the box people reach for first. -->
        <AccountTreeSelect {accountNames} selected={accounts} onToggle={toggleAccount} onClear={clearAccounts} searchKey="a" />
        <div class="w-full min-w-0 sm:ml-auto sm:w-64 sm:min-w-48">
            <SearchInput />
        </div>
    </div>
    {#if hasChips}
        <div class="flex flex-wrap items-center gap-1">
            {#if rangeChanged}
                <span class="badge h-auto min-h-(--size) max-w-full gap-1 badge-outline">
                    <span class="[overflow-wrap:anywhere]">{rangeLabel}</span>
                    <button type="button" class="cursor-pointer" aria-label="Reset date range" onclick={() => filters.setRange(dflt.from, dflt.to)}>✕</button>
                </span>
            {/if}
            {#each selectedAccounts as name (name)}
                <span class="badge h-auto min-h-(--size) max-w-full gap-1 badge-outline">
                    <span class="truncate">{name}</span>
                    {#if pin?.account !== name}
                        <!-- Pins this account's running balance and keeps the filter: how you pin from inside the journal. -->
                        <button
                            type="button"
                            class="cursor-pointer opacity-60 hover:opacity-100"
                            title="Show the running balance of {name}"
                            aria-label="Show running balance of {name}"
                            onclick={() => filters.pin(name)}>⚖</button
                        >
                    {/if}
                    <button type="button" class="cursor-pointer" aria-label="Remove account filter {name}" onclick={() => toggleAccount(name)}>✕</button>
                </span>
            {/each}
            {#if filters.value.query !== ""}
                <span class="badge h-auto min-h-(--size) max-w-full gap-1 badge-outline">
                    <span class="truncate">“{filters.value.query}”</span>
                    <button type="button" class="cursor-pointer" aria-label="Clear search filter" onclick={() => filters.setQuery("")}>✕</button>
                </span>
            {/if}
            {#if pin !== null}
                <RunningPinChip {pin} />
            {/if}
            <button type="button" class="btn btn-ghost btn-xs" onclick={resetAll}>Reset all</button>
        </div>
    {/if}
</div>
