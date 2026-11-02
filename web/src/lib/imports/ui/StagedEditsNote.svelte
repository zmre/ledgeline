<script lang="ts">
    // What review has queued for the write (plan 32 D23, D24): "3 reviewed
    // edits · 2 existing transactions edited", beside the write button, so
    // what that button will do is never a surprise.
    //
    // A FLAGGED edit is one a replay kept although the import changed under it
    // in a way the edit does not explain — a rules file edited elsewhere, a
    // mapping added. It still rides on the write (D26: nothing blocks), so the
    // note says how many there are and lists them on request, each with what it
    // does and a Discard, plus "Discard all".
    import {describeOps, editCountLine} from "../stagedReview";
    import type {StagedOverride, StagedRef} from "../reviewTypes";
    import {refKey} from "../reviewModel";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {reveal, revealArm} from "#lib/ui/reveal.js";

    let {
        overrideCount,
        journalEditCount,
        flagged,
        busy = false,
        onDiscard,
    }: {
        overrideCount: number;
        journalEditCount: number;
        flagged: readonly StagedOverride[];
        /** A write or a re-run is under way: the discards wait for it. */
        busy?: boolean;
        onDiscard: (refs: readonly StagedRef[]) => void;
    } = $props();

    /** The host flow's reveal arm (`NewTransactionsPanel`); null when mounted alone. */
    const arm = revealArm();

    const line = $derived(editCountLine(overrideCount, journalEditCount));
    let open = $state(false);
</script>

{#if line !== null}
    <div class="flex flex-col gap-2" use:reveal={{arm}} data-testid="imports-review-edits">
        <p class="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-base-content/70">
            <ReviewIcon name="pencil" class="h-3.5 w-3.5 text-success" />
            <span data-testid="imports-review-edits-count">{line}</span>
            <span class="text-base-content/50">— applied by this write</span>
            {#if flagged.length > 0}
                <button
                    type="button"
                    class="link font-medium text-warning link-hover"
                    aria-expanded={open}
                    aria-controls="imports-review-flagged"
                    onclick={() => (open = !open)}
                    data-testid="imports-review-flagged-toggle"
                >
                    {flagged.length}
                    {flagged.length === 1 ? "needs" : "need"} another look
                </button>
            {/if}
        </p>
        {#if flagged.length > 0 && open}
            <div
                class="flex flex-col gap-2 rounded-box border border-warning/30 bg-warning/5 p-3"
                id="imports-review-flagged"
                use:reveal={{arm}}
                role="region"
                aria-label="Edits that need another look"
                data-testid="imports-review-flagged"
            >
                <p class="text-xs text-base-content/70">
                    The import changed under these after you edited them, so they may no longer do what you meant. They're still written unless you discard
                    them.
                </p>
                <ul class="flex flex-col divide-y divide-base-content/10">
                    {#each flagged as override (refKey(override.ref))}
                        <li class="flex items-start gap-3 py-1.5 text-sm" data-testid="imports-review-flagged-row">
                            <span class="w-20 shrink-0 text-xs text-base-content/55 tabular-nums">{override.original.txn.date}</span>
                            <span class="min-w-0 grow">
                                <span class="block truncate font-mono text-xs" title={override.original.txn.description}
                                    >{override.original.txn.description}</span
                                >
                                <span class="mt-0.5 flex flex-wrap gap-1">
                                    {#each describeOps(override.ops) as what, at (at)}
                                        <span class="badge badge-ghost badge-xs">{what}</span>
                                    {/each}
                                </span>
                            </span>
                            <button
                                type="button"
                                class="btn shrink-0 btn-ghost btn-xs"
                                disabled={busy}
                                onclick={() => onDiscard([override.ref])}
                                aria-label="Discard the edit to {override.original.txn.date} {override.original.txn.description}"
                            >
                                Discard
                            </button>
                        </li>
                    {/each}
                </ul>
                {#if flagged.length > 1}
                    <button
                        type="button"
                        class="btn self-end btn-outline btn-warning btn-xs"
                        disabled={busy}
                        onclick={() => onDiscard(flagged.map((override) => override.ref))}
                        data-testid="imports-review-discard-flagged"
                    >
                        Discard all {flagged.length}
                    </button>
                {/if}
            </div>
        {/if}
    </div>
{/if}
