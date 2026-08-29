<script lang="ts">
    // The whole-transaction popup (daisyUI modal), shared by ADD (blank → POST)
    // and EDIT-ALL (prefilled from a transaction → PUT). Fields: date, status,
    // code, description, and a dynamic list of posting rows (account + optional
    // amount + commodity). Leaving a posting's amount blank marks the elided leg.
    // Client-side validation is minimal (date + ≥1 posting); the engine does the
    // real balancing and its 400 message is shown inline. A 409 closes the popup
    // and lets the page's "changed on disk" banner take over.
    import {tick} from "svelte";
    import {decToInput, dominantCommodity, formToBody, txnToForm, validateForm, blankForm, emptyPosting, type TxnForm} from "$lib/api/editMapping";
    import type {WireBalanceAssertion, WirePostingType} from "$lib/api/native";
    import {dismissible} from "$lib/keys/dismissible";
    import {localToday} from "$lib/stores/filters.svelte";
    import {editing} from "$lib/stores/editing.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {txnModal} from "./modalState.svelte";
    import AccountInput from "./AccountInput.svelte";

    let form = $state<TxnForm>(blankForm(localToday(), "$"));
    let clientErrors = $state<string[]>([]);
    let serverError = $state<string | null>(null);
    let submitting = $state(false);
    let confirmingDelete = $state(false);

    // Seed the form ONCE each time the modal opens (add → blank, edit → prefill),
    // never on later reactive ticks, so it doesn't clobber what the user typed.
    let wasOpen = false;
    let dateField = $state<HTMLInputElement | null>(null);
    $effect(() => {
        const open = txnModal.open;
        if (open && !wasOpen) {
            const target = txnModal.target;
            form = txnModal.mode === "edit" && target !== null ? txnToForm(target) : blankForm(localToday(), dominantCommodity(journal.txns));
            clientErrors = [];
            serverError = null;
            confirmingDelete = false;
            // Nothing used to be focused when this opened, so Escape did nothing
            // at all until you clicked into a field — the handler was bubble-phase
            // on a wrapper the user was not inside. (On macOS WebKit a click does
            // not even focus a <button>, so focus was typically still on <body>.)
            //
            // After a tick, because the modal is always mounted and only becomes
            // visible when `modal-open` lands: `.focus()` on a `visibility:hidden`
            // element is a silent no-op.
            void tick().then(() => dateField?.focus());
        }
        wasOpen = open;
    });

    const title = $derived(txnModal.mode === "edit" ? "Edit transaction" : "Add transaction");
    const submitLabel = $derived(txnModal.mode === "edit" ? "Save changes" : "Add transaction");

    function addRow(): void {
        form.postings = [...form.postings, emptyPosting(dominantCommodity(journal.txns))];
    }
    function removeRow(index: number): void {
        form.postings = form.postings.filter((_, i) => i !== index);
    }

    async function submit(): Promise<void> {
        clientErrors = validateForm(form);
        if (clientErrors.length > 0) return;
        serverError = null;
        submitting = true;
        const body = formToBody(form, dominantCommodity(journal.txns));
        const target = txnModal.target;
        const result = txnModal.mode === "edit" && target !== null ? await editing.replace(target.index, body) : await editing.add(body);
        submitting = false;
        if (result.ok) {
            txnModal.close();
            return;
        }
        // A 409 already flipped the page-level conflict banner + refetched.
        if (result.failure.kind === "conflict") {
            txnModal.close();
            return;
        }
        serverError = result.failure.message;
    }

    async function confirmDelete(): Promise<void> {
        const target = txnModal.target;
        if (target === null) return;
        submitting = true;
        const result = await editing.remove(target.index);
        submitting = false;
        if (result.ok || result.failure.kind === "conflict") {
            txnModal.close();
            return;
        }
        serverError = result.failure.message;
    }

    function onSubmit(event: SubmitEvent): void {
        event.preventDefault();
        if (!submitting) void submit();
    }

    function onKeydown(event: KeyboardEvent): void {
        // Cmd/Ctrl+Enter saves from anywhere. Every field here is single-line, so
        // plain Enter already submits natively via the <form> — this is the chord
        // power users reach for regardless, and it keeps working if a multi-line
        // field is ever added.
        //
        // First modifier read anywhere in this codebase; `metaKey` on macOS,
        // `ctrlKey` elsewhere, accepting either rather than sniffing the platform.
        if (event.key !== "Enter" || !(event.metaKey || event.ctrlKey)) return;
        // A completion popup takes precedence: it has already prevented this.
        if (event.defaultPrevented) return;
        event.preventDefault();
        if (!submitting) void submit();
    }

    /** Human name for a non-regular posting type (the popup preserves these but doesn't edit them). */
    function postingTypeLabel(type: WirePostingType): string {
        return type === "virtual" ? "Unbalanced virtual" : "Balanced virtual";
    }

    /** The assertion as it appears in the journal, e.g. `== $500.00` or `=* 10 EUR`. */
    function assertionLabel(assertion: WireBalanceAssertion): string {
        const op = `${assertion.total ? "==" : "="}${assertion.inclusive ? "*" : ""}`;
        const qty = decToInput({m: BigInt(assertion.amount.quantity.mantissa), p: assertion.amount.quantity.places});
        return `${op} ${assertion.amount.commodity}${qty}`;
    }
