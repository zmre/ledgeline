<!-- The forecast journal: the scenario's generated transactions and nothing
     else, date-ascending — `hledger print --forecast tag:generated`
     (plans/28, follow-ups 2026-10-26). Read-only.

     Laid out as `print` lays a transaction out, because that is the shape that
     shows what a rule actually does: a header line (date, description, and
     which part of the scenario produced it), then one line per posting with
     its account and amount. The journal tab's own TransactionTable is not
     reused: it is bound to the windowed journal store and to editing, and a
     generated transaction is in neither. Its amount cell and account label
     are.

     Paginated rather than virtualised: each transaction is as tall as its
     posting count, which fixed-pitch windowing cannot represent, and a page
     of 100 keeps a 24-month, many-rule forecast cheap to render. The
     engine caps the list (MAX_JOURNAL_ENTRIES) and says when it did. -->
<script lang="ts">
    import AccountLabel from "#lib/components/AccountLabel.svelte";
    import AmountCell from "#lib/journal/AmountCell.svelte";
    import type {ForecastEntry, ForecastJournal} from "../types";

    let {journal}: {journal: ForecastJournal} = $props();

    const PAGE_SIZE = 100;
    let page = $state(0);

    const pages = $derived(Math.max(1, Math.ceil(journal.entries.length / PAGE_SIZE)));
    // A shorter forecast (fewer periods, a deleted rule) can leave the page past the end.
    const at = $derived(Math.min(page, pages - 1));
    const shown = $derived(journal.entries.slice(at * PAGE_SIZE, (at + 1) * PAGE_SIZE));
    const first = $derived(journal.entries.length === 0 ? 0 : at * PAGE_SIZE + 1);
    const last = $derived(Math.min((at + 1) * PAGE_SIZE, journal.entries.length));

    /** The rule's period as hledger marks it (`~ monthly`), else what kind of entry this is. */
    function originLabel(entry: ForecastEntry): string {
        if (entry.origin.kind === "event") return "event";
        if (entry.origin.kind === "growth") return "growth";
        return entry.txn.tags.find(([name]) => name === "generated-transaction")?.[1] ?? "rule";
    }
</script>

{#snippet pager()}
    {#if pages > 1}
        <div class="join">
            <button type="button" class="btn join-item btn-xs" disabled={at === 0} onclick={() => (page = at - 1)} aria-label="Previous page">‹</button>
            <span class="btn pointer-events-none join-item font-normal btn-xs">Page {at + 1} of {pages}</span>
            <button type="button" class="btn join-item btn-xs" disabled={at >= pages - 1} onclick={() => (page = at + 1)} aria-label="Next page">›</button>
        </div>
    {/if}
{/snippet}

<div class="flex flex-col gap-2" data-testid="forecast-journal">
    <div class="flex flex-wrap items-center justify-between gap-2 text-xs text-base-content/60">
        <p data-testid="forecast-journal-count">
            {#if journal.entries.length === 0}
                The scenario generates no transactions over these periods.
            {:else}
                Generated transactions {first}–{last} of {journal.total}{journal.truncated ? ` (the first ${journal.entries.length} are listed)` : ""}.
            {/if}
        </p>
        {@render pager()}
    </div>

    {#if shown.length > 0}
        <div class="overflow-x-auto rounded-box border border-base-content/10">
            <table class="table table-sm [&_tbody_tr]:border-0">
                <thead>
                    <tr class="text-[0.65rem] text-base-content/40">
                        <th class="w-28 font-normal">Date</th>
                        <th class="font-normal">Description · account</th>
                        <th class="w-36 text-right font-normal">Amount</th>
                    </tr>
                </thead>
                {#each shown as entry (entry.txn.index)}
                    <tbody class="border-t-2 border-base-content/10" data-testid="forecast-entry" data-origin={entry.origin.kind} data-key={entry.origin.key}>
                        <tr class="[&>td]:pt-3 [&>td]:pb-1">
                            <td class="align-baseline font-mono text-xs whitespace-nowrap tabular-nums">{entry.txn.date}</td>
                            <td class="font-medium" colspan="2">
                                <span>{entry.txn.description}</span>
                                <span class="ml-2 badge badge-ghost font-mono badge-xs" title="From the scenario's {entry.origin.kind} {entry.origin.key}"
                                    >{originLabel(entry)}</span
                                >
                            </td>
                        </tr>
                        {#each entry.txn.postings as posting, p (p)}
                            <tr class="text-base-content/80 [&>td]:py-0.5">
                                <td></td>
                                <td class="pl-6" title={posting.account}><AccountLabel name={posting.account} maxWidth={480} /></td>
                                <td><AmountCell amounts={posting.amounts} /></td>
                            </tr>
                        {/each}
                    </tbody>
                {/each}
            </table>
        </div>
        <div class="flex justify-end">{@render pager()}</div>
    {/if}
</div>
