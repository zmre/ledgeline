<script lang="ts">
    // The ordered rules list: everything the preference panels do not speak for,
    // in the order hledger will read it.
    //
    // ORDER IS SEMANTICS HERE, which is why the heading says so. hledger applies
    // every matching conditional block in turn, so a later `account2` overwrites
    // an earlier one — "later matches win". A list that let you reorder without
    // saying that would be a list of controls with no visible consequence.
    //
    // The list is the COMPLEMENT of what the panels claim (`ruleIndices`), not a
    // filter for `ifBlock`s. That is what guarantees no item in the file is
    // invisible on this screen: a comment, an `include`, a duplicate `skip`, an
    // assignment this GUI has no editor for — each gets a read-only card here
    // rather than silently existing only in the bytes.
    //
    // One consequence, and it is deliberate: positions are positions IN THIS
    // LIST, comment cards included. "Move rule 3 up" swaps the third card with
    // the second whatever the second is, which is what someone watching the list
    // sees happen. Numbering rules separately from the cards between them would
    // put a number on screen that no button agrees with.
    import {appendRule, blankRule, describeItem, moveRule, ruleIndices, type FormItem} from "../model";
    import IfBlockCard from "./IfBlockCard.svelte";
    import KeptItemCard from "./KeptItemCard.svelte";

    let {
        items,
        accountNames,
        csvFields,
        fallbackAccount,
        onChange,
        disabled,
    }: {
        items: FormItem[];
        accountNames: string[];
        csvFields: string[];
        /** The file's `account2`, seeded into a new rule so the commonest edit is one field. */
        fallbackAccount: string;
        onChange: (items: FormItem[]) => void;
        disabled: boolean;
    } = $props();

    const slots = $derived(ruleIndices(items));
    const entries = $derived(slots.map((index) => items[index]).filter((item): item is FormItem => item !== undefined));

    function move(from: number, to: number): void {
        onChange(moveRule(items, from, to));
    }

    function remove(at: number): void {
        const index = slots[at];
        if (index === undefined) return;
        onChange(items.filter((_, position) => position !== index));
    }

    function add(): void {
        onChange(appendRule(items, blankRule(fallbackAccount)));
    }
</script>

<section class="flex flex-col gap-3">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
        <div>
            <h2 class="text-sm font-semibold tracking-tight">Rules</h2>
            <p class="text-base-content/60 text-xs">
                Read top to bottom — <strong>later matches win</strong>, so a rule further down overrides one above it.
            </p>
        </div>
        <button type="button" class="btn btn-sm gap-1" {disabled} onclick={add}>+ Add rule</button>
    </div>

    {#if entries.length === 0}
        <div class="border-base-content/10 rounded-box border border-dashed p-6 text-center" data-testid="imports-no-rules">
            <p class="text-base-content/70 text-sm">No rules yet. Add one to send matching rows somewhere other than the fallback category.</p>
        </div>
    {:else}
        <div class="flex flex-col gap-2">
            {#each entries as item, at (at)}
                {#if item.kind === "ifBlock"}
                    <IfBlockCard
                        rule={item}
                        position={at + 1}
                        total={entries.length}
                        {accountNames}
                        {csvFields}
                        {disabled}
                        onMoveUp={() => move(at, at - 1)}
                        onMoveDown={() => move(at, at + 1)}
                        onRemove={() => remove(at)}
                    />
                {:else}
                    <KeptItemCard
                        summary={describeItem(item)}
                        position={at + 1}
                        total={entries.length}
                        {disabled}
                        onMoveUp={() => move(at, at - 1)}
                        onMoveDown={() => move(at, at + 1)}
                    />
                {/if}
            {/each}
        </div>
    {/if}
</section>
