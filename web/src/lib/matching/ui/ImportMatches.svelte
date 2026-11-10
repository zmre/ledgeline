<script lang="ts">
    // Find Matches on an import's result screen (plan 35 D9 "Import"). When
    // the import's new transactions have a proposed partner, it offers the
    // same overlay the journal opens, scoped to the pairs with a new half
    // (`since`). A QuickBooks import inserts by date, possibly into several
    // files, so it has no such scope and offers the whole journal's pairs
    // (`since` null). Hidden when there is nothing to offer, and again once
    // the overlay has linked or rejected everything it showed.
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {matches} from "#lib/stores/matches.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";

    let {
        since,
        imported,
    }: {
        /** `JOURNAL_ID:COUNT` (see `importSince`), or null for the whole journal. */
        since: string | null;
        /** How many transactions the import wrote: nothing new, nothing to offer. */
        imported: number;
    } = $props();

    /** The pairs on offer, or null until read (or when the read failed: the offer is an aid). */
    let count = $state<number | null>(null);

    // Re-read with the journal (a link from the overlay moves the generation)
    // and the match settings.
    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        void settings.matchWindow;
        void settings.matchLookback;
        const scope = since;
        if (serverUrl === null || generation === null || imported <= 0) {
            count = null;
            return;
        }
        let live = true;
        const read = scope === null ? matches.sync(serverUrl, generation).then(() => matches.count) : matches.scopedCount(serverUrl, scope);
        read.then(
            (n) => {
                if (live) count = n;
            },
            () => {
                if (live) count = null;
            }
        );
        return () => {
            live = false;
        };
    });
</script>

{#if count !== null && count > 0}
    <div class="flex flex-wrap items-center gap-3 rounded-box border border-info/30 bg-info/10 px-3 py-2 text-sm" data-testid="imports-find-matches">
        <ReviewIcon name="link" class="h-4 w-4 shrink-0 text-info" />
        <span class="grow" data-testid="imports-find-matches-text">
            {#if since !== null}
                {count} transfer {count === 1 ? "pair has" : "pairs have"} a half in this import.
            {:else}
                {count} transfer {count === 1 ? "pair is" : "pairs are"} waiting to be linked in the journal.
            {/if}
        </span>
        <button type="button" class="btn btn-sm" onclick={() => matches.openRun(since)} data-testid="imports-find-matches-button">
            Find Matches <span class="badge badge-sm tabular-nums badge-info">{count}</span>
        </button>
    </div>
{/if}
