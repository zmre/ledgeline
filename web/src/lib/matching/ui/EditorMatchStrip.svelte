<script lang="ts">
    // The transaction editor's transfer line (plan 35 Amendment A2): a
    // transaction with an unlinked clearing posting offers ⇄ Find match,
    // which opens Find Matches focused on it; a linked one shows each link
    // with Unlink (asks first, then names every member of the link).
    //
    // Both read the transaction as served — its postings' tags carry the
    // inherited `clearing:` and their own `match:` — so showing the line costs
    // no request. Both wait for a clean card: Find match replaces the editor
    // until the matcher closes (it comes back re-read), and Unlink re-opens it
    // on the transaction as rewritten.
    import type {Transaction} from "#lib/domain/types.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {matches} from "#lib/stores/matches.svelte.js";
    import {findableClearing, linkIdsOf} from "../model";

    let {
        txn,
        dirty,
        disabled = false,
        onUnlinked,
    }: {
        txn: Transaction;
        /** The card has unsaved edits: both actions wait for a save or a revert. */
        dirty: boolean;
        disabled?: boolean;
        /** The unlink landed: the editor re-reads the transaction. */
        onUnlinked: () => void;
    } = $props();

    const findable = $derived(findableClearing(txn).length > 0);
    const links = $derived(linkIdsOf(txn));
    const blocked = $derived(disabled || dirty || matches.busy);
    const why = $derived(dirty ? "Save or revert the changes first" : undefined);
    let armed = $state<string | null>(null);
    let failure = $state<string | null>(null);

    async function unlink(id: string): Promise<void> {
        if (blocked) return;
        if (armed !== id) {
            armed = id;
            return;
        }
        armed = null;
        failure = null;
        const outcome = await matches.unlinkFrom(txn.index, id);
        if (outcome.ok) onUnlinked();
        else if (outcome.failure.kind !== "conflict") failure = outcome.failure.message;
    }
</script>

{#if findable || links.length > 0}
    <div class="flex flex-col gap-2 text-sm" data-testid="review-match-strip">
        {#each links as id (id)}
            <div class="flex flex-wrap items-center gap-2" data-testid="review-match-link">
                <span class="inline-flex items-center gap-1.5 text-info"><ReviewIcon name="link" class="h-4 w-4" /> Linked transfer</span>
                <code class="text-xs text-base-content/55">match:{id}</code>
                {#if armed === id}
                    <span class="text-xs text-base-content/70">Remove this link from every posting carrying it? Both transactions stay.</span>
                    <button type="button" class="btn btn-warning btn-xs" disabled={blocked} onclick={() => void unlink(id)} data-testid="review-unlink-go"
                        >Unlink</button
                    >
                    <button type="button" class="btn btn-ghost btn-xs" onclick={() => (armed = null)}>Keep</button>
                {:else}
                    <button
                        type="button"
                        class="btn gap-1 btn-ghost btn-xs"
                        disabled={blocked}
                        title={why}
                        onclick={() => void unlink(id)}
                        data-testid="review-unlink"
                    >
                        <ReviewIcon name="unlink" class="h-3.5 w-3.5" /> Unlink
                    </button>
                {/if}
            </div>
        {/each}
        {#if findable}
            <div class="flex flex-wrap items-center gap-2">
                <button
                    type="button"
                    class="btn gap-1.5 btn-sm"
                    disabled={blocked}
                    title={why ?? "Find the other half of this transfer"}
                    onclick={() => matches.openFor(txn.index)}
                    data-testid="review-find-match"
                >
                    <ReviewIcon name="link" class="h-3.5 w-3.5" /> Find match
                </button>
                <span class="text-xs text-base-content/55">A transfer half here isn't linked to its other half yet.</span>
            </div>
        {/if}
        {#if failure !== null}
            <p class="text-xs text-error" role="alert" data-testid="review-match-failure">Not unlinked: {failure}</p>
        {/if}
    </div>
{/if}
