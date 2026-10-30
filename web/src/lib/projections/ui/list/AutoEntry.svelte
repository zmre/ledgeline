<!-- An automatic posting (`= QUERY` block), read as a sentence (plan 31 C3):

         When revenue:sales is projected:
             expenses:people-costs:sales-commission     3% of it
             → assets:mercury:checking                  ▸ rule text

     A rule the form cannot state is shown by its text, with a lock and the
     reason: it is kept, applied and saved exactly as written. The rule's own
     text is always one click away in a disclosure. -->
<script lang="ts">
    import type {AmountStyle} from "#lib/domain/types.js";
    import {autoLockedReason, autoPostingText} from "../../entryText";
    import {attachedText, type Entry} from "../../entries";
    import EntryFrame from "./EntryFrame.svelte";
    import {POSTING_GRID, type FrameProps} from "./frame";

    let {
        entry,
        frame,
        styles,
    }: {
        entry: Extract<Entry, {kind: "auto"}>;
        frame: FrameProps;
        styles: ReadonlyMap<string, AmountStyle>;
    } = $props();

    const rule = $derived(entry.rule);
    const form = $derived(rule.form);
    const locked = $derived(autoLockedReason(rule));
    const attached = $derived(attachedText(entry.item));
    /** The text the file holds: the piece's own while it is unchanged, else the rule's last render. */
    const text = $derived((entry.item.raw !== "" ? entry.item.raw : rule.raw).trimEnd());
</script>

<EntryFrame {entry} {...frame}>
    {#snippet header(id)}
        <h3 {id} class={["flex min-w-0 flex-wrap items-baseline gap-x-2 text-sm", entry.disabled && "line-through decoration-base-content/50"]}>
            {#if form !== null}
                <span class="text-base-content/70">When</span>
                <span class="font-mono text-[0.8125rem] text-base-content">{form.matchAccount === "" ? "…" : form.matchAccount}</span>
                {#if form.extraQuery.trim() !== ""}
                    <span class="font-mono text-xs text-base-content/60">and {form.extraQuery}</span>
                {/if}
                <span class="text-base-content/70">is projected</span>
                {#if form.comment !== ""}
                    <span class="min-w-0 truncate font-mono text-xs text-base-content/45 italic">; {form.comment}</span>
                {/if}
            {:else}
                <span class="font-mono text-[0.8125rem] text-base-content/80"><span class="text-base-content/35">=</span> {rule.query}</span>
            {/if}
        </h3>
        {#if locked !== null}
            <span class="inline-flex items-center gap-1 text-xs text-warning" title={locked}
                ><span aria-hidden="true">🔒</span>{rule.unsupported === null ? "edit in the file" : "never fires"}</span
            >
        {/if}
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
        {#if form !== null}
            <div class={POSTING_GRID} data-testid="entry-postings">
                {#each form.postings as posting, at (at)}
                    <span class="min-w-0 truncate text-base-content/90" title={posting.account}
                        >{posting.account === "" ? "(no account)" : posting.account}</span
                    >
                    <span class="text-right whitespace-nowrap text-base-content/80">{autoPostingText(posting, styles)}</span>
                    <span class="hidden lg:block"></span>
                {/each}
                {#if form.counter !== null}
                    <span class="col-span-full min-w-0 truncate text-base-content/50" data-testid="entry-counter"
                        ><span aria-hidden="true">→</span><span class="sr-only">balanced against</span> {form.counter}</span
                    >
                {/if}
            </div>
        {:else if locked !== null}
            <p class="mt-1 pl-6 text-xs text-warning/90">{locked}</p>
        {/if}
        {#if text !== ""}
            <details class="group/raw mt-1 pl-6" open={form === null}>
                <summary class="cursor-pointer text-xs text-base-content/45 select-none hover:text-base-content/70">rule text</summary>
                <pre class="mt-1 overflow-x-auto rounded bg-base-100/60 px-2 py-1 font-mono text-xs text-base-content/70">{text}</pre>
            </details>
        {/if}
    {/snippet}
</EntryFrame>
