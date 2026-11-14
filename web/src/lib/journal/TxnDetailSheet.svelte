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
    // the whole transaction, read-only: date, status, description, every
    // posting with its full account and amount, tags and comments. The
    // card's actions sit under it, so a long press opens the same sheet.
    //
    // Presentational: the table works out which actions apply and what they
    // do. `dismissible` makes it the overlay the iOS app's back button closes.
    import {formatAmount} from "#lib/domain/money.js";
    import type {Posting, Transaction, TxnStatus} from "#lib/domain/types.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import StatusBadge from "./StatusBadge.svelte";

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

    const STATUS_LABEL: Record<TxnStatus, string> = {cleared: "Cleared", pending: "Pending", unmarked: "Unmarked"};

    const tagText = (tags: [string, string][]): string => tags.map(([name, value]) => (value === "" ? name : `${name}:${value}`)).join(", ");
    const amountText = (posting: Posting): string => (posting.amounts.length === 0 ? "—" : posting.amounts.map((amount) => formatAmount(amount)).join(", "));
    const negative = (posting: Posting): boolean => posting.amounts.length > 0 && posting.amounts.every((amount) => amount.qty.m < 0n);
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
            <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm text-base-content/70">
                <span class="font-mono whitespace-nowrap tabular-nums" data-testid="detail-date">{txn.date}</span>
                {#if txn.date2 !== undefined}<span class="font-mono text-xs" title="Secondary date">={txn.date2}</span>{/if}
                <StatusBadge status={txn.status} />
                <span>{STATUS_LABEL[txn.status]}</span>
                {#if txn.code !== ""}<span class="font-mono text-xs">({txn.code})</span>{/if}
            </div>
            <h2 class="mt-1 text-lg font-semibold [overflow-wrap:anywhere]" data-testid="detail-description">{txn.description || "(no description)"}</h2>
            {#if note !== null}<p class="mt-1 text-sm [overflow-wrap:anywhere] text-base-content/60">{note}</p>{/if}
            {#if txn.comment !== ""}
                <p class="mt-2 text-sm [overflow-wrap:anywhere] whitespace-pre-wrap text-base-content/70" data-testid="detail-comment">; {txn.comment}</p>
            {/if}
            {#if txn.tags.length > 0}
                <p class="mt-2 text-sm [overflow-wrap:anywhere] text-base-content/60" data-testid="detail-tags">Tags: {tagText(txn.tags)}</p>
            {/if}
            <ul class="mt-3 flex flex-col divide-y divide-base-300 rounded-box border border-base-300" data-testid="detail-postings">
                {#each txn.postings as posting, index (index)}
                    <li class="flex min-w-0 flex-col gap-0.5 px-3 py-2">
                        <div class="flex min-w-0 flex-wrap items-baseline justify-between gap-x-3">
                            <span class="min-w-0 font-medium [overflow-wrap:anywhere]">
                                {posting.account}
                                {#if posting.generated !== undefined}
                                    <span class="badge border-dashed badge-ghost badge-sm font-normal italic" title="Added by = {posting.generated.query}"
                                        >auto</span
                                    >
                                {/if}
                            </span>
                            <span
                                class={[
                                    "ml-auto shrink-0 text-right font-mono text-sm tabular-nums",
                                    negative(posting) && "text-error",
                                    posting.elided && "text-base-content/50",
                                ]}
                                title={posting.elided ? "Not written in the file: the amount that balances the transaction" : undefined}
                                >{amountText(posting)}</span
                            >
                        </div>
                        {#if posting.comment !== ""}
                            <span class="text-xs [overflow-wrap:anywhere] whitespace-pre-wrap text-base-content/60">; {posting.comment}</span>
                        {/if}
                    </li>
                {/each}
            </ul>
        </div>
        <!-- Natural widths that wrap, never truncate: at a big interface scale
             "Mark reviewed" needs most of a line. -->
        <div class="flex shrink-0 flex-wrap gap-2 border-t border-base-300 px-4 pt-3 pb-[calc(1rem+var(--ll-bottom-inset))]" data-testid="detail-actions">
            {#each actions as action (action.label)}
                <button
                    type="button"
                    class={["btn grow btn-sm", action.tone === "primary" ? "btn-primary" : action.tone === "error" ? "btn-outline btn-error" : ""]}
                    onclick={action.run}
                    data-testid={action.testid}>{action.label}</button
                >
            {/each}
            <button type="button" class="btn grow btn-ghost btn-sm" onclick={onClose} data-testid="detail-close">Close</button>
        </div>
    </div>
    <button type="button" class="modal-backdrop" aria-label="Close" onclick={onClose}></button>
</div>
