<!-- "Totals don't match the rows" (invoicer PLAN, Phase 3 feedback F10): the
     amounts written in the bill table that the arithmetic disagrees with,
     stored next to computed, and Recalculate. A draft is recalculated at
     once. A sent invoice's amount is booked, so it asks first: recalculating
     re-books it. A void one is history: no button. Presentational: the page
     does the call. -->
<script lang="ts">
    import type {Mismatch} from "#lib/invoices.js";

    interface Props {
        number: string;
        status: string | null;
        items: Mismatch[];
        busy?: boolean;
        error?: string | null;
        /** Recalculate; `rebook` once the user confirmed re-booking a sent invoice. */
        onRecalculate: (rebook: boolean) => void;
    }

    let {number, status, items, busy = false, error = null, onRecalculate}: Props = $props();

    let confirming = $state(false);
</script>

{#if items.length > 0}
    <section class="rounded-box border border-warning bg-warning/10 p-3 text-sm" aria-labelledby="totals-title" data-testid="totals-notice">
        <p id="totals-title" class="font-medium">Totals don't match the rows</p>
        <table class="table mt-1 w-auto table-xs">
            <thead>
                <tr><th>Row</th><th class="text-right">Written</th><th class="text-right">Computed</th></tr>
            </thead>
            <tbody>
                {#each items as item (item.row)}
                    <tr data-testid="totals-row">
                        <td>{item.label}</td>
                        <td class="text-right tabular-nums">{item.written}</td>
                        <td class="text-right tabular-nums">{item.computed}</td>
                    </tr>
                {/each}
            </tbody>
        </table>

        {#if error !== null}
            <div class="mt-2 alert text-sm alert-error" role="alert" data-testid="totals-error">{error}</div>
        {/if}

        {#if status === "void"}
            <p class="mt-2 text-base-content/70">It is void: its amounts stay as they were booked.</p>
        {:else if status === "sent" && confirming}
            <div class="mt-2 rounded-box border border-base-300 bg-base-200 p-3" role="alertdialog" aria-labelledby="rebook-title" data-testid="totals-confirm">
                <p id="rebook-title" class="font-medium">Recalculate and re-book invoice {number}?</p>
                <p class="mt-1 text-base-content/70">
                    It is sent, so its amount is in the books. Recalculating rewrites its amounts and replaces its transaction with the new total. It is refused
                    for an invoice dated in a closed period: void it and reissue it instead.
                </p>
                <div class="mt-3 flex flex-wrap gap-2">
                    <button
                        type="button"
                        class="btn btn-sm btn-warning"
                        disabled={busy}
                        onclick={() => {
                            confirming = false;
                            onRecalculate(true);
                        }}
                        data-testid="totals-rebook">Recalculate and re-book</button
                    >
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirming = false)}>Cancel</button>
                </div>
            </div>
        {:else}
            <button
                type="button"
                class="btn mt-2 btn-sm btn-warning"
                disabled={busy}
                onclick={() => (status === "sent" ? (confirming = true) : onRecalculate(false))}
                data-testid="totals-recalc"
            >
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                {status === "sent" ? "Recalculate…" : "Recalculate"}
            </button>
        {/if}
    </section>
{/if}
