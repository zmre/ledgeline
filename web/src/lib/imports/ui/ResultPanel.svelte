<script lang="ts">
    // What was written, and the one thing an import can leave behind: a journal
    // that is no longer in date order.
    //
    // The re-sort is offered rather than done. It moves transactions inside a
    // file the user owns, so it is confirmed against a list of exactly which
    // ones would move and where — and it is Ledgeline's own sort, not
    // `hledger print`, which flattens `include` directives into one file and
    // drops every `account`, `commodity`, `P` and standalone comment on the way
    // through. Our sort moves whole transactions between barriers and leaves
    // everything else byte-for-byte where it was.
    //
    // The sort itself is a PRESSED action with a one-line outcome, so it reports
    // inline rather than replacing this panel with a spinner. The commit it sits
    // inside is the async surface.
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {reorderOffer, writtenLines} from "../importModel";
    import type {CommitResult} from "../importTypes";

    let {
        view,
        result,
        error,
        sorting,
        sortMoved,
        sortError,
        onRetry,
        onSort,
    }: {
        view: import("$lib/stores/loadState").DataView;
        result: CommitResult | null;
        error: Error | null;
        sorting: boolean;
        /** How many transactions the confirmed re-sort moved, or null before one ran. */
        sortMoved: number | null;
        sortError: string | null;
        onRetry: () => void;
        onSort: () => void;
    } = $props();
</script>

<AsyncSection {view} value={result} {error} testid="imports-commit-error" label="the import" loadingLabel="Writing the import" {onRetry}>
    {#snippet children(commit)}
        <section class="border-success/40 rounded-box flex flex-col gap-3 border p-3" aria-label="Import result" data-testid="imports-result">
            <h2 class="text-sm font-semibold tracking-tight">Done</h2>
            <ul class="list-inside list-disc text-sm">
                {#each writtenLines(commit) as line (line)}
                    <li>{line}</li>
                {/each}
            </ul>

            {#if reorderOffer(commit) !== null && sortMoved === null}
                <!-- `flex` before `flex-col`: `.alert` is a grid with
                     `grid-auto-flow:column`, so without it the sentence, the
                     list of moved lines and the button become three thin
                     side-by-side columns. See `routes/alertStacking.test.ts`. -->
                <div class="alert alert-warning rounded-box flex flex-col items-start gap-2 py-2 text-sm" role="alert" data-testid="imports-out-of-order">
                    <span>{reorderOffer(commit)}</span>
                    <ul class="max-h-48 overflow-auto font-mono text-xs">
                        {#each commit.ordering.moves as move (`${move.fromLine}-${move.toLine}`)}
                            <li>{move.date} {move.description} — line {move.fromLine} → {move.toLine}</li>
                        {/each}
                    </ul>
                    <button type="button" class="btn btn-sm" disabled={sorting} onclick={onSort} data-testid="imports-sort">
                        {#if sorting}<span class="loading loading-spinner loading-xs"></span>{/if}
                        Re-sort by date
                    </button>
                </div>
            {/if}

            {#if sortMoved !== null}
                <p class="text-success text-sm" data-testid="imports-sorted">
                    Re-sorted: {sortMoved} transaction{sortMoved === 1 ? "" : "s"} moved. Directives, includes and comments are untouched.
                </p>
            {/if}
            {#if sortError !== null}
                <p class="text-error text-sm" role="alert" data-testid="imports-sort-error">{sortError}</p>
            {/if}
        </section>
    {/snippet}
</AsyncSection>
