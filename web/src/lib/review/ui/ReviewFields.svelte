<script lang="ts">
    // The card's editable fields (plan 32 D6, D8): description, postings, tags
    // and the comment's free text — each label carrying the card-mode key that
    // jumps to it — plus the date, which is rarely needed and so only shows
    // once the ⋯ menu asks for it. Every field is a `data-review-field` the
    // overlay focuses by name.
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {Transaction} from "#lib/domain/types.js";
    import type {Chips} from "../card";
    import type {PayeeUse, ReviewForm, TagUse} from "../types";
    import DescriptionInput from "./DescriptionInput.svelte";
    import PostingRows from "./PostingRows.svelte";
    import TagChips from "./TagChips.svelte";

    let {
        form = $bindable(),
        txn,
        classes,
        accountNames,
        payees,
        tags,
        today,
        chips,
        selected,
        showDate,
        defaultCommodity,
        disabled = false,
        onTabChip,
        onPickChip,
        onRule,
    }: {
        form: ReviewForm;
        txn: Transaction;
        classes: AccountClasses;
        accountNames: string[];
        payees: readonly PayeeUse[];
        tags: readonly TagUse[];
        today: string;
        chips: Chips;
        selected: number | null;
        showDate: boolean;
        defaultCommodity: string;
        disabled?: boolean;
        onTabChip: (fieldValue: string) => boolean;
        onPickChip: (at: number) => void;
        onRule: (rule: number) => void;
    } = $props();
</script>

{#snippet label(text: string, key: string)}
    <span class="flex items-center gap-1.5 pt-1.5 text-xs font-medium tracking-wide text-base-content/55 uppercase">
        <kbd class="kbd w-5 justify-center kbd-xs font-normal normal-case">{key}</kbd>{text}
    </span>
{/snippet}

<div class="grid grid-cols-[6.5rem_minmax(0,1fr)] items-start gap-x-3 gap-y-3">
    {#if showDate}
        <span class="pt-1.5 text-xs font-medium tracking-wide text-base-content/55 uppercase">Date</span>
        <div data-review-field="date">
            <input type="date" class="input w-44 input-sm" bind:value={form.date} aria-label="Date" {disabled} />
        </div>
    {/if}

    {@render label("Description", "d")}
    <div data-review-field="description">
        <DescriptionInput bind:value={form.description} {payees} {today} {disabled} />
    </div>

    {@render label("Postings", "a")}
    <div>
        <PostingRows
            bind:postings={form.postings}
            {txn}
            {classes}
            {accountNames}
            {chips}
            {selected}
            {defaultCommodity}
            {disabled}
            {onTabChip}
            {onPickChip}
            {onRule}
        />
    </div>

    {@render label("Tags", "t")}
    <div data-review-field="tags">
        <TagChips bind:tags={form.tags} vocabulary={tags} {disabled} />
    </div>

    {@render label("Comment", "m")}
    <div data-review-field="comment">
        <input
            type="text"
            class="input w-full input-sm"
            bind:value={form.comment}
            placeholder="a note (free text)"
            aria-label="Comment"
            autocomplete="off"
            {disabled}
        />
    </div>
</div>
