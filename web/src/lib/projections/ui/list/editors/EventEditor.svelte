<!-- The dialog's fields for a one-off — a `~ YYYY-MM-DD` block (plan 31 C3).

     The date, what it is, and its postings with SIGNED amounts: an event is
     where a balance-sheet movement is written (the raise that puts $2M into
     `assets:cash`), and its direction is the user's to state, not an account
     type's (plan 22 amendment 19). Then the counter-account and the header
     comment. Edits write the draft only. -->
<script lang="ts">
    import {decToInput, parseAmountInput} from "#lib/api/editMapping.js";
    import AccountInput from "#lib/journal/edit/AccountInput.svelte";
    import {accountOrNull, isIsoDate, noteProblem} from "../../../scenarioModel";
    import type {ForecastDefaults, ScenarioEvent} from "../../../types";

    let {
        event,
        accountNames,
        commodity,
        defaults,
        cash,
        onEdit,
    }: {
        /** The draft's event. Mutated in place. */
        event: ScenarioEvent;
        accountNames: string[];
        commodity: string;
        defaults: ForecastDefaults | null;
        cash: string | null;
        onEdit: () => void;
    } = $props();

    const noteError = $derived(noteProblem(event.description));
    const names = $derived(
        event.postings.map((posting, at) => (posting.account.trim() === "" ? `posting ${at + 1}` : `${posting.account.trim()} (posting ${at + 1})`))
    );

    function setDate(value: string): void {
        if (!isIsoDate(value)) return;
        event.date = value;
        onEdit();
    }

    function setAmount(at: number, raw: string): void {
        const parsed = parseAmountInput(raw);
        const posting = event.postings[at];
        if (parsed === null || posting === undefined) return;
        posting.amount = {...posting.amount, quantity: parsed, precision: Math.max(posting.amount.precision, parsed.p)};
        onEdit();
    }

    function addPosting(): void {
        event.postings.push({account: "", amount: {commodity, quantity: {m: 0n, p: 2}, precision: 2}, comment: ""});
        onEdit();
    }

    function dropPosting(at: number): void {
        if (event.postings.length <= 1) return;
        event.postings.splice(at, 1);
        onEdit();
    }
</script>

<div class="flex flex-col gap-4" data-testid="event-editor">
    <div class="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,11rem)_minmax(0,1fr)]">
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Date</span>
            <input type="date" class="input w-full input-sm" value={event.date} aria-label="Date" onchange={(e) => setDate(e.currentTarget.value)} />
        </label>
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Description</span>
            <input
                type="text"
                class={["input w-full input-sm", noteError !== null && "input-error"]}
                value={event.description}
                placeholder="what it is"
                aria-label="Description"
                aria-invalid={noteError !== null}
                onchange={(e) => {
                    event.description = e.currentTarget.value;
                    onEdit();
                }}
            />
            {#if noteError !== null}<span class="text-xs text-error">{noteError}</span>{/if}
        </label>
    </div>

    <div class="flex flex-col gap-1.5">
        <div class="flex items-baseline justify-between">
            <span class="text-xs font-medium text-base-content/70">Postings</span>
            <span class="text-xs text-base-content/45">Signed: a minus is money leaving that account.</span>
        </div>
        <ol class="flex flex-col gap-2">
            {#each event.postings as posting, at (at)}
                {@const name = names[at]}
                <li class="rounded-box border border-base-content/8 bg-base-100/40 p-2" data-testid="event-posting">
                    <div class="flex flex-wrap items-center gap-2">
                        <div class="min-w-48 grow basis-56">
                            <AccountInput
                                bind:value={posting.account}
                                {accountNames}
                                size="sm"
                                label="Account for {name}"
                                placeholder="account"
                                onCommit={onEdit}
                            />
                        </div>
                        <label class="input w-40 input-sm">
                            <span class="text-base-content/50">{posting.amount.commodity}</span>
                            <input
                                type="text"
                                inputmode="decimal"
                                class="text-right font-mono"
                                value={decToInput(posting.amount.quantity)}
                                aria-label="Amount for {name}"
                                onchange={(e) => setAmount(at, e.currentTarget.value)}
                            />
                        </label>
                        <button
                            type="button"
                            class="btn btn-square btn-ghost text-error btn-xs"
                            disabled={event.postings.length <= 1}
                            aria-label="Remove {name}"
                            onclick={() => dropPosting(at)}>✕</button
                        >
                    </div>
                    <input
                        type="text"
                        class="input mt-1.5 w-full input-ghost font-mono text-base-content/70 input-xs"
                        value={posting.comment}
                        placeholder="; comment on this posting (optional)"
                        aria-label="Comment for {name}"
                        onchange={(e) => {
                            posting.comment = e.currentTarget.value;
                            onEdit();
                        }}
                    />
                </li>
            {/each}
        </ol>
        <button type="button" class="btn self-start btn-ghost btn-xs" onclick={addPosting}>+ Add a posting</button>
    </div>

    <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Counter-account <span class="text-base-content/40">(takes what the postings leave)</span></span>
            <AccountInput
                bind:value={() => event.counter ?? "", (next) => (event.counter = next === "" ? null : next)}
                {accountNames}
                size="sm"
                label="Counter-account"
                placeholder={defaults?.events.get(event.id) ?? cash ?? defaults?.cash ?? "the default cash account"}
                onCommit={() => {
                    event.counter = accountOrNull(event.counter);
                    onEdit();
                }}
            />
        </label>
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Header comment</span>
            <input
                type="text"
                class="input w-full font-mono input-sm"
                value={event.comment.split("\n").join(" ")}
                placeholder="; on the ~ line (optional)"
                aria-label="Header comment"
                onchange={(e) => {
                    event.comment = e.currentTarget.value;
                    onEdit();
                }}
            />
        </label>
    </div>
</div>
