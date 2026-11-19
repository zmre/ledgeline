<script lang="ts" module>
    export interface DetailAction {
        label: string;
        run: () => void;
        tone?: "primary" | "error";
        testid?: string;
    }
</script>

<script lang="ts">
    // A journal card's detail sheet (plan 42): what a tap on a narrow card
    // opens. A card has room for one line of who and two of where; this shows
    // the whole transaction, read-only (TxnDetailBody: date, status,
    // description, every posting with its full account and amount, tags and
    // comments). Under it,
    // at most two buttons: Edit, and Mark reviewed when that applies. A tap
    // outside the sheet (its backdrop) closes it, as do Escape and the iOS
    // app's back button. Comments show their free text only: the tags they
    // carry are already on the "Tags:" line.
    //
    // Presentational: the table works out which actions apply and what they
    // do. `dismissible` makes it the overlay the iOS app's back button closes.
    import type {Transaction} from "#lib/domain/types.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import TxnDetailBody from "./TxnDetailBody.svelte";

    let {
        txn,
        note = null,
        actions,
        onClose,
    }: {
        txn: Transaction;
        /** A line under the heading, e.g. a transfer's two accounts or a rejection. */
        note?: string | null;
        actions: DetailAction[];
        onClose: () => void;
    } = $props();
</script>

<div
    class="modal modal-open modal-bottom"
    role="dialog"
    aria-modal="true"
    aria-label="Transaction {txn.date} {txn.description}"
    data-testid="row-menu"
    use:dismissible={{onDismiss: onClose}}
>
    <!-- The body scrolls; the actions stay at the bottom of the sheet. -->
    <div class="modal-box flex max-h-[85dvh] min-w-0 flex-col overflow-x-hidden p-0" data-testid="txn-detail">
        <div class="min-w-0 overflow-x-hidden overflow-y-auto overscroll-contain px-4 pt-4 pb-2">
            <TxnDetailBody {txn} {note} />
        </div>
        <!-- Natural widths that wrap, never truncate: at a big interface scale
             "Mark reviewed" needs most of a line. -->
        {#if actions.length > 0}
            <div class="flex shrink-0 flex-wrap gap-2 border-t border-base-300 px-4 pt-3 pb-[calc(1rem+var(--ll-bottom-inset))]" data-testid="detail-actions">
                {#each actions as action (action.label)}
                    <button
                        type="button"
                        class={["btn grow btn-sm", action.tone === "primary" ? "btn-primary" : action.tone === "error" ? "btn-outline btn-error" : ""]}
                        onclick={action.run}
                        data-testid={action.testid}>{action.label}</button
                    >
                {/each}
            </div>
        {:else}
            <div class="shrink-0 pb-[var(--ll-bottom-inset)]"></div>
        {/if}
    </div>
    <!-- The rest of the screen: a tap outside the sheet closes it. -->
    <button type="button" class="modal-backdrop" aria-label="Close" onclick={onClose} data-testid="detail-backdrop"></button>
</div>
