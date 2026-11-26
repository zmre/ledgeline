<!-- A customer's open POs on its page, read-only view (invoicer PLAN 5b,
     S5–S6): each PO with its amount, expiry, what its sent invoices add up
     to and what is left; Add PO, Edit and Close save at once (the page writes
     `open_pos` by key, with the note's hash, so nothing else in the note
     moves). Closing takes the PO off the list (S5): new invoices stop
     suggesting it, invoices that quote it keep their reference, and the
     server notes it under `## Closed POs` in the note's body.

     A PO's attachment (its PDF, say) shows as a contract does
     (`AttachmentLink`); editing a PO here keeps it (the form attaches and
     removes them).

     The edit row is part of the page's form element, so its buttons are
     `type="button"` and Enter in a field saves the row, not the page. -->
<script lang="ts">
    import DateInput from "#ui/components/DateInput.svelte";
    import {posOf, type PoForm, type PoUsage} from "#lib/customers.js";
    import AttachmentLink from "./AttachmentLink.svelte";
    import ConfirmDialog from "./ConfirmDialog.svelte";

    let {
        usage,
        busy = false,
        onSave,
        invoiceHref,
        noteHref,
        fileHref,
        onOpenFile,
    }: {
        usage: PoUsage[];
        busy?: boolean;
        /** Write the whole list; true when saved. */
        onSave: (pos: PoForm[]) => Promise<boolean>;
        invoiceHref: (number: string) => string;
        /** The app's view of a note (an attachment that is one). */
        noteHref: (path: string) => string;
        /** Where a file in the books opens in a browser tab. */
        fileHref: (path: string) => string;
        /** Open a file in its viewer (the window); absent: a link, or text outside the books. */
        onOpenFile?: (path: string) => void;
    } = $props();

    /** The row being edited: its index, or `null` for a new PO. */
    let editing = $state<{at: number | null; reference: string; amount: string; expires: string} | null>(null);
    let closing = $state<PoUsage | null>(null);
    let rowError = $state<string | null>(null);

    function add(): void {
        rowError = null;
        editing = {at: null, reference: "", amount: "", expires: ""};
    }

    function edit(at: number): void {
        const po = usage[at];
        rowError = null;
        editing = {at, reference: po.reference, amount: po.amount ?? "", expires: po.expires ?? ""};
    }

    async function saveRow(): Promise<void> {
        if (editing === null) return;
        const row: PoForm = {
            reference: editing.reference.trim(),
            amount: editing.amount.trim() === "" ? null : editing.amount.trim(),
            expires: editing.expires === "" ? null : editing.expires,
            was: editing.at === null ? null : usage[editing.at].reference,
        };
        if (row.reference === "") {
            rowError = "Give the PO its reference, as the customer quotes it (PO #105).";
            return;
        }
        const list = posOf(usage);
        if (editing.at === null) list.push(row);
        else list[editing.at] = row;
        rowError = null;
        if (await onSave(list)) editing = null;
    }

    async function close(): Promise<void> {
        if (closing === null) return;
        const gone = closing.reference;
        if (await onSave(posOf(usage).filter((po) => po.reference !== gone))) closing = null;
    }

    /** The reference field takes focus when the row opens (a click on Add or Edit opened it). */
    function focusNow(node: HTMLInputElement): void {
        node.focus();
    }

    function rowKey(event: KeyboardEvent): void {
        if (event.key === "Enter") {
            event.preventDefault();
            void saveRow();
        } else if (event.key === "Escape") {
            event.preventDefault();
            editing = null;
        }
    }

    const usedText = (po: PoUsage): string | null => {
        if (po.amount !== null && po.remaining !== null) return `${po.remaining} left of ${po.amount}`;
        if (po.invoiced !== null) return `${po.invoiced} invoiced`;
        return po.amount;
    };
</script>

