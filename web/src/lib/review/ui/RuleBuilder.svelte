<script lang="ts">
    // "Remember as a rule" (plan 32 D25, D27): turn the change on a review card
    // into an `if` rule at the end of a rules file, so the next statement
    // arrives already categorised.
    //
    // Off by default (Q9): rules get written only when asked for. The overlay
    // shows this on a card whose description or account changed (not a
    // split) — an import's CSV card, or a journal card the engine found a
    // rules file for — and binds `R` to the toggle.
    //
    //   Saving to [citi-2026.csv.rules ▾] newest of 2 for citi   (journal: `file`)
    //   Match     [description ▾] [ blue.*bottle.*oakland ]
    //   Then set  ☑ account2 → expenses:food:coffee
    //             ☐ description → Blue Bottle Coffee
    //   Matches 7 of 112 rows in checking.csv · 0 already categorised
    //
    // This is the toggle and the drawer; the rule itself is `RuleBody`, keyed
    // on `source` so a journal card that switches rules file starts the rule
    // again from that file. The `file` line (the journal's picker) sits above
    // it and is NOT remounted, so the select keeps focus as the pick changes.
    // A null `source` is a journal card whose file is still to be picked: the
    // body is disabled until it is. Nothing here writes: the overlay saves the
    // rule (through `source.save`) when the card is saved.
    import type {Snippet} from "svelte";
    import type {CardChange} from "#lib/api/editMapping.js";
    import type {Transaction} from "#lib/domain/types.js";
    import type {RuleSource, RuleSpec} from "#lib/imports/reviewTypes.js";
    import type {QueueItem} from "../types";
    import ReviewIcon from "./ReviewIcon.svelte";
    import RuleBody from "./RuleBody.svelte";

    let {
        source,
        item,
        original,
        change,
        on = $bindable(false),
        error = null,
        busy = false,
        file,
    }: {
        /** The rules file the rule goes to, or null while a journal card's is still to be picked. */
        source: RuleSource | null;
        /** The card: the source builds the prefill from it. */
        item: QueueItem;
        /** The card's transaction as it was (an import's: as generated), which `change` is measured from. */
        original: Transaction;
        change: CardChange;
        /** Remember as a rule. */
        on?: boolean;
        /** Why saving the rule failed, from the overlay. */
        error?: string | null;
        /** The rule is being saved. */
        busy?: boolean;
        /** Which rules file, above Match: a journal card's picker (D27). An import has one file and no line. */
        file?: Snippet;
    } = $props();

    let body = $state<{currentRule: () => RuleSpec | null} | null>(null);

    /** The rule as it stands, for the overlay to save. Null while it has no file, no pattern or nothing to set. */
    export function currentRule(): RuleSpec | null {
        return body?.currentRule() ?? null;
    }

    const hint = $derived.by(() => {
        if (source === null) return on ? "Choose the rules file to add it to" : "So the next import does this by itself";
        return on ? `Added to the end of ${source.rulesLabel} when you save this card` : `So the next import from ${source.fileLabel} does this by itself`;
    });
</script>

<section class="rounded-box border border-base-content/10 bg-base-200/40" data-testid="review-rule" data-on={on ? "" : undefined}>
    <label class="flex cursor-pointer items-center gap-3 px-3 py-2">
        <input type="checkbox" class="toggle toggle-sm toggle-success" bind:checked={on} disabled={busy} data-testid="review-rule-toggle" />
        <span class="flex min-w-0 grow flex-col">
            <span class="flex items-center gap-1.5 text-sm font-medium">
                <ReviewIcon name="bolt" class="h-3.5 w-3.5 text-base-content/50" />
                Remember as a rule
                <kbd class="kbd kbd-xs font-normal">⇧R</kbd>
            </span>
            <span class="truncate text-xs text-base-content/50">{hint}</span>
        </span>
    </label>

    <!-- 0fr → 1fr, like Similar: the builder opens without measuring anything. -->
    <div class={["grid transition-[grid-template-rows] duration-120 ease-out motion-reduce:transition-none", on ? "grid-rows-[1fr]" : "grid-rows-[0fr]"]}>
        <div class="min-h-0 overflow-hidden" inert={!on}>
            <div class="border-t border-base-content/10">
                {#if file !== undefined}
                    <div class="px-3 pt-3" data-testid="review-rule-file-line">{@render file()}</div>
                {/if}
                {#key source}
                    <RuleBody bind:this={body} {source} {item} {original} {change} {on} {error} {busy} />
                {/key}
            </div>
        </div>
    </div>
</section>
