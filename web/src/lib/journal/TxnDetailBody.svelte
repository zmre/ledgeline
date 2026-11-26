<script lang="ts">
    // A transaction, read-only and whole: date, status and description, the
    // comment, the "Tags:" line, then every posting with its full account,
    // amount and comment. The body of the journal card's detail sheet (plan
    // 42), and of a settlement card's "Details" (plan 45 Phase 3b), where
    // `highlight` marks the posting the card matched so it stands out among
    // the transaction's others. Comments show their free text only: the tags
    // they carry are already on the "Tags:" line.
    import {commentFreeText} from "#lib/api/editMapping.js";
    import {formatAmount} from "#lib/domain/money.js";
    import type {Posting, Transaction, TxnStatus} from "#lib/domain/types.js";
    import StatusBadge from "./StatusBadge.svelte";

    let {
        txn,
        note = null,
        highlight = null,
        compact = false,
    }: {
        txn: Transaction;
        /** A line under the heading, e.g. a transfer's two accounts or a rejection. */
        note?: string | null;
        /** The posting (index into `txn.postings`) to mark, or null. */
        highlight?: number | null;
        /** Inside a card: the description is a line of text, not the sheet's heading. */
        compact?: boolean;
    } = $props();

    const STATUS_LABEL: Record<TxnStatus, string> = {cleared: "Cleared", pending: "Pending", unmarked: "Unmarked"};

    const tagText = (tags: [string, string][]): string => tags.map(([name, value]) => (value === "" ? name : `${name}:${value}`)).join(", ");
    const amountText = (posting: Posting): string => (posting.amounts.length === 0 ? "—" : posting.amounts.map((amount) => formatAmount(amount)).join(", "));
    const txnComment = $derived(commentFreeText(txn.comment));
    const negative = (posting: Posting): boolean => posting.amounts.length > 0 && posting.amounts.every((amount) => amount.qty.m < 0n);
</script>

<div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted">
    <span class="font-mono whitespace-nowrap tabular-nums" data-testid="detail-date">{txn.date}</span>
    {#if txn.date2 !== undefined}<span class="font-mono text-xs" title="Secondary date">={txn.date2}</span>{/if}
    <StatusBadge status={txn.status} />
    <span>{STATUS_LABEL[txn.status]}</span>
    {#if txn.code !== ""}<span class="font-mono text-xs">({txn.code})</span>{/if}
</div>
{#if compact}
    <p class="mt-1 font-semibold [overflow-wrap:anywhere]" data-testid="detail-description">{txn.description || "(no description)"}</p>
{:else}
    <h2 class="mt-1 text-lg font-semibold [overflow-wrap:anywhere]" data-testid="detail-description">{txn.description || "(no description)"}</h2>
{/if}
{#if note !== null}<p class="mt-1 text-sm [overflow-wrap:anywhere] text-muted">{note}</p>{/if}
{#if txnComment !== ""}
    <p class="mt-2 text-sm [overflow-wrap:anywhere] whitespace-pre-wrap text-muted" data-testid="detail-comment">{txnComment}</p>
{/if}
{#if txn.tags.length > 0}
    <p class="mt-2 text-sm [overflow-wrap:anywhere] text-muted" data-testid="detail-tags">Tags: {tagText(txn.tags)}</p>
{/if}
<ul class="mt-3 flex flex-col divide-y divide-base-300 rounded-box border border-base-300" data-testid="detail-postings">
    {#each txn.postings as posting, index (index)}
        {@const postingComment = commentFreeText(posting.comment)}
        <li
            class={["flex min-w-0 flex-col gap-0.5 px-3 py-2", highlight === index && "border-l-4 border-l-primary bg-primary/10"]}
            aria-current={highlight === index ? "true" : undefined}
            data-highlighted={highlight === index ? "" : undefined}
        >
            <div class="flex min-w-0 flex-wrap items-baseline justify-between gap-x-3">
                <span class="min-w-0 font-medium [overflow-wrap:anywhere]">
                    {posting.account}
                    {#if posting.generated !== undefined}
                        <span class="badge border-dashed badge-ghost badge-sm font-normal italic" title="Added by = {posting.generated.query}">auto</span>
                    {/if}
                </span>
                <span
                    class={["ml-auto shrink-0 text-right font-mono text-sm tabular-nums", negative(posting) && "text-error", posting.elided && "text-muted"]}
                    title={posting.elided ? "Not written in the file: the amount that balances the transaction" : undefined}>{amountText(posting)}</span
                >
            </div>
            {#if postingComment !== ""}
                <span class="text-xs [overflow-wrap:anywhere] whitespace-pre-wrap text-muted" data-testid="detail-posting-comment">{postingComment}</span>
            {/if}
        </li>
    {/each}
</ul>
