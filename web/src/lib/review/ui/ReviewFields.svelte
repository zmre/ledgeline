<script lang="ts">
    // The card's editable fields (plan 32 D6, D8, D33): description, postings,
    // tags and the comment's free text, each label carrying the card-mode key
    // that jumps to it. The date, status, code and secondary date are the
    // details row in the hero (`ReviewDetails`). Every field is a
    // `data-review-field` the overlay focuses by name.
    //
    // The comment is a growing textarea, not an input: it is the free text of
    // the WHOLE transaction comment, one line per comment line (p7 amendment
    // 17), and an input would strip the line breaks.
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {Transaction} from "#lib/domain/types.js";
    import type {Chips} from "../card";
    import type {PayeeUse, ReviewForm, TagUse} from "../types";
    import DescriptionInput from "./DescriptionInput.svelte";
    import GrowingTextarea from "./GrowingTextarea.svelte";
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
        commodities,
        defaultCommodity,
        disabled = false,
        onTabChip,
        onPickChip,
        onRule,
        onPayeePick,
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
        /** The journal's commodities, most used first, for the posting rows' commodity fields. */
        commodities: readonly string[];
        defaultCommodity: string;
        disabled?: boolean;
        onTabChip: (fieldValue: string) => boolean;
        onPickChip: (at: number) => void;
        onRule: (rule: number) => void;
        /** A payee was picked from the description's list. */
        onPayeePick?: (payee: string) => void;
    } = $props();
</script>

<!-- Labels sit to the left from `md`; below it (a phone) they go above their
     fields, which then get the whole width (plan 42). -->
{#snippet label(text: string, key: string)}
    <span class="mt-2 flex items-center gap-1.5 text-xs font-medium tracking-wide text-base-content/55 uppercase first:mt-0 md:mt-0 md:pt-1.5">
        <kbd class="kbd w-5 justify-center kbd-xs font-normal normal-case">{key}</kbd>{text}
    </span>
{/snippet}

<div class="grid grid-cols-1 items-start gap-x-3 gap-y-1 md:grid-cols-[6.5rem_minmax(0,1fr)] md:gap-y-3">
    {@render label("Description", "d")}
    <div data-review-field="description">
        <DescriptionInput bind:value={form.description} {payees} {today} {disabled} onPick={onPayeePick} />
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
            {commodities}
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
        <GrowingTextarea bind:value={form.comment} placeholder="a note (free text)" aria-label="Comment" autocomplete="off" {disabled} />
    </div>
</div>
