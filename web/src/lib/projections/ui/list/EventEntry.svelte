<!-- A one-off (`~ YYYY-MM-DD` block), read like its text: the date and what
     it is, then each posting with its account's type badge and its SIGNED
     amount (an event's direction is the user's to state, plan 22 amendment
     19), then the counter-account — greyed when it is the default, and absent
     when the postings balance on their own. -->
<script lang="ts">
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {amountText} from "../../entryText";
    import {attachedText, type Entry} from "../../entries";
    import EntryFrame from "./EntryFrame.svelte";
    import PostingAccount from "./PostingAccount.svelte";
    import {POSTING_GRID, POSTING_NOTES, type FrameProps} from "./frame";

    let {
        entry,
        frame,
        declared,
        styles,
    }: {
        entry: Extract<Entry, {kind: "event"}>;
        frame: FrameProps;
        /** The scenario's account types (`scenarioClasses`), for the badges. */
        declared: AccountClasses;
        styles: ReadonlyMap<string, AmountStyle>;
    } = $props();

    const event = $derived(entry.event);
    const attached = $derived(attachedText(entry.item));
    const counter = $derived(entry.counter);
</script>

<EntryFrame {entry} {...frame}>
    {#snippet header(id)}
        <h3 {id} class={["flex min-w-0 flex-wrap items-baseline gap-x-3 text-sm", entry.disabled && "line-through decoration-base-content/50"]}>
            <span class="font-mono text-[0.8125rem] whitespace-nowrap text-base-content/75"><span class="text-base-content/35">~</span> {event.date}</span>
            {#if event.description !== ""}
                <span class="min-w-0 font-medium break-words text-base-content">{event.description}</span>
            {/if}
            {#if event.comment !== ""}
                <span class="min-w-0 truncate font-mono text-xs text-base-content/45 italic">; {event.comment.split("\n").join(" · ")}</span>
            {/if}
        </h3>
    {/snippet}
    {#snippet body()}
        {#if attached !== ""}
            <p class="mt-0.5 pl-3 font-mono text-xs whitespace-pre-wrap text-base-content/50 italic" data-testid="entry-attached">
                {attached
                    .split("\n")
                    .map((line) => `; ${line}`)
                    .join("\n")}
            </p>
        {/if}
        <div class={POSTING_GRID} data-testid="entry-postings">
            {#each event.postings as posting, at (at)}
                {@const amount = amountText(posting.amount.commodity, posting.amount.quantity, styles)}
                <PostingAccount account={posting.account} classes={declared} class="text-base-content/90" />
                <!-- Neutral, minus or not: a minus is a direction here, not a warning. -->
                <span class="text-right whitespace-nowrap" data-testid="entry-amount">{amount}</span>
                <span class="min-w-0 truncate text-xs text-base-content/55 italic {POSTING_NOTES}" title={posting.comment}
                    >{#if posting.comment !== ""}; {posting.comment.split("\n").join(" · ")}{/if}</span
                >
            {/each}
            {#if counter !== null}
                <PostingAccount
                    account={counter.account ?? ""}
                    classes={declared}
                    dim={counter.inferred}
                    class="col-span-full text-base-content/50"
                    testid="entry-counter"
                >
                    <span aria-hidden="true">→</span><span class="sr-only">balanced against</span>
                    {#if !counter.inferred}
                        {counter.account}
                    {:else}
                        <span class="italic">{counter.account ?? "the default cash account"}</span>
                        <span class="font-sans text-xs text-base-content/35">(default)</span>
                    {/if}
                </PostingAccount>
            {/if}
        </div>
    {/snippet}
</EntryFrame>
