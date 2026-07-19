<script lang="ts">
    // The whole-transaction popup (daisyUI modal), shared by ADD (blank → POST)
    // and EDIT-ALL (prefilled from a transaction → PUT). Fields: date, status,
    // code, description, and a dynamic list of posting rows (account + optional
    // amount + commodity). Leaving a posting's amount blank marks the elided leg.
    // Client-side validation is minimal (date + ≥1 posting); the engine does the
    // real balancing and its 400 message is shown inline. A 409 closes the popup
    // and lets the page's "changed on disk" banner take over.
    import {dominantCommodity, formToBody, txnToForm, validateForm, blankForm, emptyPosting, type TxnForm} from "$lib/api/editMapping";
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
    $effect(() => {
        const open = txnModal.open;
        if (open && !wasOpen) {
            const target = txnModal.target;
            form = txnModal.mode === "edit" && target !== null ? txnToForm(target) : blankForm(localToday(), dominantCommodity(journal.txns));
            clientErrors = [];
            serverError = null;
            confirmingDelete = false;
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
        const body = formToBody(form);
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

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Escape" && !submitting) txnModal.close();
    }
</script>

<div class="modal" class:modal-open={txnModal.open} role="dialog" aria-modal="true" aria-label={title} onkeydown={onKeydown} tabindex="-1">
    <div class="modal-box max-w-2xl">
        <h3 class="mb-3 text-lg font-semibold">{title}</h3>

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-4">
            <label class="form-control sm:col-span-1">
                <span class="label-text text-xs">Date</span>
                <input type="date" class="input input-sm w-full" bind:value={form.date} disabled={submitting} aria-label="Date" />
            </label>
            <label class="form-control sm:col-span-1">
                <span class="label-text text-xs">Status</span>
                <select class="select select-sm w-full" bind:value={form.status} disabled={submitting} aria-label="Status">
                    <option value="unmarked">Unmarked</option>
                    <option value="pending">Pending (!)</option>
                    <option value="cleared">Cleared (*)</option>
                </select>
            </label>
            <label class="form-control sm:col-span-1">
                <span class="label-text text-xs">Code</span>
                <input type="text" class="input input-sm w-full" bind:value={form.code} disabled={submitting} placeholder="opt." aria-label="Code" />
            </label>
            <label class="form-control sm:col-span-1">
                <span class="label-text text-xs">Description</span>
                <input
                    type="text"
                    class="input input-sm w-full"
                    bind:value={form.description}
                    disabled={submitting}
                    placeholder="payee | note"
                    aria-label="Description"
                />
            </label>
        </div>

        <div class="mt-4">
            <div class="mb-1 flex items-center justify-between">
                <span class="label-text text-xs font-medium">Postings</span>
                <span class="text-base-content/50 text-xs">Leave an amount blank for the inferred leg</span>
            </div>
            <div class="flex flex-col gap-2">
                {#each form.postings as posting, index (index)}
                    <div class="flex items-start gap-2">
                        <div class="min-w-0 grow-[3] basis-0">
                            <AccountInput bind:value={posting.account} accountNames={journal.accountNames} placeholder="account:sub" disabled={submitting} />
                        </div>
                        <input
                            type="text"
                            inputmode="decimal"
                            class="input input-sm min-w-0 grow-[2] basis-0 text-right font-mono"
                            bind:value={posting.amount}
                            disabled={submitting}
                            placeholder="auto"
                            aria-label="Amount for posting {index + 1}"
                        />
                        <input
                            type="text"
                            class="input input-sm w-16 shrink-0"
                            bind:value={posting.commodity}
                            disabled={submitting}
                            placeholder="$"
                            aria-label="Commodity for posting {index + 1}"
                        />
                        <button
                            type="button"
                            class="btn btn-ghost btn-sm btn-square shrink-0"
                            onclick={() => removeRow(index)}
                            disabled={submitting || form.postings.length <= 1}
                            aria-label="Remove posting {index + 1}"
                            title="Remove posting"
                        >
                            ✕
                        </button>
                    </div>
                    {#if posting.cost !== null}
                        <div class="text-base-content/50 -mt-1 pl-1 text-xs">
                            {posting.cost.kind === "unit" ? "@" : "@@"}
                            {posting.cost.amount.commodity} cost preserved on save
                        </div>
                    {/if}
                {/each}
            </div>
            <button type="button" class="btn btn-ghost btn-xs mt-2 gap-1" onclick={addRow} disabled={submitting}>
                <span class="text-base leading-none">+</span> Add posting
            </button>
        </div>

        {#if clientErrors.length > 0}
            <ul class="text-error mt-3 list-inside list-disc text-sm" role="alert">
                {#each clientErrors as message (message)}
                    <li>{message}</li>
                {/each}
            </ul>
        {/if}
        {#if serverError !== null}
            <div class="alert alert-error mt-3 py-2 text-sm" role="alert">
                <span class="break-words">{serverError}</span>
            </div>
        {/if}

        <div class="modal-action mt-4 items-center justify-between">
            <div>
                {#if txnModal.mode === "edit"}
                    {#if confirmingDelete}
                        <span class="text-xs">Delete this transaction?</span>
                        <button type="button" class="btn btn-error btn-sm ml-2" onclick={confirmDelete} disabled={submitting}>Confirm delete</button>
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
                <button type="button" class="btn btn-primary btn-sm" onclick={submit} disabled={submitting}>
                    {#if submitting}<span class="loading loading-spinner loading-xs"></span>{/if}
                    {submitLabel}
                </button>
            </div>
        </div>
    </div>
    <button type="button" class="modal-backdrop" aria-label="Close" onclick={() => !submitting && txnModal.close()}>close</button>
</div>
