<script lang="ts">
    // One side of a Find Matches card (plan 35 D9, Card mockup): the date and
    // status, the description (editable, with the review card's payee
    // completion), the real account and its amount, the clearing account the
    // money went "via", and the file the transaction is written in.
    import {formatAmount} from "#lib/domain/money.js";
    import {longDate} from "#lib/review/card.js";
    import DescriptionInput from "#lib/review/ui/DescriptionInput.svelte";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import type {PayeeUse} from "#lib/review/types.js";
    import {halfView} from "../model";
    import type {MatchHalf} from "../types";

    let {
        half,
        side,
        description = $bindable(),
        payees,
        today,
        disabled = false,
    }: {
        half: MatchHalf;
        /** "left" / "right": the test id and the field `1` / `2` focus. */
        side: "left" | "right";
        description: string;
        payees: readonly PayeeUse[];
        today: string;
        disabled?: boolean;
    } = $props();

    const view = $derived(halfView(half));
    const txn = $derived(half.txn);
    const negative = $derived(view.realAmount !== null && view.realAmount.qty.m < 0n);
</script>

<section class="flex min-w-0 flex-col gap-2 p-5 sm:p-6" data-testid="match-half-{side}" data-txn={txn.index}>
    <div class="flex items-center gap-2 text-sm text-base-content/70">
        <span class="font-medium text-base-content tabular-nums">{longDate(txn.date)}</span>
        {#if txn.status === "cleared"}
            <span class="inline-flex items-center gap-1 text-success"><ReviewIcon name="check" class="h-3.5 w-3.5" /> cleared</span>
        {:else if txn.status === "pending"}
            <span class="text-warning">! pending</span>
        {/if}
    </div>
    <div data-match-field="description-{side}">
        <DescriptionInput bind:value={description} {payees} {today} {disabled} placeholder="description" />
    </div>
    {#if view.real !== null}
        <div class="flex min-w-0 items-baseline justify-between gap-3 text-sm">
            <span class="truncate font-medium" title={view.real.account}>{view.real.account}</span>
            {#if view.realAmount !== null}
                <span class={["shrink-0 font-mono tabular-nums", negative ? "text-base-content" : "text-success"]} data-testid="match-half-amount"
                    >{formatAmount(view.realAmount)}</span
                >
            {/if}
        </div>
    {/if}
    {#if view.clearing !== null}
        <div class="truncate text-xs text-base-content/55" title={view.clearing.account}>via {view.clearing.account}</div>
    {/if}
    {#if half.file !== ""}
        <div class="truncate font-mono text-xs text-base-content/45" title={half.file} data-testid="match-half-file">{half.file}</div>
    {/if}
</section>
