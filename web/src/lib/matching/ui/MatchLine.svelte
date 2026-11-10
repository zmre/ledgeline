<script lang="ts">
    // One posting on one line (plan 35 Amendment A): the date, the
    // description, the real account, the gap, any flag, the file, and the
    // amount that moved on the real side. A combination card's group, and the
    // focused view's partners, nearby postings and links, are lists of these.
    import {formatAmount} from "#lib/domain/money.js";
    import {longDate} from "#lib/review/card.js";
    import {flowAmount, gapLabel, halfView} from "../model";
    import type {MatchHalf} from "../types";

    let {
        half,
        gapDays = null,
        flags = [],
    }: {
        half: MatchHalf;
        /** Days from the posting being matched, when it is a partner. */
        gapDays?: number | null;
        /** "outside the 10-day window", "before the lookback". */
        flags?: readonly string[];
    } = $props();

    const real = $derived(halfView(half).real);
    const amount = $derived(flowAmount(half));
    const negative = $derived(amount !== null && amount.qty.m < 0n);
</script>

<span class="flex min-w-0 grow items-baseline gap-3 text-sm" data-testid="match-line" data-txn={half.txn.index}>
    <span class="shrink-0 tabular-nums">{longDate(half.txn.date)}</span>
    <span class="min-w-0 truncate" title={half.txn.description}>{half.txn.description || "(no description)"}</span>
    {#if real !== null}
        <span class="min-w-0 truncate text-xs text-base-content/55" title={real.account}>{real.account}</span>
    {/if}
    <span class="grow"></span>
    {#if gapDays !== null}
        <span class="shrink-0 text-xs whitespace-nowrap text-base-content/50" data-testid="match-line-gap">{gapLabel(gapDays)}</span>
    {/if}
    {#each flags as flag (flag)}
        <span class="badge shrink-0 badge-xs whitespace-nowrap badge-warning" data-testid="match-line-flag">{flag}</span>
    {/each}
    {#if half.file !== ""}
        <span class="hidden shrink-0 font-mono text-xs text-base-content/45 sm:inline" title={half.file}>{half.file}</span>
    {/if}
    <span class={["shrink-0 font-mono tabular-nums", !negative && "text-success"]} data-testid="match-line-amount"
        >{amount === null ? "—" : formatAmount(amount)}</span
    >
</span>
