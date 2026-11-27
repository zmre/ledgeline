<!-- "Your company's details have changed" (invoicer PLAN D18): a quiet line on a
     sent invoice whose copy of your company (name, address, email, phone, logo)
     differs from the company note now, offering Update from company. A draft
     never shows it: drafts use the company's current details anyway. Updating
     rewrites a sent invoice's copy and re-books it, so it asks first, as every
     step that writes the books does (ConfirmDialog: focus on Cancel, Escape
     cancels); the customer's PDF stays as it was. `confirming` is bindable so
     the page's key (`u`) can ask. Presentational: the page does the call. -->
<script lang="ts">
    import ConfirmDialog from "./ConfirmDialog.svelte";

    interface Props {
        number: string;
        status: string | null;
        /** The server's `company_changed`. */
        changed: boolean;
        busy?: boolean;
        error?: string | null;
        /** The question is open. */
        confirming?: boolean;
        onUpdate: () => void;
    }

    let {number, status, changed, busy = false, error = null, confirming = $bindable(false), onUpdate}: Props = $props();

    const sent = $derived(status === "sent");
</script>

{#if sent && (changed || error !== null)}
    <section class="flex flex-col gap-2 text-sm" aria-label="Your company's details" data-testid="company-notice">
        {#if changed}
            <p class="flex flex-wrap items-center gap-x-2 text-base-content/70">
                <span>Your company's details have changed since this invoice was sent.</span>
                <button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={() => (confirming = true)} data-testid="company-update">
                    {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                    Update from company…
                </button>
            </p>
        {/if}
        {#if error !== null}
            <div class="alert text-sm alert-error" role="alert" data-testid="company-error">{error}</div>
        {/if}
    </section>
{/if}

{#if sent && confirming}
    <ConfirmDialog
        title="Update invoice {number} from your company?"
        confirmLabel="Update"
        testid="company-confirm"
        onConfirm={() => {
            confirming = false;
            onUpdate();
        }}
        onCancel={() => (confirming = false)}
    >
        <p>Its name, address, email, phone and logo become your company's current ones, and it is re-booked.</p>
        <p class="text-base-content/70">The PDF your customer has stays as it was: export it again to send the new one.</p>
    </ConfirmDialog>
{/if}
