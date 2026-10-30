<!-- A one-off (`~ YYYY-MM-DD` block), read like its text: the date and what
     it is, then each posting with its SIGNED amount (an event's direction is
     the user's to state, plan 22 amendment 19), then the counter-account. -->
<script lang="ts">
    import type {AmountStyle} from "#lib/domain/types.js";
    import {amountText} from "../../entryText";
    import {attachedText, type Entry} from "../../entries";
    import type {ForecastDefaults} from "../../types";
    import EntryFrame from "./EntryFrame.svelte";
    import {POSTING_GRID, POSTING_NOTES, type FrameProps} from "./frame";

    let {
        entry,
        frame,
        styles,
        defaults,
    }: {
        entry: Extract<Entry, {kind: "event"}>;
        frame: FrameProps;
        styles: ReadonlyMap<string, AmountStyle>;
        defaults: ForecastDefaults | null;
    } = $props();

    const event = $derived(entry.event);
    const attached = $derived(attachedText(entry.item));
    const counterDefault = $derived(defaults?.events.get(event.id) ?? null);
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
                <span class="min-w-0 truncate text-base-content/90" title={posting.account}>{posting.account === "" ? "(no account)" : posting.account}</span>
                <span class={["text-right whitespace-nowrap", amount.startsWith("−") && "text-error"]}>{amount}</span>
                <span class="min-w-0 truncate text-xs text-base-content/55 italic {POSTING_NOTES}" title={posting.comment}
                    >{#if posting.comment !== ""}; {posting.comment.split("\n").join(" · ")}{/if}</span
                >
            {/each}
            <span class="col-span-full min-w-0 truncate text-base-content/50" data-testid="entry-counter">
                <span aria-hidden="true">→</span><span class="sr-only">balanced against</span>
                {#if event.counter !== null}
                    {event.counter}
                {:else}
                    <span class="italic">{counterDefault ?? "the default cash account"}</span>
                    <span class="font-sans text-xs text-base-content/35">(default)</span>
                {/if}
            </span>
        </div>
    {/snippet}
</EntryFrame>
