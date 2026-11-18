<script lang="ts">
    // A month of posted recognition entries as ONE journal row (plan 45 D8):
    // "Recognition · Nov 2026 · 37 entries · $41,250.00". The engine serves
    // it (its `txn` is the month's latest entry) and, when expanded, serves the
    // members as ordinary rows right under it; this row only says what it
    // stands for and offers ▸/▾ (the table owns which months are expanded, and
    // `l` and Enter toggle it from the keyboard). Same height as any row, so
    // the list's fixed-pitch windowing holds.
    import {monthShort} from "#lib/accrual/model.js";
    import type {Transaction} from "#lib/domain/types.js";
    import {fmt} from "#lib/format/amounts.js";
    import type {RecognitionGroup} from "#lib/journalView/types.js";
    import {hint} from "#lib/keys/hint.svelte.js";
    import {rowShadow} from "#lib/review/card.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import type {ColumnConfig} from "#lib/stores/settings.svelte.js";
    import AccountsCell from "./AccountsCell.svelte";

    let {
        group,
        txn,
        columns,
        mode = "row",
        cursor = false,
        pinned = null,
        at,
        onToggle,
    }: {
        group: RecognitionGroup;
        /** The month's latest entry: the row's date and its accounts. */
        txn: Transaction;
        columns: ColumnConfig;
        mode?: "row" | "card";
        cursor?: boolean;
        /** A pinned running balance (plan 36) never groups; kept for the column count. */
        pinned?: string | null;
        at?: number;
        onToggle?: () => void;
    } = $props();

    const label = $derived(`Recognition · ${monthShort(group.month)} · ${group.count} ${group.count === 1 ? "entry" : "entries"}`);
    const totals = $derived(group.totals.map((total) => fmt(total.commodity, total.quantity, journal.reportStyles)));
    const toggleTitle = $derived(hint(group.expanded ? "Collapse the month's entries" : "Show the month's entries", "l"));
</script>

{#snippet toggle()}
    <button
        type="button"
        class={["btn btn-square shrink-0 btn-ghost btn-xs", mode === "row" && "absolute top-1/2 right-full mr-1 -translate-y-1/2"]}
        aria-expanded={group.expanded}
        aria-label={group.expanded ? `Collapse ${monthShort(group.month)}'s recognition entries` : `Expand ${monthShort(group.month)}'s recognition entries`}
        title={toggleTitle}
        onclick={() => onToggle?.()}
        data-testid="recognition-toggle"
    >
        <ReviewIcon name={group.expanded ? "chevron-down" : "chevron-right"} class="h-3.5 w-3.5" />
    </button>
{/snippet}

{#if mode === "row"}
    <tr
        class={["h-10 [&>td:first-child]:[box-shadow:var(--row-shadow)]", cursor ? "bg-primary/25" : "bg-base-200/30 hover:bg-base-200/60"]}
        style:--row-shadow={rowShadow(null, cursor)}
        aria-current={cursor ? "true" : undefined}
        data-txn={txn.index}
        data-slot={at}
        data-recognition-group={group.month}
        data-testid="recognition-group-row"
    >
        {#if columns.date}
            <td class="py-1 font-mono text-xs whitespace-nowrap text-base-content/80">{txn.date}</td>
        {/if}
        {#if columns.status}
            <td class="py-1"></td>
        {/if}
        {#if columns.description}
            <td class="max-w-0 py-1">
                <div class="relative flex min-w-0 items-center gap-1.5">
                    {@render toggle()}
                    <button type="button" class="min-w-0 truncate text-left text-base-content/80 italic" title={label} onclick={() => onToggle?.()}>
                        {label}
                    </button>
                </div>
            </td>
        {/if}
        {#if columns.accounts}
            <td class="max-w-0 py-1 opacity-70"><AccountsCell {txn} /></td>
        {/if}
        {#if columns.amount}
            <td class="py-1 text-right font-mono text-sm tabular-nums" data-testid="recognition-group-total">
                {#each totals as total (total)}
                    <span class="block whitespace-nowrap">{total}</span>
                {/each}
            </td>
        {/if}
        {#if pinned !== null}
            <td class="py-1"></td>
            <td class="py-1"></td>
        {/if}
    </tr>
{:else}
    <!-- The narrow list's card (plan 42): the same h-20 as any card. -->
    <div class="mb-2 h-20" data-slot={at}>
        <article
            class={["card relative flex h-full flex-row items-center gap-2 overflow-hidden bg-base-200/60 px-3", cursor && "ring-2 ring-primary ring-inset"]}
            aria-current={cursor ? "true" : undefined}
            data-txn={txn.index}
            data-recognition-group={group.month}
            data-testid="recognition-group-card"
        >
            {@render toggle()}
            <button type="button" class="min-w-0 grow truncate text-left text-sm text-base-content/80 italic" title={label} onclick={() => onToggle?.()}>
                {label}
            </button>
            {#if columns.amount && totals.length > 0}
                <span class="shrink-0 font-mono text-sm whitespace-nowrap tabular-nums">
                    {totals[0]}{#if totals.length > 1}<span class="text-xs text-base-content/50"> +{totals.length - 1}</span>{/if}
                </span>
            {/if}
        </article>
    </div>
{/if}