</script>

<!-- `dismissible` now owns Escape and focus restore. It checks `defaultPrevented`,
     so the account combobox's first Escape (which closes its own popup) does not
     also close this modal — the bug that discarded a half-typed transaction.
     `active` matters because this modal is ALWAYS mounted: without it the action
     would hold the top of the dismissal stack forever. -->
<div
    class="modal"
    class:modal-open={txnModal.open}
    role="dialog"
    aria-modal="true"
    aria-label={title}
    onkeydown={onKeydown}
    tabindex="-1"
    use:dismissible={{active: txnModal.open, trap: true, onDismiss: () => !submitting && txnModal.close()}}
>
    <div class="modal-box max-w-2xl">
        <h3 class="mb-3 text-lg font-semibold">{title}</h3>

        <!-- A real <form>, so Enter saves from any field via native implicit
             submission rather than a hand-rolled keydown table. Every field here
             is single-line (the "Comment / tags" field is an <input>, not a
             textarea), so there is no case where Enter should mean "newline".
             Every other button in here is already explicitly type="button", so
             only the save button changes. Precedent: ServerSetupModal. -->
        <form onsubmit={onSubmit}>
            <div class="grid grid-cols-1 gap-3 sm:grid-cols-4">
                <label class="form-control sm:col-span-1">
                    <span class="label-text text-xs">Date</span>
                    <input bind:this={dateField} type="date" class="input w-full input-sm" bind:value={form.date} disabled={submitting} aria-label="Date" />
                </label>
                <label class="form-control sm:col-span-1">
                    <span class="label-text text-xs">Secondary date</span>
                    <input type="date" class="input w-full input-sm" bind:value={form.date2} disabled={submitting} aria-label="Secondary date" />
                </label>
                <label class="form-control sm:col-span-1">
                    <span class="label-text text-xs">Status</span>
                    <select class="select w-full select-sm" bind:value={form.status} disabled={submitting} aria-label="Status">
                        <option value="unmarked">Unmarked</option>
                        <option value="pending">Pending (!)</option>
                        <option value="cleared">Cleared (*)</option>
                    </select>
                </label>
                <label class="form-control sm:col-span-1">
                    <span class="label-text text-xs">Code</span>
                    <input type="text" class="input w-full input-sm" bind:value={form.code} disabled={submitting} placeholder="opt." aria-label="Code" />
                </label>
            </div>

            <div class="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
                <label class="form-control">
                    <span class="label-text text-xs">Description</span>
                    <input
                        type="text"
                        class="input w-full input-sm"
                        bind:value={form.description}
                        disabled={submitting}
                        placeholder="payee | note"
                        aria-label="Description"
                    />
                </label>
                <label class="form-control">
                    <span class="label-text text-xs">Comment / tags</span>
                    <input
                        type="text"
                        class="input w-full input-sm"
                        bind:value={form.comment}
                        disabled={submitting}
                        placeholder="note; key:value adds a tag"
                        aria-label="Comment or tags"
                    />
                    <span class="label-text-alt mt-1 text-xs text-base-content/50"
                        >A <code>key:value</code> pair (e.g. <code>category:food</code>) becomes a tag.</span
                    >
                </label>
            </div>

            <div class="mt-4">
                <div class="mb-1 flex items-center justify-between">
                    <span class="label-text text-xs font-medium">Postings</span>
                    <span class="text-xs text-base-content/50">Leave an amount blank for the inferred leg</span>
                </div>
                <div class="flex flex-col gap-3">
                    {#each form.postings as posting, index (index)}
                        <div class="flex flex-col gap-1">
                            <div class="flex items-start gap-2">
                                <div class="min-w-0 grow-[3] basis-0">
                                    <AccountInput
                                        bind:value={posting.account}
                                        accountNames={journal.accountNames}
                                        placeholder="account:sub"
                                        disabled={submitting}
                                    />
                                </div>
                                <input
                                    type="text"
                                    inputmode="decimal"
                                    class="input min-w-0 grow-[2] basis-0 text-right font-mono input-sm"
                                    bind:value={posting.amount}
                                    disabled={submitting}
                                    placeholder="auto"
                                    aria-label="Amount for posting {index + 1}"
                                />
                                <input
                                    type="text"
                                    class="input w-16 shrink-0 input-sm"
                                    bind:value={posting.commodity}
                                    disabled={submitting}
                                    placeholder="$"
                                    aria-label="Commodity for posting {index + 1}"
                                />
                                <button
                                    type="button"
                                    class="btn btn-square shrink-0 btn-ghost btn-sm"
                                    onclick={() => removeRow(index)}
                                    disabled={submitting || form.postings.length <= 1}
                                    aria-label="Remove posting {index + 1}"
                                    title="Remove posting"
                                >
                                    ✕
                                </button>
                            </div>
                            <input
                                type="text"
                                class="input ml-1 w-full input-xs"
                                bind:value={posting.comment}
                                disabled={submitting}
                                placeholder="posting comment (optional)"
                                aria-label="Comment for posting {index + 1}"
                            />
                            {#if posting.cost !== null}
                                <div class="pl-1 text-xs text-base-content/50">
                                    {posting.cost.kind === "unit" ? "@" : "@@"}
                                    {posting.cost.amount.commodity} cost preserved on save
                                </div>
                            {/if}
                            {#if posting.type !== "regular"}
                                <div class="pl-1 text-xs text-base-content/50">
                                    {postingTypeLabel(posting.type)} posting — written as
                                    <code>{posting.type === "virtual" ? `(${posting.account || "account"})` : `[${posting.account || "account"}]`}</code>
                                </div>
                            {/if}
                            {#if posting.balanceAssertion !== null}
                                <div class="flex items-center gap-2 pl-1 text-xs text-base-content/50">
                                    <span>
                                        Balance assertion <code>{assertionLabel(posting.balanceAssertion)}</code> preserved on save
                                    </span>
                                    <button
                                        type="button"
                                        class="btn btn-ghost btn-xs"
                                        onclick={() => (posting.balanceAssertion = null)}
                                        disabled={submitting}
                                        aria-label="Remove the balance assertion on posting {index + 1}"
                                    >
                                        Remove
                                    </button>
                                </div>
                            {/if}
                        </div>
                    {/each}
                </div>
                <button type="button" class="btn mt-2 gap-1 btn-ghost btn-xs" onclick={addRow} disabled={submitting}>
                    <span class="text-base leading-none">+</span> Add posting
                </button>
            </div>

            {#if clientErrors.length > 0}
                <ul class="mt-3 list-inside list-disc text-sm text-error" role="alert">
                    {#each clientErrors as message (message)}
                        <li>{message}</li>
                    {/each}
                </ul>
            {/if}
            {#if serverError !== null}
                <div class="mt-3 alert py-2 text-sm alert-error" role="alert">
                    <span class="break-words">{serverError}</span>
                </div>
            {/if}

            <div class="modal-action mt-4 items-center justify-between">
                <div>
                    {#if txnModal.mode === "edit"}
                        {#if confirmingDelete}
                            <span class="text-xs">Delete this transaction?</span>
                            <button type="button" class="btn ml-2 btn-error btn-sm" onclick={confirmDelete} disabled={submitting}>Confirm delete</button>
                            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirmingDelete = false)} disabled={submitting}>Keep</button>
                        {:else}
                            <button type="button" class="btn btn-outline btn-error btn-sm" onclick={() => (confirmingDelete = true)} disabled={submitting}
                                >Delete</button
                            >
                        {/if}
                    {/if}
                </div>
                <div class="flex gap-2">
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => txnModal.close()} disabled={submitting}>Cancel</button>
                    <button type="submit" class="btn btn-primary btn-sm" disabled={submitting}>
                        {#if submitting}<span class="loading loading-xs loading-spinner"></span>{/if}
                        {submitLabel}
                    </button>
                </div>
            </div>
        </form>
    </div>
    <button type="button" class="modal-backdrop" aria-label="Close" onclick={() => !submitting && txnModal.close()}>close</button>
</div>
