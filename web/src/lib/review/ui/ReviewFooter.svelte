<script lang="ts">
    // The card's footer (plan 32 D7): four states, one row of fixed height so
    // swapping between them never moves the card.
    //
    //   clean          ✓ Reviewed (success)             Skip →   (no key hint: it would read "→ →")
    //   clean-unknown  ✓ Reviewed, disabled (why)       Skip →
    //   dirty          Mark Reviewed and Save → (success)  ✕ Revert Changes
    //   dirty-unknown  Save → (primary, not reviewed)    ✕ Revert Changes
    //
    // Every button shows its key.
    import {formatKeys} from "#lib/keys/chord.js";
    import type {FooterState} from "../card";
    import {footerFade} from "./motion";
    import ReviewIcon from "./ReviewIcon.svelte";

    let {
        phase,
        reduced,
        busy = false,
        onPrimary,
        onSkip,
        onRevert,
    }: {phase: FooterState; reduced: boolean; busy?: boolean; onPrimary: () => void; onSkip: () => void; onRevert: () => void} = $props();

    const saveKeys = formatKeys("mod+Enter")
        .map((token) => token.text)
        .join("");
    const dirty = $derived(phase === "dirty" || phase === "dirty-unknown");
</script>

<footer
    class="flex min-h-[4.5rem] shrink-0 items-center justify-between gap-3 border-t border-base-content/10 px-5 py-3 sm:px-8"
    data-testid="review-footer"
    data-state={phase}
>
    {#key phase}
        <div class="flex w-full flex-wrap items-center justify-between gap-3" in:footerFade={{reduced}}>
            {#if phase === "clean"}
                <button type="button" class="btn gap-2 btn-success" onclick={onPrimary} disabled={busy} data-testid="review-primary">
                    <ReviewIcon name="check" /> Reviewed
                    <kbd class="kbd border-success-content/30 bg-success-content/10 kbd-xs text-success-content">{saveKeys}</kbd>
                </button>
            {:else if phase === "clean-unknown"}
                <span class="tooltip tooltip-top" data-tip="Categorise the unknown posting first — or Skip it for now">
                    <button type="button" class="btn gap-2 btn-success" disabled data-testid="review-primary">
                        <ReviewIcon name="check" /> Reviewed <kbd class="kbd kbd-xs">{saveKeys}</kbd>
                    </button>
                </span>
            {:else if phase === "dirty"}
                <button type="button" class="btn gap-2 btn-success" onclick={onPrimary} disabled={busy} data-testid="review-primary">
                    Mark Reviewed and Save <ReviewIcon name="arrow-right" />
                    <kbd class="kbd border-success-content/30 bg-success-content/10 kbd-xs text-success-content">{saveKeys}</kbd>
                </button>
            {:else}
                <span class="tooltip tooltip-top" data-tip="Saves your changes without marking it reviewed: an account is still unknown">
                    <button type="button" class="btn gap-2 btn-primary" onclick={onPrimary} disabled={busy} data-testid="review-primary">
                        Save <ReviewIcon name="arrow-right" />
                        <kbd class="kbd border-primary-content/30 bg-primary-content/10 kbd-xs text-primary-content">{saveKeys}</kbd>
                    </button>
                </span>
            {/if}

            {#if dirty}
                <button type="button" class="btn gap-2 btn-ghost" onclick={onRevert} data-testid="review-revert">
                    <ReviewIcon name="x" /> Revert Changes <kbd class="kbd kbd-xs">z</kbd>
                </button>
            {:else}
                <button type="button" class="btn gap-2 btn-ghost" onclick={onSkip} data-testid="review-skip">
                    Skip <ReviewIcon name="arrow-right" />
                </button>
            {/if}
        </div>
    {/key}
</footer>
