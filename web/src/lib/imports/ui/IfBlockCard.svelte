<script lang="ts">
    // One rule, opened for editing: an OR of AND-groups, and the fields to set
    // when any of those groups match.
    //
    // This is the EDIT view. The rules list shows each rule as a one-line
    // summary and opens exactly one of these at a time, because a file with
    // thirty rules is not readable as thirty stacked editors.
    //
    // # What is editable, and what is still not
    //
    // Matchers within a group are AND-ed and groups are OR-ed, which is exactly
    // hledger's line-prefix `&`: a plain matcher line opens a new OR branch and
    // each `&` line below it is AND-ed onto that branch. The AND is carried as
    // NESTING and never as text — the engine writes the `&` — so nothing typed
    // into a pattern here can become a combinator.
    //
    // Still refused, and still a decision rather than a gap: a `!` negation, an
    // `&&` join (on one line those bytes may be two literal ampersands in a
    // regex, and telling them apart needs hledger's own parser), a leading `&`
    // with no group above it, a capture group whose value a later assignment
    // refers back into, and an `if` TABLE, whose rows read positionally against
    // its header. The engine classifies each of those `opaque` and this GUI
    // shows them read-only.
    //
    // Fields bind straight into the rule object. `form.items` is `$state`, so
    // the proxy makes a nested write reactive without the parent having to
    // rebuild an array on every keystroke; add/remove replace the whole
    // `groups`/`assignments` array, exactly as `TransactionModal` does for
    // posting rows.
    import AccountInput from "$lib/journal/edit/AccountInput.svelte";
    import {ASSIGNABLE_FIELDS, describeIfBlock, type IfBlockItem} from "../model";

    let {
        rule,
        position,
        total,
        accountNames,
        csvFields,
        disabled,
        onMoveUp,
        onMoveDown,
        onRemove,
        onClose,
    }: {
        rule: IfBlockItem;
        /** 1-based position in the rules list, which is what the user is reordering. */
        position: number;
        total: number;
        accountNames: string[];
        /** The CSV's own column names, so a matcher can be scoped to one by name. */
        csvFields: string[];
        disabled: boolean;
        onMoveUp: () => void;
        onMoveDown: () => void;
        onRemove: () => void;
        /** Collapse back to the summary line. The rule keeps every edit — nothing is committed or discarded here. */
        onClose: () => void;
    } = $props();

    let confirmingRemove = $state(false);

    /**
     * Escape, from anywhere inside the card.
     *
     * The global keymap deliberately hears nothing while focus is in a field —
     * "Escape inside a field belongs to that field" — and opening a rule puts
     * focus straight into its first matcher, so without this the one key that
     * should back out of an editor would do nothing at all in the state it is
     * most often pressed in.
     *
     * Claimed only where it is consumed, and only if nothing nearer has claimed
     * it already: `AccountInput` calls `preventDefault` for its own popup, and
     * stealing that Escape would close the whole card instead of the suggestion
     * list under the cursor.
     */
    function onKeydown(event: KeyboardEvent): void {
        if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return;
        event.preventDefault();
        if (confirmingRemove) confirmingRemove = false;
        else onClose();
    }

    /** Add an AND condition to one group: this group matches only when every row in it does. */
    function addMatcher(groupAt: number): void {
        rule.groups = rule.groups.map((group, at) => (at === groupAt ? {matchers: [...group.matchers, {field: "", pattern: ""}]} : group));
    }

    /**
     * Remove one condition, and the group with it once it holds nothing.
     *
     * An empty group is refused by the engine rather than dropped, because a
     * group that vanished when the groups were flattened back into lines would
     * silently re-group its neighbours into one branch.
     */
    function removeMatcher(groupAt: number, index: number): void {
        rule.groups = rule.groups
            .map((group, at) => (at === groupAt ? {matchers: group.matchers.filter((_, m) => m !== index)} : group))
            .filter((group) => group.matchers.length > 0);
    }

    /** Add a new OR branch: the rule matches when this one does, whatever the others say. */
    function addGroup(): void {
        rule.groups = [...rule.groups, {matchers: [{field: "", pattern: ""}]}];
    }

    function addAssignment(): void {
        rule.assignments = [...rule.assignments, {field: "account2", value: ""}];
    }
    function removeAssignment(index: number): void {
        rule.assignments = rule.assignments.filter((_, at) => at !== index);
    }

    /** The scope options: whole row, the CSV's own columns, plus any field this rule already names. */
    const scopes = $derived([
        ...new Set([
            ...csvFields.filter((name) => name !== ""),
            ...rule.groups.flatMap((group) => group.matchers.map((matcher) => matcher.field)).filter((field) => field !== ""),
        ]),
    ]);

    /** An `accountN` assignment gets the account autocomplete; everything else is a plain value. */
    function isAccountField(field: string): boolean {
        return /^account\d{1,2}$/.test(field);
    }

    // The same line the collapsed card shows, so opening a rule does not change
    // what it is called — and it follows the edit as it is typed.
    const summary = $derived(describeIfBlock(rule));
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="card min-w-0 border border-primary/40 bg-base-200" data-testid="imports-rule" onkeydown={onKeydown}>
    <div class="card-body gap-3 p-3">
        <div class="flex items-center gap-2">
            <!-- Disabled at the bounds rather than hidden: a control that
                 vanishes makes the row jump under the pointer, and a disabled
                 button is still in the accessibility tree, so `getByRole` finds
                 it and a screen reader announces why it cannot be used. -->
            <div class="join">
                <button
                    type="button"
                    class="btn join-item btn-xs"
                    disabled={disabled || position === 1}
                    onclick={onMoveUp}
                    aria-label="Move rule {position} up"
                    title="Move up"
                >
                    ↑
                </button>
                <button
                    type="button"
                    class="btn join-item btn-xs"
                    disabled={disabled || position === total}
                    onclick={onMoveDown}
                    aria-label="Move rule {position} down"
                    title="Move down"
                >
                    ↓
                </button>
            </div>
            <span class="shrink-0 text-xs text-base-content/60">Rule {position}</span>
            <span class="grow truncate text-sm font-medium" title={summary}>{summary}</span>
            {#if confirmingRemove}
                <span class="text-xs">Delete this rule?</span>
                <button type="button" class="btn btn-error btn-xs" {disabled} onclick={onRemove}>Delete</button>
                <button type="button" class="btn btn-ghost btn-xs" {disabled} onclick={() => (confirmingRemove = false)}>Keep</button>
            {:else}
                <button type="button" class="btn btn-ghost btn-xs" {disabled} onclick={() => (confirmingRemove = true)} aria-label="Delete rule {position}">
                    ✕
                </button>
                <!-- Closing is not saving and not cancelling: the edit stays in
                     the form either way, and the one Save button above the list
                     is still the only thing that writes. -->
                <button type="button" class="btn btn-xs" onclick={onClose} aria-label="Close rule {position}" aria-expanded="true">Done</button>
            {/if}
        </div>

        <div class="flex flex-col gap-2">
            <span class="text-xs text-base-content/60">When <strong>any</strong> of these match:</span>
            {#each rule.groups as group, groupAt (groupAt)}
                {#if groupAt > 0}
                    <div class="flex items-center gap-2 text-xs font-semibold tracking-wide text-base-content/40 uppercase">
                        <span class="h-px grow bg-base-content/10"></span>
                        or
                        <span class="h-px grow bg-base-content/10"></span>
                    </div>
                {/if}
                <!-- The rail is the AND: everything inside it has to match at
                     once, and without it two conditions look exactly like two
                     alternatives. -->
                <div class="flex flex-col gap-2 border-l-2 border-base-content/20 pl-2">
                    {#each group.matchers as matcher, index (index)}
                        {#if index > 0}
                            <span class="text-xs font-semibold tracking-wide text-base-content/40 uppercase">and</span>
                        {/if}
                        <div class="flex items-center gap-2">
                            <select
                                class="select w-32 shrink-0 select-xs sm:w-40"
                                {disabled}
                                aria-label="Rule {position}, group {groupAt + 1}, match {index + 1} column"
                                bind:value={matcher.field}
                            >
                                <option value="">the whole row</option>
                                {#each scopes as scope (scope)}
                                    <option value={scope}>{scope}</option>
                                {/each}
                            </select>
                            <input
                                type="text"
                                class="input min-w-0 grow font-mono input-xs"
                                {disabled}
                                placeholder="text or regex"
                                aria-label="Rule {position}, group {groupAt + 1}, match {index + 1} text"
                                bind:value={matcher.pattern}
                            />
                            <button
                                type="button"
                                class="btn btn-square shrink-0 btn-ghost btn-xs"
                                {disabled}
                                onclick={() => removeMatcher(groupAt, index)}
                                aria-label="Remove match {index + 1} from group {groupAt + 1} of rule {position}"
                            >
                                ✕
                            </button>
                        </div>
                    {/each}
                    <button
                        type="button"
                        class="btn gap-1 self-start btn-ghost btn-xs"
                        {disabled}
                        onclick={() => addMatcher(groupAt)}
                        aria-label="Add an AND condition to group {groupAt + 1} of rule {position}"
                        title="Both this and the conditions above it must match"
                    >
                        + AND condition
                    </button>
                </div>
            {/each}
            <button
                type="button"
                class="btn gap-1 self-start btn-ghost btn-xs"
                {disabled}
                onclick={addGroup}
                aria-label="Add an OR group to rule {position}"
                title="Another alternative — the rule matches if this one does"
            >
                + OR group
            </button>
        </div>

        <div class="flex flex-col gap-2">
            <span class="text-xs text-base-content/60">Then set:</span>
            {#each rule.assignments as assignment, index (index)}
                <div class="flex items-center gap-2">
                    <select
                        class="select w-32 shrink-0 select-xs sm:w-40"
                        {disabled}
                        aria-label="Rule {position}, set {index + 1} field"
                        bind:value={assignment.field}
                    >
                        {#each [...new Set([...ASSIGNABLE_FIELDS, assignment.field])] as field (field)}
                            <option value={field}>{field}</option>
                        {/each}
                    </select>
                    <div class="min-w-0 grow">
                        {#if isAccountField(assignment.field)}
                            <AccountInput bind:value={assignment.value} {accountNames} {disabled} size="xs" placeholder="expenses:food" />
                        {:else}
                            <input
                                type="text"
                                class="input w-full input-xs"
                                {disabled}
                                aria-label="Rule {position}, set {index + 1} value"
                                bind:value={assignment.value}
                            />
                        {/if}
                    </div>
                    <button
                        type="button"
                        class="btn btn-square shrink-0 btn-ghost btn-xs"
                        {disabled}
                        onclick={() => removeAssignment(index)}
                        aria-label="Remove set {index + 1} from rule {position}"
                    >
                        ✕
                    </button>
                </div>
            {/each}
            <button type="button" class="btn gap-1 self-start btn-ghost btn-xs" {disabled} onclick={addAssignment}>+ Add field to set</button>
        </div>
    </div>
</div>
