<script lang="ts">
    // Who and how much, front and centre (plan 32 D6): the payee large, the
    // `| note` muted beside it, the amount in big tabular figures coloured by
    // which way the money went, and under them the date, the funding account
    // and the status. A Recents card a similar-apply touched says so (D5a).
    //
    // `peek` is the same hero on a card waiting beneath the top one. It carries
    // no test ids, so `review-payee` and the rest always name the top card.
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import {formatAmount, neg} from "#lib/domain/money.js";
    import type {Transaction} from "#lib/domain/types.js";
    import {heroOf, longDate, type HeroTone} from "../card";

    let {txn, classes, updatedFrom = null, peek = false}: {txn: Transaction; classes: AccountClasses; updatedFrom?: number | null; peek?: boolean} = $props();

    const hero = $derived(heroOf(txn, classes));
    const TONE: Record<HeroTone, string> = {out: "text-base-content", in: "text-success", transfer: "text-info"};
    const SIGN: Record<HeroTone, string> = {out: "−", in: "+", transfer: ""};
    const id = (name: string): string | undefined => (peek ? undefined : name);
    const shown = $derived(hero.amounts.map((amount) => formatAmount(amount.qty.m < 0n ? {...amount, qty: neg(amount.qty)} : amount)));
</script>

<header class="flex items-start justify-between gap-6" data-testid={id("review-hero")} data-tone={hero.tone}>
    <div class="min-w-0">
        <h2 class="text-2xl leading-tight font-semibold tracking-tight break-words text-base-content sm:text-[2rem]" data-testid={id("review-payee")}>
            {hero.payee || "(no description)"}
            {#if hero.note !== ""}<span class="font-normal text-base-content/45">| {hero.note}</span>{/if}
        </h2>
        <p class="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-base-content/60">
            <span class="tabular-nums">{longDate(txn.date)}</span>
            {#if hero.funding !== null}
                <span aria-hidden="true" class="text-base-content/25">·</span>
                <span>{hero.funding.preposition} <span class="font-medium text-base-content/85">{hero.funding.account}</span></span>
            {/if}
            {#if txn.status !== "unmarked"}
                <span aria-hidden="true" class="text-base-content/25">·</span>
                <span class={txn.status === "pending" ? "text-warning" : ""}>{txn.status}</span>
            {/if}
        </p>
        {#if updatedFrom !== null && !peek}
            <p class="mt-2 inline-flex items-center gap-1.5 rounded-full bg-info/10 px-2 py-0.5 text-xs text-info" data-testid="review-updated-from">
                updated from card {updatedFrom}
            </p>
        {/if}
    </div>
    <div class="shrink-0 text-right">
        {#each shown as text, at (at)}
            <div class="font-mono text-3xl leading-none font-light tracking-tight tabular-nums sm:text-4xl {TONE[hero.tone]}" data-testid={id("review-amount")}>
                {SIGN[hero.tone]}{text}
            </div>
        {:else}
            <div class="font-mono text-3xl leading-none text-base-content/30">—</div>
        {/each}
    </div>
</header>
