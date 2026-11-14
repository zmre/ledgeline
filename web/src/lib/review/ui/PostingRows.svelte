<script lang="ts">
    // The card's postings (plan 32 D6, D8, D13, D33): one row per WRITTEN
    // posting — account (the existing AccountInput), then the amount with its
    // commodity as a small prefix field — with an unknown account outlined in
    // warning, the suggestion chips under the row they fill, and the postings an
    // `=` rule generated listed read-only beneath, each naming and linking to
    // its rule (the only place it can be changed).
    //
    // Per row (D33):
    // - a balance assertion is a chip (`== $500.00`) whose × removes it. Rows
    //   keep one slot for it whenever the transaction has any, so the amounts
    //   stay in one column;
    // - a posting comment is a muted `;` line under the row, edited in place.
    //   ⋯ reveals it on a row that has none. It is the posting's WHOLE comment,
    //   tags and continuation lines included, so it is a textarea too;
    // - a blank amount (an elided leg, D35, p7 amendment 18) shows what it
    //   balances to as its placeholder ("auto · -6.75", `balancingHint`) and
    //   stays blank, so it stays elided. Typing a number makes it explicit;
    // - a cost stays read-only, noted under the row;
    // - the commodity of an elided leg is disabled until it has an amount:
    //   a blank leg writes no amount, so the commodity would go nowhere.
    //
    // Tab on the chip row's account field fills the selected chip in and, pressed
    // again, cycles top → alt 1 → alt 2 → top. It is caught in the CAPTURE
    // phase on the row's wrapper, before AccountInput sees it — AccountInput
    // stays unchanged (D15) and finds nothing left to complete, so the Tab ends
    // there. Anything the user has typed themselves goes to AccountInput as usual.
    import {tick} from "svelte";
    import {emptyPosting, txnToForm, type PostingForm} from "#lib/api/editMapping.js";
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {Transaction} from "#lib/domain/types.js";
    import AmountCell from "#lib/journal/AmountCell.svelte";
    import AccountInput from "#lib/journal/edit/AccountInput.svelte";
    import {generatedPostings} from "#lib/journal/rowModel.js";
    import {formatKeys} from "#lib/keys/chord.js";
    import {balancingHint, isUncategorized, percent, recurringLabel, shortAccount, type Chips} from "../card";
    import type {Suggestion} from "../types";
    import {assertionLabel} from "#lib/api/editMapping.js";
    import CommodityInput from "./CommodityInput.svelte";
    import GrowingTextarea from "./GrowingTextarea.svelte";
    import ReviewIcon from "./ReviewIcon.svelte";

    let {
        postings = $bindable([]),
        txn,
        classes,
        accountNames,
        chips,
        selected,
        commodities,
        defaultCommodity,
        disabled = false,
        onTabChip,
        onPickChip,
        onRule,
    }: {
        postings?: PostingForm[];
        txn: Transaction;
        classes: AccountClasses;
        accountNames: string[];
        chips: Chips;
        selected: number | null;
        commodities: readonly string[];
        defaultCommodity: string;
        disabled?: boolean;
        /** Tab on the chip row's account field, as it would fill (true: handled). */
        onTabChip: (fieldValue: string) => boolean;
        onPickChip: (at: number) => void;
        onRule: (rule: number) => void;
    } = $props();

    const generated = $derived(generatedPostings(txn));
    /** Whether the rows keep a slot for an assertion chip: the transaction has one, so removing it doesn't shift the amounts. */
    const assertionSlot = $derived(txn.postings.some((posting) => posting.generated === undefined && posting.balanceAssertion !== undefined));
    /** Rows whose comment line stays up even when empty: ⋯ revealed it, or it has had focus (clearing it must not pull it away). */
    let revealed = $state<number[]>([]);
    const acceptKeys = formatKeys("mod+shift+Enter")
        .map((token) => token.text)
        .join("");

    function onTargetKeydown(event: KeyboardEvent, row: number): void {
        if (event.key !== "Tab" || event.shiftKey || event.isComposing || row !== chips.target || chips.chips.length === 0) return;
        if (onTabChip(postings[row]?.account ?? "")) event.preventDefault();
    }

    /** A CAPTURE-phase keydown listener (an action, so the wrapper stays a plain div to the a11y rules). */
    function captureKeys(node: HTMLElement, handler: (event: KeyboardEvent) => void) {
        let current = handler;
        const listener = (event: KeyboardEvent): void => current(event);
        node.addEventListener("keydown", listener, true);
        return {
            update(next: (event: KeyboardEvent) => void): void {
                current = next;
            },
            destroy(): void {
                node.removeEventListener("keydown", listener, true);
            },
        };
    }

    function why(chip: Suggestion): string {
        switch (chip.source) {
            case "exactKey":
                return `${chip.count}× for this payee · ${percent(chip.confidence)}`;
            case "payeePrefix":
                return `similar payees · ${percent(chip.confidence)}`;
            case "recurring":
                return `matches a recurring charge · ${percent(chip.confidence)}`;
            case "ruleFile":
                return "from the import rules";
        }
    }

    function addRow(): void {
        postings = [...postings, emptyPosting(defaultCommodity)];
    }
    function removeRow(at: number): void {
        postings = postings.filter((_, index) => index !== at);
        revealed = revealed.filter((row) => row !== at).map((row) => (row > at ? row - 1 : row));
    }

    /** ⋯: show the row's comment line and put the caret in it. */
    async function revealComment(at: number, button: HTMLElement): Promise<void> {
        if (!revealed.includes(at)) revealed = [...revealed, at];
        await tick();
        button.closest("li")?.querySelector<HTMLTextAreaElement>(`[data-review-field="posting-comment-${at}"] textarea`)?.focus();
    }

    /** The rows as the transaction has them, for `balancingHint`'s "is the engine's value still true?". */
    const originalRows = $derived(txnToForm(txn).postings);
