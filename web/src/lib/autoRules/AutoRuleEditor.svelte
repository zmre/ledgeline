<!-- One `= QUERY` auto-posting rule, edited as a form (plan 30 Phase 4).

     "When a posting to MATCH ACCOUNT is projected, add these postings below
     it": each a percentage of the matched amount or a fixed amount, plus a
     counter-account that takes the balance. The engine turns the form into
     the `=` block (`auto_form::render`): an anchored, forecast-only query, the
     counter as the opposite multiplier, and every sign inferred from the
     accounts' types — so the user types POSITIVE figures meaning "this account
     grows by". The one-line preview under the form says what that comes to.

     Reusable on purpose: it edits a bare `AutoRuleForm` and knows nothing of
     any store. The Projections table hosts one per scenario rule; Settings ›
     Automatic postings will host one per journal rule. The host owns
     persistence and is told of every committed edit through `onChange`.

     Edits mutate `form` in place (it is the host's `$state`), and `onChange`
     fires on COMMIT — a change event, an account chosen — never per keystroke,
     the same rhythm as the projections table's own cells. -->
<script lang="ts">
    import {decToInput, parseAmountInput} from "#lib/api/editMapping.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import AccountInput from "#lib/journal/edit/AccountInput.svelte";
    import {blankPosting, rulePreview, type AutoRuleForm, type AutoRuleFormPosting, type RuleAmount} from "./ruleForm";

    let {
        form,
        accountNames,
        name,
        commodity,
        sample = null,
        styles,
        onChange,
    }: {
        /** The rule being edited. Mutated in place. */
        form: AutoRuleForm;
        accountNames: string[];
        /** The rule's accessible name ("rule 2"), unique on the page: every field's label is built from it. */
        name: string;
        /** The commodity a new fixed amount opens in. */
        commodity: string;
        /** A typical amount on the matched account, for the preview; null when the host knows none. */
        sample?: RuleAmount | null;
        styles: ReadonlyMap<string, AmountStyle>;
        /** Called after every committed edit. */
        onChange: () => void;
    } = $props();

    /** Open while there are extra terms, or once the user asks for them. */
    let advanced = $state(false);
    const showAdvanced = $derived(advanced || form.extraQuery.trim() !== "");

    /** The text of a number box the user left unreadable, per posting, so the box keeps it and says so. */
    let invalid = $state<Record<number, string>>({});
    /** A commodity typed before its posting's number, per posting. */
    let pendingCommodity = $state<Record<number, string>>({});

    /** The commodity a fixed posting is in, or will be once it has a number. */
    const commodityOf = (posting: AutoRuleFormPosting, at: number): string => posting.amount?.commodity ?? pendingCommodity[at] ?? commodity;

    const preview = $derived(rulePreview(form, sample, styles));

    function setKind(posting: AutoRuleFormPosting, at: number, kind: "percent" | "fixed"): void {
        if (posting.kind === kind) return;
        posting.kind = kind;
        // The other mode's number means something else; start this one empty.
        posting.percent = null;
        posting.amount = null;
        delete invalid[at];
        onChange();
    }

    function setNumber(posting: AutoRuleFormPosting, at: number, text: string): void {
        const parsed = parseAmountInput(text);
        if (parsed === null && text.trim() !== "") {
            invalid[at] = text;
            return;
        }
        delete invalid[at];
        if (posting.kind === "percent") {
            posting.percent = parsed;
        } else {
            posting.amount = parsed === null ? null : {commodity: commodityOf(posting, at), quantity: parsed, precision: parsed.p};
        }
        onChange();
    }

    function setCommodity(posting: AutoRuleFormPosting, at: number, text: string): void {
        if (posting.amount === null) {
            pendingCommodity[at] = text.trim();
            return;
        }
        posting.amount.commodity = text.trim();
        onChange();
    }

    function addPosting(): void {
        form.postings.push(blankPosting());
        onChange();
    }

    function dropPosting(at: number): void {
        form.postings.splice(at, 1);
        invalid = {};
        pendingCommodity = {};
        onChange();
    }

    function numberText(posting: AutoRuleFormPosting, at: number): string {
        if (invalid[at] !== undefined) return invalid[at];
        if (posting.kind === "percent") return posting.percent === null ? "" : decToInput(posting.percent);
        return posting.amount === null ? "" : decToInput(posting.amount.quantity);
    }
</script>

<div class="flex flex-col gap-2 text-xs" data-testid="auto-rule-editor">
    <div class="flex flex-wrap items-center gap-2">
        <span class="text-base-content/70">When a posting to</span>
        <div class="w-64">
            <AccountInput
                bind:value={form.matchAccount}
                {accountNames}
                size="xs"
                label="Matched account for {name}"
                placeholder="account"
                onCommit={onChange}
            />
        </div>
        <span class="text-base-content/70">or one of its subaccounts is projected</span>
        <span
            class="badge badge-ghost badge-xs"
            title="The rule is written with tag:_generated-transaction, so `hledger --auto --forecast` applies it to forecast transactions only, as Ledgeline does."
            >forecasts only</span
        >
        {#if !showAdvanced}
            <button type="button" class="btn btn-ghost btn-xs" onclick={() => (advanced = true)}>+ query terms</button>
        {/if}
    </div>
    {#if showAdvanced}
        <label class="flex items-center gap-2">
            <span class="text-base-content/70" title="hledger query terms, such as desc:acme or amt:>1000, combined with the account as hledger combines them"
                >and also</span
            >
            <input
                type="text"
                class="input w-full max-w-md font-mono input-xs"
                placeholder="desc:acme amt:>1000"
                bind:value={form.extraQuery}
                aria-label="Extra query terms for {name}"
                onchange={onChange}
            />
        </label>
    {/if}

    <table class="table w-auto table-xs">
        <thead>
            <tr>
                <th class="w-64">Add a posting to</th>
                <th class="w-36">Of</th>
                <th class="w-40">Amount</th>
                <th class="w-8"><span class="sr-only">Remove</span></th>
            </tr>
        </thead>
        <tbody>
            {#each form.postings as posting, at (at)}
                <tr data-testid="auto-rule-posting">
                    <td>
                        <AccountInput
                            bind:value={posting.account}
                            {accountNames}
                            size="xs"
                            label="Account for posting {at + 1} of {name}"
                            placeholder="account"
                            onCommit={onChange}
                        />
                    </td>
                    <td>
                        <select
                            class="select w-full select-xs"
                            value={posting.kind}
                            aria-label="Amount kind for posting {at + 1} of {name}"
                            onchange={(e) => setKind(posting, at, e.currentTarget.value === "fixed" ? "fixed" : "percent")}
                        >
                            <option value="percent">% of matched</option>
                            <option value="fixed">fixed amount</option>
                        </select>
                    </td>
                    <td>
                        <label class="input flex items-center gap-1 input-xs" class:input-error={invalid[at] !== undefined}>
                            {#if posting.kind === "fixed"}
                                <input
                                    type="text"
                                    class="w-10 text-base-content/60"
                                    value={commodityOf(posting, at)}
                                    aria-label="Commodity for posting {at + 1} of {name}"
                                    onchange={(e) => setCommodity(posting, at, e.currentTarget.value)}
                                />
                            {/if}
                            <input
                                type="text"
                                inputmode="decimal"
                                class="w-full grow"
                                placeholder={posting.kind === "percent" ? "3.5" : "0.00"}
                                value={numberText(posting, at)}
                                aria-label="{posting.kind === 'percent' ? 'Percent' : 'Amount'} for posting {at + 1} of {name}"
                                aria-invalid={invalid[at] !== undefined}
                                onchange={(e) => setNumber(posting, at, e.currentTarget.value)}
                            />
                            {#if posting.kind === "percent"}
                                <span class="text-base-content/50">%</span>
                            {/if}
                        </label>
                    </td>
                    <td>
                        {#if form.postings.length > 1}
                            <button
                                type="button"
                                class="btn btn-ghost text-error btn-xs"
                                aria-label="Remove posting {at + 1} of {name}"
                                onclick={() => dropPosting(at)}>✕</button
                            >
                        {/if}
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
    <div class="flex flex-wrap items-center gap-2">
        <button type="button" class="btn btn-ghost btn-xs" onclick={addPosting}>+ Add a posting</button>
        <span
            class="text-base-content/70"
            title="Takes the opposite of the postings above, so each firing balances. Leave it empty only when the postings balance by themselves."
            >balanced against</span
        >
        <div class="w-64">
            <AccountInput
                bind:value={() => form.counter ?? "", (next) => (form.counter = next.trim() === "" ? null : next)}
                {accountNames}
                size="xs"
                label="Counter-account for {name}"
                placeholder="none"
                onCommit={onChange}
            />
        </div>
        <input type="text" class="input w-56 input-xs" placeholder="comment" bind:value={form.comment} aria-label="Comment for {name}" onchange={onChange} />
    </div>
    {#if preview !== null}
        <p class="text-base-content/60" data-testid="auto-rule-preview">{preview}</p>
    {/if}
</div>
