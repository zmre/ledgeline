<!-- Add or edit one budget goal.

     Four fields, in the order the decision is actually made: how often, on what,
     against what history, how much. The history strip sits between the account
     and the amount deliberately — it is the answer to "how much" and it should
     be on screen before the box that asks.

     Signs never appear here. The user types a magnitude for an expense and a
     magnitude for income alike; the engine negates an income goal on the way in
     and un-negates it on the way out (`budget_api.rs`, "Signs"). The preview
     line shows what will actually be written, so that flip is visible rather
     than surprising. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {parseAmountInput} from "$lib/api/editMapping";
    import {BUDGET_PERIODS, type AccountReference, type BudgetPeriod, type GoalDraft, type GoalSubmission} from "$lib/budget/types";
    import type {AmountStyle} from "$lib/domain/types";
    import AccountInput from "$lib/journal/edit/AccountInput.svelte";
    import type {DataView} from "$lib/stores/loadState";
    import ReferenceStrip from "./ReferenceStrip.svelte";

    let {
        draft,
        accountNames,
        reference,
        referenceView,
        styles,
        saving,
        error,
        isBudgeted,
        onAccountChange,
        onSubmit,
        onCancel,
    }: {
        draft: GoalDraft;
        accountNames: string[];
        reference: AccountReference | null;
        referenceView: DataView;
        styles: ReadonlyMap<string, AmountStyle>;
        saving: boolean;
        /** The engine's own sentence for a refused save, or null. */
        error: string | null;
        /**
         * Whether the rule this goal would join already budgets this account, in
         * which case the engine will refuse the save. Asked as a function rather
         * than passed as a list because the answer depends on the period, and the
         * period is a field of THIS form — the parent cannot know it in advance.
         */
        isBudgeted: (account: string, period: BudgetPeriod) => boolean;
        /** Called whenever the account or period settles, so the strip can refetch. */
        onAccountChange: (account: string, period: BudgetPeriod) => void;
        onSubmit: (submission: GoalSubmission) => void;
        onCancel: () => void;
    } = $props();

    const editing = $derived(draft.goal !== null);
    // The period of an existing goal is a property of the RULE it lives in, and
    // moving a goal between rules is a different (and much larger) edit than
    // changing its number. So it is shown and not offered.
    const periodFixed = $derived(editing);

    // Seeded ONCE from the draft, and then owned by the fields — typing into the
    // amount box must not be undone by the draft the parent is still holding.
    // `untrack` says that rather than suppressing the `state_referenced_locally`
    // warning it would otherwise raise; this repo carries no `svelte-ignore`
    // comments. The parent `{#key}`s this component on the draft's identity, so
    // "once" means once per goal being edited, not once per session.
    let period = $state<BudgetPeriod>(untrack(() => draft.period));
    let account = $state(untrack(() => draft.account));
    let amount = $state(untrack(() => draft.amount));

    const parsed = $derived(parseAmountInput(amount));
    const accountValid = $derived(account.trim() !== "");
    // A goal this rule already states. Not an error in the form — nothing typed
    // here is wrong — but a save the engine would refuse, so it is said here and
    // the button is held rather than letting the user find out afterwards.
    const duplicate = $derived(!editing && accountValid && isBudgeted(account.trim(), period));
    const canSubmit = $derived(accountValid && parsed !== null && !saving && !duplicate);

    // The strip follows the two fields it is a function of. Debounce-free: the
    // account field commits on blur/selection rather than on every keystroke, so
    // this fires once per settled choice.
    let lastAsked = "";
    $effect(() => {
        const key = `${period}|${account.trim()}`;
        if (!accountValid || key === lastAsked) return;
        lastAsked = key;
        onAccountChange(account.trim(), period);
    });

    const periodLabel = $derived(BUDGET_PERIODS.find((p) => p.id === period)?.label ?? period);

    /**
     * The line that will actually be written, shown verbatim.
     *
     * Two things it makes visible that would otherwise be a surprise: the sign
     * flip on an income goal, and the `(account)` form the goal is written in.
     * The commodity is the engine's to choose for a new goal, so it is only
     * shown when the goal already has one.
     */
    const preview = $derived.by(() => {
        if (parsed === null || !accountValid) return null;
        const inverted = reference?.inverted === true && reference.account === account.trim();
        const sign = inverted ? "-" : "";
        const commodity = draft.goal?.entry?.commodity ?? "";
        return `(${account.trim()})  ${commodity}${sign}${amount.trim()}`;
    });

    function submit(event: SubmitEvent): void {
        event.preventDefault();
        if (parsed === null || !accountValid) return;
        onSubmit({period, account: account.trim(), value: parsed});
    }