</script>

<ol class="flex flex-col gap-1.5" aria-label="Postings">
    {#each postings as posting, at (at)}
        {@const unknown = isUncategorized(posting.account, classes)}
        {@const inert = posting.elided === true && posting.amount.trim() === ""}
        <li class="flex flex-col gap-1" data-testid="review-posting">
            <!-- One line from `md`. Below it (a phone) the account takes the first
                 line beside its number and buttons, and the amount (and any
                 assertion) wraps to a second, right-aligned (plan 42). -->
            <div class="flex flex-wrap items-center gap-2 md:flex-nowrap">
                <kbd class="kbd w-6 shrink-0 justify-center kbd-xs text-base-content/50" title="Press {at + 1} to edit this account">{at + 1}</kbd>
                <div
                    class={["relative min-w-0 grow basis-[calc(100%-8rem)] rounded-field md:basis-auto", unknown && "ring-2 ring-warning/70"]}
                    data-review-field="account-{at}"
                    data-unknown={unknown ? "" : undefined}
                    use:captureKeys={(event) => onTargetKeydown(event, at)}
                >
                    <AccountInput bind:value={posting.account} {accountNames} label="Account for posting {at + 1}" placeholder="account:sub" {disabled} />
                </div>
                {#if unknown}
                    <span class="tooltip tooltip-left shrink-0 text-warning" data-tip="Uncategorised: pick an account">
                        <ReviewIcon name="warning" />
                    </span>
                {/if}
                <div class="order-last ml-auto w-44 shrink-0 md:order-none md:ml-0 md:w-40" data-review-field="amount-{at}">
                    <div class="input w-full gap-1 font-mono tabular-nums input-sm">
                        <!-- An elided leg writes no amount, so it has no commodity to change (p7 amendment 22): the field waits for an amount. -->
                        <CommodityInput
                            bind:value={posting.commodity}
                            {commodities}
                            label="Commodity for posting {at + 1}"
                            placeholder={defaultCommodity}
                            disabled={disabled || inert}
                            title={inert ? "Type an amount to set its commodity" : undefined}
                        />
                        <input
                            type="text"
                            inputmode="decimal"
                            class="min-w-0 grow text-right placeholder:text-base-content/35"
                            bind:value={posting.amount}
                            placeholder={balancingHint(originalRows, postings, at)}
                            aria-label="Amount for posting {at + 1}"
                            autocomplete="off"
                            data-review-focus
                            {disabled}
                        />
                    </div>
                </div>
                {#if assertionSlot}
                    <div class="order-last flex w-32 shrink-0 items-center md:order-none">
                        {#if posting.balanceAssertion !== null}
                            {@const assertion = assertionLabel(posting.balanceAssertion)}
                            <span
                                class="flex max-w-full items-center gap-0.5 rounded-full border border-base-content/15 py-0.5 pr-0.5 pl-2 font-mono text-xs text-base-content/60"
                                title="Balance assertion: the account's balance after this posting"
                                data-testid="review-assertion"
                            >
                                <span class="truncate">{assertion}</span>
                                <button
                                    type="button"
                                    class="btn btn-circle h-4 min-h-0 w-4 btn-ghost btn-xs"
                                    aria-label="Remove the balance assertion on posting {at + 1}"
                                    title="Remove the assertion"
                                    onclick={() => (posting.balanceAssertion = null)}
                                    {disabled}
                                >
                                    <ReviewIcon name="x" class="h-3 w-3" />
                                </button>
                            </span>
                        {/if}
                    </div>
                {/if}
                <button
                    type="button"
                    class={["btn btn-square shrink-0 btn-ghost btn-xs", (posting.comment !== "" || revealed.includes(at)) && "invisible"]}
                    aria-label="Add a comment to posting {at + 1}"
                    title="Add a posting comment"
                    onclick={(event) => void revealComment(at, event.currentTarget)}
                    {disabled}
                >
                    <ReviewIcon name="ellipsis" class="h-3.5 w-3.5" />
                </button>
                <button
                    type="button"
                    class="btn btn-square shrink-0 btn-ghost btn-xs"
                    aria-label="Remove posting {at + 1}"
                    title="Remove posting"
                    onclick={() => removeRow(at)}
                    disabled={disabled || postings.length <= 1}
                >
                    <ReviewIcon name="x" class="h-3.5 w-3.5" />
                </button>
            </div>
            {#if at === chips.target && chips.chips.length > 0}
                <div class="mt-2 mb-1 ml-8 flex flex-wrap items-center gap-2" data-testid="review-chips" aria-label="Suggestions">
                    {#each chips.chips as chip, index (chip.account)}
                        {@const active = index === selected}
                        <button
                            type="button"
                            class={[
                                "group flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs transition-colors",
                                active
                                    ? "border-success/60 bg-success/15 text-base-content"
                                    : "border-base-content/15 text-base-content/70 hover:border-base-content/30",
                            ]}
                            aria-pressed={active}
                            data-testid="review-chip"
                            title={chip.strong ? "A strong match" : "A weaker match: Tab to it to use it"}
                            onclick={() => onPickChip(index)}
                            {disabled}
                        >
                            {#if active}
                                <kbd class="kbd kbd-xs">{acceptKeys}</kbd>
                            {:else}
                                <ReviewIcon name={chip.source === "recurring" ? "bolt" : "sparkle"} class="h-3 w-3 text-base-content/40" />
                            {/if}
                            {#if chips.mode === "usually"}<span class="text-base-content/50">usually</span>{/if}
                            {#if chip.description !== null}
                                <span class="font-medium">→ {chip.description}</span><span class="text-base-content/40">·</span>
                            {/if}
                            <span class="font-mono">{shortAccount(chip.account)}</span>
                            {#if recurringLabel(chip) !== null}
                                <span class="text-base-content/40">·</span><span data-testid="review-chip-fill">{recurringLabel(chip)}</span>
                            {/if}
                            <span class="text-base-content/45">{why(chip)}</span>
                        </button>
                    {/each}
                    <span class="key-hint text-[0.7rem] text-base-content/40"><kbd class="kbd kbd-xs">Tab</kbd> in the account fills it in</span>
                </div>
            {/if}
            {#if posting.comment !== "" || revealed.includes(at)}
                <div class="mr-14 ml-8 flex items-start gap-1.5" data-review-field="posting-comment-{at}">
                    <span class="pt-1 font-mono text-xs text-base-content/30" aria-hidden="true">;</span>
                    <GrowingTextarea
                        bind:value={posting.comment}
                        size="xs"
                        class="textarea-ghost text-base-content/60 focus:text-base-content"
                        placeholder="posting comment"
                        aria-label="Comment for posting {at + 1}"
                        autocomplete="off"
                        onfocus={() => !revealed.includes(at) && (revealed = [...revealed, at])}
                        {disabled}
                    />
                </div>
            {/if}
            {#if posting.cost !== null || posting.type !== "regular"}
                <p class="ml-8 text-xs text-base-content/45">
                    {[
                        posting.cost !== null ? `${posting.cost.kind === "unit" ? "@" : "@@"} ${posting.cost.amount.commodity} cost` : "",
                        posting.type !== "regular" ? `${posting.type} posting` : "",
                    ]
                        .filter((part) => part !== "")
                        .join(" · ")} kept as written
                </p>
            {/if}
        </li>
    {/each}
</ol>
<button type="button" class="btn mt-1 ml-7 gap-1 btn-ghost btn-xs" onclick={addRow} {disabled}>
    <span class="text-base leading-none">+</span> Add posting
</button>

{#if generated.length > 0}
    <ul class="mt-2 flex flex-col gap-1 rounded-box border border-dashed border-base-content/15 p-2" aria-label="Added by automatic posting rules">
        {#each generated as posting, at (at)}
            {@const unknown = isUncategorized(posting.account, classes)}
            <li class="flex items-center gap-2 text-sm text-base-content/60" data-testid="review-generated">
                <span class={["min-w-0 grow truncate font-mono italic", unknown && "text-warning"]} title={posting.account}>{posting.account}</span>
                <span class="shrink-0"><AmountCell amounts={posting.amounts} /></span>
                {#if posting.generated !== undefined}
                    {@const rule = posting.generated}
                    <button
                        type="button"
                        class="btn shrink-0 btn-ghost font-normal btn-xs"
                        title="Settings › Automatic postings: change the rule to change this posting"
                        onclick={() => onRule(rule.rule)}
                    >
                        from automatic posting rule <code>= {rule.query}</code>
                    </button>
                {/if}
            </li>
        {/each}
        {#if generated.some((posting) => isUncategorized(posting.account, classes))}
            <li class="text-xs text-warning">A rule adds an uncategorised posting. Fix it in the rule; until then this card can't be marked reviewed.</li>
        {/if}
    </ul>
{/if}
