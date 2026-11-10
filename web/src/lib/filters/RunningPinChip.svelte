<script lang="ts">
    // The pinned running balance's chip (plan 36), in the filter bar's chip row:
    //
    //   ⚖ Running balance: assets:checking  [$ ▾]  ✗ first failure 2026-03-31  ✕
    //
    // Styled apart from the filter chips (primary, not outline-neutral) because
    // it changes what the rows SHOW, not which rows there are. Its ✕ removes the
    // columns and nothing else: the account filter stays as it is.
    import {firstFailure, pinnedCommodity, widenRange} from "#lib/journal/runningBalance.js";
    import {filters, type RunningPin} from "#lib/stores/filters.svelte.js";
    import {problems} from "#lib/stores/problems.svelte.js";
    import {running} from "#lib/stores/running.svelte.js";

    let {pin}: {pin: RunningPin} = $props();

    const info = $derived(running.for(pin.account));
    const refused = $derived(running.refused(pin.account));
    const commodity = $derived(pinnedCommodity(info, pin));
    const commodities = $derived(info?.commodities.map((c) => c.commodity) ?? []);
    const failure = $derived(firstFailure(info, commodity));

    /** Picking the default stores no commodity at all, so the URL keeps meaning "the usual one". */
    function choose(next: string): void {
        filters.setPinCommodity(next === commodities[0] ? null : next);
    }

    /**
     * Scroll to the first failing assertion, the way the Problems drawer jumps:
     * widen the date range only as far as needed, then ask the table to reveal
     * and pulse the row. The account filter and search are left alone.
     */
    function jumpToFailure(): void {
        if (failure === null) return;
        const current = filters.value;
        const {from, to} = widenRange(current.from, current.to, failure.date);
        if (from !== current.from || to !== current.to) filters.setRange(from, to);
        problems.requestFocus(failure.tindex);
    }
</script>

<span class="badge h-auto max-w-full flex-wrap gap-1.5 badge-outline py-0.5 badge-primary" data-testid="running-pin-chip">
    <span aria-hidden="true">⚖</span>
    <span class="truncate">Running balance: <span class="font-mono">{pin.account}</span></span>
    {#if refused}
        <span class="text-error" data-testid="running-pin-refused">not in this journal</span>
    {:else if commodities.length > 1 && commodity !== null}
        <select
            class="select h-5 min-h-0 w-auto select-xs"
            aria-label="Running balance commodity"
            value={commodity}
            onchange={(e) => choose(e.currentTarget.value)}
        >
            {#each commodities as c (c)}
                <option value={c}>{c}</option>
            {/each}
        </select>
    {/if}
    {#if failure !== null}
        <button type="button" class="cursor-pointer text-error underline-offset-2 hover:underline" onclick={jumpToFailure} data-testid="running-first-failure">
            ✗ first failure {failure.date}
        </button>
    {/if}
    <button type="button" class="cursor-pointer" aria-label="Remove running balance" onclick={() => filters.unpin()}>✕</button>
</span>
