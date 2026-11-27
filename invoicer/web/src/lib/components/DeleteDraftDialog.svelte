<!-- Delete draft… (plan 46 T10): a draft was never in the books, so deleting
     it only removes its file; time billed on it goes back to Unbilled, and
     the dialog says how much. Focus starts on Cancel (ConfirmDialog). -->
<script lang="ts">
    import {onMount} from "svelte";
    import {deleteDraft} from "#lib/editorApi.js";
    import {errorText} from "#lib/labels.js";
    import {hoursWords} from "#lib/time/billing/words.js";
    import {fetchInvoiceTime} from "#lib/time/billing/invoiceTime.js";
    import type {Qty} from "#lib/time/types.js";
    import ConfirmDialog from "./ConfirmDialog.svelte";

    let {number, onDeleted, onCancel}: {number: string; onDeleted: (released: number) => void; onCancel: () => void} = $props();

    let time = $state<Qty | null>(null);
    let busy = $state(false);
    let error = $state<string | null>(null);

    onMount(() => {
        fetchInvoiceTime(number).then((t) => (time = t));
    });

    async function confirm(): Promise<void> {
        busy = true;
        error = null;
        try {
            const done = await deleteDraft(number);
            onDeleted(done.time?.released ?? 0);
        } catch (e: unknown) {
            error = errorText(e);
        } finally {
            busy = false;
        }
    }
</script>

<ConfirmDialog
    title="Delete draft {number}?"
    confirmLabel="Delete draft"
    tone="error"
    {busy}
    {error}
    onConfirm={() => void confirm()}
    {onCancel}
    testid="confirm-delete-draft"
>
    <p>Its file is deleted. A draft was never in the books, so nothing there changes.</p>
    {#if time !== null}
        <p data-testid="confirm-delete-draft-time">Its {hoursWords(time)} of time go back to Unbilled.</p>
    {/if}
</ConfirmDialog>