</script>

<!-- daisyUI modal. `onCancel` on Escape mirrors every other modal in the app. -->
<div class="modal modal-open" role="dialog" aria-modal="true" aria-label={editing ? "Edit budget goal" : "Add budget goal"} data-testid="goal-modal">
    <div class="modal-box max-w-lg">
        <h3 class="mb-4 text-lg font-semibold">{editing ? "Edit budget goal" : "Add a budget goal"}</h3>

        <form class="flex flex-col gap-4" onsubmit={submit}>
            <label class="form-control">
                <span class="label-text mb-1 block text-xs text-base-content/70">How often</span>
                {#if periodFixed}
                    <!-- Shown, not offered: see `periodFixed`. -->
                    <div class="flex items-center gap-2">
                        <span class="badge badge-neutral">{periodLabel}</span>
                        <span class="text-xs text-base-content/50">
                            set by the rule this goal is in{draft.rule !== null && draft.rule.description !== "" ? ` — “${draft.rule.description}”` : ""}
                        </span>
                    </div>
                {:else}
                    <select class="select w-full select-sm" bind:value={period} aria-label="How often">
                        {#each BUDGET_PERIODS as option (option.id)}
                            <option value={option.id}>{option.label}</option>
                        {/each}
                    </select>
                {/if}
            </label>

            <label class="form-control">
                <span class="label-text mb-1 block text-xs text-base-content/70">Category</span>
                {#if editing}
                    <!-- Renaming a goal's account is a move, not an edit: it
                         would mean deleting one line and writing another, under
                         a control that looks like a text field. -->
                    <input type="text" class="input w-full input-sm" value={account} disabled aria-label="Category" />
                {:else}
                    <!-- The suggestion list opens on TYPING, not on focus. This
                         field is autofocused, and a list opened during mount is
                         positioned against a modal that is still animating into
                         place — see AccountInput's `onInput`. -->
                    <AccountInput bind:value={account} {accountNames} placeholder="expenses:food" autofocus />
                    {#if duplicate}
                        <span class="mt-1 text-xs text-warning" data-testid="goal-duplicate">
                            This category already has a {periodLabel.toLowerCase()} goal. Close this and edit that goal instead — a rule states an account's goal
                            once.
                        </span>
                    {/if}
                {/if}
            </label>

            <ReferenceStrip view={referenceView} {reference} {styles} />

            <label class="form-control">
                <span class="label-text mb-1 block text-xs text-base-content/70">
                    Amount per {BUDGET_PERIODS.find((p) => p.id === period)?.plural.replace(/s$/, "") ?? "period"}
                </span>
                <input
                    type="text"
                    inputmode="decimal"
                    class="input w-40 font-mono tabular-nums input-sm"
                    bind:value={amount}
                    placeholder="400"
                    aria-label="Amount"
                    aria-invalid={amount.trim() !== "" && parsed === null}
                />
                {#if amount.trim() !== "" && parsed === null}
                    <span class="mt-1 text-xs text-error">That isn't a number.</span>
                {:else if reference?.inverted === true}
                    <!-- Said out loud, because it is the one place the app writes
                         a different number than the one that was typed. -->
                    <span class="mt-1 text-xs text-base-content/60">This is an income account, so hledger records it as a negative amount.</span>
                {/if}
            </label>

            {#if preview !== null}
                <div class="rounded-box bg-base-200 px-3 py-2">
                    <span class="mb-1 block text-xs text-base-content/60">Will be written as</span>
                    <code class="font-mono text-sm" data-testid="goal-preview">{preview}</code>
                </div>
            {/if}

            {#if error !== null}
                <div class="alert py-2 text-sm alert-error" role="alert" data-testid="goal-error">
                    <span class="break-words">{error}</span>
                </div>
            {/if}

            <div class="modal-action mt-0">
                <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={saving}>Cancel</button>
                <button type="submit" class="btn btn-primary btn-sm" disabled={!canSubmit}>
                    {#if saving}<span class="loading loading-xs loading-spinner"></span>{/if}
                    {editing ? "Save" : "Add goal"}
                </button>
            </div>
        </form>
    </div>
    <!-- The backdrop dismisses, like every other modal here. -->
    <button type="button" class="modal-backdrop" onclick={onCancel} aria-label="Close" disabled={saving}></button>
</div>
