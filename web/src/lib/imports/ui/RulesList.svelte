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
    //
    // # Display and edit are two different renderings of the same rule
    //
    // Every rule is a one-line summary (`RuleSummaryCard`) until it is opened,
    // and exactly one can be open at a time. Rendering all of them as full
    // editors — which is what this list used to do — made a file of any size a
    // very long column of identical controls with no way to find anything in it.
    //
    // WHICH rule is open lives here rather than in `EditRulesPanel`, beside the
    // keyboard cursor and for the same reason: it is a fact about how this list
    // is being looked at, not about the document, and nothing outside this
    // component can act on it. The panel keys this whole subtree on
    // `form.id#formEpoch`, so switching files or reverting rebuilds the list and
    // takes the open card with it. A SAVE is the one thing the list cannot see
    // for itself, so it is passed `savedAt` and closes the card when it moves —
    // the edit has landed, and leaving the editor open over it invites a second,
    // accidental edit of a rule the user is finished with.
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {listCursor} from "$lib/ui/listCursor.svelte";
    import {tick} from "svelte";
    import {appendRule, blankRule, describeIfBlock, describeItem, moveRule, ruleIndices, type FormItem} from "../model";
    import IfBlockCard from "./IfBlockCard.svelte";
    import KeptItemCard from "./KeptItemCard.svelte";
    import RuleSummaryCard from "./RuleSummaryCard.svelte";

    let {
        items,
        accountNames,
        csvFields,
        fallbackAccount,
        savedAt,
        onChange,
        disabled,
    }: {
        items: FormItem[];
        accountNames: string[];
        csvFields: string[];
        /** The file's `account2`, seeded into a new rule so the commonest edit is one field. */
        fallbackAccount: string;
        /** When the last successful save landed, or null. Moves once per save; closes the open rule. */
        savedAt: number | null;
        onChange: (items: FormItem[]) => void;
        disabled: boolean;
    } = $props();

    const slots = $derived(ruleIndices(items));
    const entries = $derived(slots.map((index) => items[index]).filter((item): item is FormItem => item !== undefined));

    /** The rules-list position of the rule being edited, or null when they are all collapsed. */
    let openAt = $state<number | null>(null);

    // A LATCH on the value, not on truthiness: `savedAt` stays set after a save,
    // and reacting to "is set" would make the card impossible to open again
    // until the next edit. Same shape as the panel's own `seededFrom`.
    let closedFor: number | null = null;
    $effect(() => {
        if (savedAt !== null && savedAt !== closedFor) {
            closedFor = savedAt;
            openAt = null;
        }
    });

    /**
     * Where the open card ends up once the entry at `from` has moved to `to`.
     *
     * Moving a rule must not silently open a different one: positions are the
     * only identity these entries have (a rule the user just added has no id at
     * all), so the one being edited has to be tracked through the same shuffle
     * its card goes through.
     */
    function afterMove(at: number, from: number, to: number): number {
        if (at === from) return to;
        if (from < at && at <= to) return at - 1;
        if (to <= at && at < from) return at + 1;
        return at;
    }

    function move(from: number, to: number): void {
        if (openAt !== null) openAt = afterMove(openAt, from, to);
        onChange(moveRule(items, from, to));
    }

    function remove(at: number): void {
        const index = slots[at];
        if (index === undefined) return;
        openAt = null;
        onChange(items.filter((_, position) => position !== index));
    }

    /**
     * Append a rule and open it.
     *
     * Computed from the array being sent rather than read back off `entries`,
     * which has not been rebuilt from the new props yet — and a new rule is
     * blank, so its summary line says nothing worth collapsing.
     */
    function add(): void {
        const next = appendRule(items, blankRule(fallbackAccount));
        openAt = ruleIndices(next).length - 1;
        onChange(next);
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

    /**
     * Open the cursored rule, or close it again.
     *
     * Enter used to focus the first control of an always-expanded card, which
     * was the honest answer while there was no collapsed state to open. Now
     * there is one, so Enter does what it does on every other list in this app
     * (`BalanceSheetView`, `IncomeStatementView`): it opens the thing under the
     * cursor. Focus still lands inside, after a tick, because the controls do
     * not exist until the card renders — so the keystroke that opens a rule is
     * still the keystroke that starts editing it.
     *
     * A kept item has nothing to open and is left alone rather than being given
     * a card that would only say so.
     */
    function toggleOpen(): void {
        const at = cursor.index;
        if (at < 0 || entries[at]?.kind !== "ifBlock") return;
        if (openAt === at) {
            openAt = null;
            return;
        }
        openAt = at;
        // The first INPUT, not the first control: in document order the card
        // opens with its ↑/↓ buttons, and landing on "move this rule up" is not
        // what "edit this rule" should do.
        void tick().then(() => {
            document.querySelector<HTMLElement>(`[data-rule="${at}"] input`)?.focus();
        });
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
            // Escape backs out one step at a time — close the open rule first,
            // clear the cursor once there is nothing open. Same shape as
            // `TransactionTable`, whose Escape disarms a delete before it
            // clears.
            {
                keys: "Escape",
                label: "Close the open rule, or clear the cursor",
                group: "Imports",
                run: () => (openAt === null ? cursor.clear() : (openAt = null)),
            },
            {keys: "Enter", label: "Open or close this rule", group: "Imports", run: toggleOpen},
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
                        {#if openAt === at}
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
                                onClose={() => (openAt = null)}
                            />
                        {:else}
                            <RuleSummaryCard
                                summary={describeIfBlock(item)}
                                position={at + 1}
                                total={entries.length}
                                {disabled}
                                onMoveUp={() => move(at, at - 1)}
                                onMoveDown={() => move(at, at + 1)}
                                onOpen={() => (openAt = at)}
                            />
                        {/if}
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