{#snippet editRow()}
    {#if editing !== null}
        <div class="grid gap-2 rounded-box bg-base-200/60 p-3 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)]" data-testid="po-edit">
            <label class="flex min-w-0 flex-col gap-1 text-sm"
                >Reference
                <input
                    class="input w-full input-sm"
                    bind:value={editing.reference}
                    onkeydown={rowKey}
                    placeholder="PO #105"
                    use:focusNow
                    data-testid="po-reference"
                />
            </label>
            <label class="flex min-w-0 flex-col gap-1 text-sm"
                >Amount
                <input class="input w-full input-sm" bind:value={editing.amount} onkeydown={rowKey} placeholder="$40,000" data-testid="po-amount" />
            </label>
            <label class="flex min-w-0 flex-col gap-1 text-sm"
                >Expires
                <DateInput class="w-full" bind:value={editing.expires} onkeydown={rowKey} aria-label="Expires" data-testid="po-expires" />
            </label>
            {#if rowError !== null}<p role="alert" class="text-sm text-error sm:col-span-3" data-testid="po-error">{rowError}</p>{/if}
            <div class="flex flex-wrap justify-end gap-2 sm:col-span-3">
                <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (editing = null)} data-testid="po-cancel">Cancel</button>
                <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => void saveRow()} data-testid="po-save"
                    >{busy ? "Saving…" : editing.at === null ? "Add PO" : "Save PO"}</button
                >
            </div>
        </div>
    {/if}
{/snippet}

<section class="rounded-box border border-base-300 p-4 text-sm" data-testid="customer-open-pos" aria-labelledby="open-pos-heading">
    <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
        <h2 id="open-pos-heading" class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Open POs</h2>
        {#if editing === null}
            <button type="button" class="btn btn-outline btn-xs" disabled={busy} onclick={add} data-testid="po-add">Add PO</button>
        {/if}
    </div>
    {#if usage.length === 0 && editing?.at !== null}
        <p class="text-base-content/60" data-testid="po-none">None. Add the customer's purchase orders to quote them on invoices.</p>
    {/if}
    <ul class="flex flex-col gap-2">
        {#each usage as po, at (po.reference)}
            {#if editing?.at === at}
                <li>{@render editRow()}</li>
            {:else}
                <li class="flex flex-wrap items-start justify-between gap-x-3 gap-y-1" data-testid="po-row" data-reference={po.reference}>
                    <div class="min-w-0">
                        <span class="font-medium break-words">{po.reference}</span>
                        {#if po.expired}<span class="ml-1 badge badge-sm badge-warning">Expired</span>{/if}
                        <div class="text-xs text-base-content/70">
                            {[usedText(po), po.expires === null ? null : `${po.expired ? "expired" : "expires"} ${po.expires}`]
                                .filter((t) => t !== null)
                                .join(" · ")}
                        </div>
                        {#if po.attachment != null}
                            <div class="text-xs" data-testid="po-attachment">
                                <AttachmentLink attachment={po.attachment} {noteHref} {fileHref} {onOpenFile} testid="po-attachment-link" />
                            </div>
                        {/if}
                        {#if po.invoices.length > 0}
                            <div class="text-xs text-base-content/70">
                                Invoiced on
                                <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes an href built with resolve() -->
                                {#each po.invoices as number, i (number)}<a class="link" href={invoiceHref(number)}>{number}</a>{i < po.invoices.length - 1
                                        ? ", "
                                        : ""}{/each}
                            </div>
                        {/if}
                    </div>
                    {#if editing === null}
                        <div class="flex gap-1">
                            <button
                                type="button"
                                class="btn btn-ghost btn-xs"
                                disabled={busy}
                                onclick={() => edit(at)}
                                aria-label="Edit {po.reference}"
                                data-testid="po-edit-button">Edit</button
                            >
                            <button
                                type="button"
                                class="btn btn-ghost btn-xs"
                                disabled={busy}
                                onclick={() => (closing = po)}
                                aria-label="Close {po.reference}"
                                data-testid="po-close-button">Close</button
                            >
                        </div>
                    {/if}
                </li>
            {/if}
        {/each}
        {#if editing !== null && editing.at === null}
            <li>{@render editRow()}</li>
        {/if}
    </ul>
</section>

{#if closing !== null}
    <ConfirmDialog
        title="Close {closing.reference}?"
        confirmLabel="Close PO"
        tone="warning"
        {busy}
        onConfirm={() => void close()}
        onCancel={() => (closing = null)}
        testid="po-close-dialog"
    >
        <p>
            It comes off this customer's open POs, so new invoices stop suggesting it, and is noted under “Closed POs” in the customer's note with what was
            invoiced against it. Invoices that quote it keep their reference.
        </p>
    </ConfirmDialog>
{/if}
