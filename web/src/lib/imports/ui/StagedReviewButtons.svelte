<script lang="ts">
    // "Review before writing" (plan 32 D26, D30): Review Unknowns and Review All
    // over the transactions an import would write, above its write button.
    //
    // Optional, never a gate: the strip says so, and the write button beside it
    // is not touched by anything here. Both buttons carry their count, are
    // disabled at zero, and wear the Journal toolbar's review style
    // (`REVIEW_BUTTON`), so the two places a review starts look alike.
    //
    // `u` and `r` (D17) are registered for exactly as long as this strip is
    // mounted — the hosts mount it only once a dry run (or a QuickBooks preview
    // with every account mapped) is on screen — and each key is disabled exactly
    // when its button is.
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {hint} from "#lib/keys/hint.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {REVIEW_BUTTON} from "#lib/review/ui/buttons.js";
    import type {StagedRunKind} from "../stagedReview";
    import {reveal, revealArm} from "#lib/ui/reveal.js";

    let {
        unknowns,
        all,
        writeLabel = "Write changes",
        onReview,
    }: {
        /** Unknown transactions a held override has not already recategorised. */
        unknowns: number;
        /** Every transaction the import would write. */
        all: number;
        /** The host's write button, which is what applies the edits ("Import" for QuickBooks). */
        writeLabel?: string;
        onReview: (kind: StagedRunKind) => void;
    } = $props();

    /** The host flow's reveal arm (`NewTransactionsPanel`); null when mounted alone. */
    const arm = revealArm();

    function launch(kind: StagedRunKind): void {
        if ((kind === "unknowns" ? unknowns : all) > 0) onReview(kind);
    }

    registerKeys({
        id: "staged-review",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "u", label: "Review this import's unknowns", group: "Imports", enabled: () => unknowns > 0, run: () => launch("unknowns")},
            {keys: "r", label: "Review all of this import", group: "Imports", enabled: () => all > 0, run: () => launch("all")},
        ],
    });
</script>

<div
    class="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-base-content/10 bg-base-200/40 px-3 py-2.5"
    use:reveal={{arm}}
    data-testid="imports-review"
>
    <div class="flex items-center gap-2">
        <span class="tooltip tooltip-bottom" data-tip={unknowns === 0 ? "Every transaction here has a category" : hint("Review Unknowns", "u")}>
            <button
                type="button"
                class="{REVIEW_BUTTON} gap-1.5"
                disabled={unknowns === 0}
                onclick={() => launch("unknowns")}
                data-testid="imports-review-unknowns"
            >
                Review Unknowns
                <span class={["badge badge-sm tabular-nums", unknowns > 0 ? "badge-warning" : "badge-ghost"]} data-testid="imports-review-unknowns-count"
                    >{unknowns}</span
                >
            </button>
        </span>
        <span class="tooltip tooltip-bottom" data-tip={all === 0 ? "There is nothing to import" : hint("Review All", "r")}>
            <button type="button" class="{REVIEW_BUTTON} gap-1.5" disabled={all === 0} onclick={() => launch("all")} data-testid="imports-review-all">
                Review All
                <span class="badge badge-ghost badge-sm tabular-nums" data-testid="imports-review-all-count">{all}</span>
            </button>
        </span>
    </div>
    <p class="min-w-0 grow text-xs leading-relaxed text-base-content/60">
        Optional. Categorise, rename and mark transactions reviewed before they're written — nothing is saved until you press <strong
            class="font-medium text-base-content/80">{writeLabel}</strong
        >.
    </p>
</div>
