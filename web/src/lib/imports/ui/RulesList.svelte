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
    //
    // Keyboard: j/k move a cursor between cards and J/K move the CARD, through
    // the same `moveRule` the ↑/↓ buttons use — so the tested arithmetic in
    // `reorder.ts` is reused rather than duplicated, and the buttons stay.
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {listCursor} from "$lib/ui/listCursor.svelte";
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

    // Keyed on POSITION, not on the item. These entries have no stable id
    // (`itemId` is null for a user-added rule) and object identity through a
    // `$state` proxy is exactly the trap AliasPanel documents — but more to the
    // point, position IS the identity here: it is the thing the user is
    // manipulating and the thing hledger reads.
    const cursor = listCursor<FormItem>(
        () => entries,
        (_item, at) => at
    );

    function focusCursor(): void {
        document.querySelector(`[data-rule="${cursor.index}"]`)?.scrollIntoView({block: "nearest"});
    }

    function moveCursor(delta: number): void {
        cursor.move(delta);
        focusCursor();
    }

    /** Move the CARD, and take the cursor with it — otherwise the rule slides out from under you. */
    function shift(delta: number): void {
        const at = cursor.index;
        const to = at + delta;
        if (at < 0 || to < 0 || to >= entries.length) return;
        move(at, to);
        cursor.to(to);
        focusCursor();
    }

    registerKeys({
        id: "rules-list",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: "Next rule", group: "Imports", run: () => moveCursor(1)},
            {keys: "ArrowDown", label: "Next rule", group: "Imports", run: () => moveCursor(1)},
            {keys: "k", label: "Previous rule", group: "Imports", run: () => moveCursor(-1)},
            {keys: "ArrowUp", label: "Previous rule", group: "Imports", run: () => moveCursor(-1)},
            {keys: "g g", label: "First rule", group: "Imports", run: () => (cursor.first(), focusCursor())},
            {keys: "G", label: "Last rule", group: "Imports", run: () => (cursor.last(), focusCursor())},
            {keys: "J", label: "Move rule down", group: "Imports", enabled: () => !disabled, run: () => shift(1)},
            {keys: "K", label: "Move rule up", group: "Imports", enabled: () => !disabled, run: () => shift(-1)},
            {keys: "Escape", label: "Clear the cursor", group: "Imports", run: () => cursor.clear()},
            {
                keys: "Enter",
                // Focus the card's first control rather than expanding it:
                // IfBlockCard has no collapsed state, and building one is a
                // separate feature. Handing the user to normal tabbing is honest.
                label: "Edit this rule",
                group: "Imports",
                run: () => document.querySelector<HTMLElement>(`[data-rule="${cursor.index}"] input, [data-rule="${cursor.index}"] button`)?.focus(),
            },
        ],
    });
</script>

<section class="flex flex-col gap-3">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
        <div>
            <h2 class="text-sm font-semibold tracking-tight">Rules</h2>
            <p class="text-xs text-base-content/60">
                Read top to bottom — <strong>later matches win</strong>, so a rule further down overrides one above it.
            </p>
        </div>
        <button type="button" class="btn gap-1 btn-sm" {disabled} onclick={add}>+ Add rule</button>
    </div>

    {#if entries.length === 0}
        <div class="rounded-box border border-dashed border-base-content/10 p-6 text-center" data-testid="imports-no-rules">
            <p class="text-sm text-base-content/70">No rules yet. Add one to send matching rows somewhere other than the fallback category.</p>
        </div>
    {:else}
        <div class="flex flex-col gap-2">
            {#each entries as item, at (at)}
                <!-- The cursor ring lives on a wrapper rather than on the cards,
                     so neither card component has to know the list has a cursor. -->
                <div
                    data-rule={at}
                    class="scroll-mt-2 rounded-box {cursor.index === at ? 'ring-2 ring-primary ring-offset-0' : ''}"
                    aria-current={cursor.index === at ? "true" : undefined}
                >
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
                </div>
            {/each}
        </div>
    {/if}
</section>
