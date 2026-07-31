<script lang="ts">
    // One editable rule: an OR list of things to match, and the fields to set
    // when any of them do.
    //
    // Only the OR shape is editable, and that is a decision rather than a gap.
    // A block whose matchers are joined with `&`/`!` is a positional chain whose
    // meaning depends on the order of its own lines, and one with a capture
    // group has assignments that refer back into a pattern — rewriting half of
    // either can change what the other half means, so the engine classifies them
    // `opaque` and this GUI shows them read-only.
    //
    // Fields bind straight into the rule object. `form.items` is `$state`, so
    // the proxy makes a nested write reactive without the parent having to
    // rebuild an array on every keystroke; add/remove replace the whole
    // `matchers`/`assignments` array, exactly as `TransactionModal` does for
    // posting rows.
    import AccountInput from "$lib/journal/edit/AccountInput.svelte";
    import {ASSIGNABLE_FIELDS, type IfBlockItem} from "../model";

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
    } = $props();

    let confirmingRemove = $state(false);

    function addMatcher(): void {
        rule.matchers = [...rule.matchers, {field: "", pattern: ""}];
    }
    function removeMatcher(index: number): void {
        rule.matchers = rule.matchers.filter((_, at) => at !== index);
    }
    function addAssignment(): void {
        rule.assignments = [...rule.assignments, {field: "account2", value: ""}];
    }
    function removeAssignment(index: number): void {
        rule.assignments = rule.assignments.filter((_, at) => at !== index);
    }

    /** The scope options: whole row, the CSV's own columns, plus any field this rule already names. */
    const scopes = $derived([
        ...new Set([...csvFields.filter((name) => name !== ""), ...rule.matchers.map((matcher) => matcher.field).filter((f) => f !== "")]),
    ]);

    /** An `accountN` assignment gets the account autocomplete; everything else is a plain value. */
    function isAccountField(field: string): boolean {
        return /^account\d{1,2}$/.test(field);
    }

    const summary = $derived(rule.matchers.map((matcher) => matcher.pattern).filter((pattern) => pattern !== "")[0] ?? "new rule");
</script>

<div class="card bg-base-200 border-base-content/10 border" data-testid="imports-rule">
    <div class="card-body gap-3 p-3">
        <div class="flex items-center gap-2">
            <!-- Disabled at the bounds rather than hidden: a control that
                 vanishes makes the row jump under the pointer, and a disabled
                 button is still in the accessibility tree, so `getByRole` finds
                 it and a screen reader announces why it cannot be used. -->
            <div class="join">
                <button
                    type="button"
                    class="btn btn-xs join-item"
                    disabled={disabled || position === 1}
                    onclick={onMoveUp}
                    aria-label="Move rule {position} up"
                    title="Move up"
                >
                    ↑
                </button>
                <button
                    type="button"
                    class="btn btn-xs join-item"
                    disabled={disabled || position === total}
                    onclick={onMoveDown}
                    aria-label="Move rule {position} down"
                    title="Move down"
                >
                    ↓
                </button>
            </div>
            <span class="text-base-content/60 text-xs">Rule {position}</span>
            <span class="grow truncate text-sm font-medium">{summary}</span>
            {#if confirmingRemove}
                <span class="text-xs">Delete this rule?</span>
                <button type="button" class="btn btn-error btn-xs" {disabled} onclick={onRemove}>Delete</button>
                <button type="button" class="btn btn-ghost btn-xs" {disabled} onclick={() => (confirmingRemove = false)}>Keep</button>
            {:else}
                <button type="button" class="btn btn-ghost btn-xs" {disabled} onclick={() => (confirmingRemove = true)} aria-label="Delete rule {position}">
                    ✕
                </button>
            {/if}
        </div>

        <div class="flex flex-col gap-2">
            <span class="text-base-content/60 text-xs">When <strong>any</strong> of these match:</span>
            {#each rule.matchers as matcher, index (index)}
                <div class="flex items-center gap-2">
                    <select class="select select-xs w-40 shrink-0" {disabled} aria-label="Rule {position}, match {index + 1} column" bind:value={matcher.field}>
                        <option value="">the whole row</option>
                        {#each scopes as scope (scope)}
                            <option value={scope}>{scope}</option>
                        {/each}
                    </select>
                    <input
                        type="text"
                        class="input input-xs min-w-0 grow font-mono"
                        {disabled}
                        placeholder="text or regex"
                        aria-label="Rule {position}, match {index + 1} text"
                        bind:value={matcher.pattern}
                    />
                    <button
                        type="button"
                        class="btn btn-ghost btn-xs btn-square shrink-0"
                        {disabled}
                        onclick={() => removeMatcher(index)}
                        aria-label="Remove match {index + 1} from rule {position}"
                    >
                        ✕
                    </button>
                </div>
            {/each}
            <button type="button" class="btn btn-ghost btn-xs self-start gap-1" {disabled} onclick={addMatcher}>+ Add match</button>
        </div>

        <div class="flex flex-col gap-2">
            <span class="text-base-content/60 text-xs">Then set:</span>
            {#each rule.assignments as assignment, index (index)}
                <div class="flex items-center gap-2">
                    <select class="select select-xs w-40 shrink-0" {disabled} aria-label="Rule {position}, set {index + 1} field" bind:value={assignment.field}>
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
                                class="input input-xs w-full"
                                {disabled}
                                aria-label="Rule {position}, set {index + 1} value"
                                bind:value={assignment.value}
                            />
                        {/if}
                    </div>
                    <button
                        type="button"
                        class="btn btn-ghost btn-xs btn-square shrink-0"
                        {disabled}
                        onclick={() => removeAssignment(index)}
                        aria-label="Remove set {index + 1} from rule {position}"
                    >
                        ✕
                    </button>
                </div>
            {/each}
            <button type="button" class="btn btn-ghost btn-xs self-start gap-1" {disabled} onclick={addAssignment}>+ Add field to set</button>
        </div>
    </div>
</div>
