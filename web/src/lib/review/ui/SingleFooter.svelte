<script lang="ts">
    // The card's footer in single mode (plan 32 D31, D32): no queue, so no
    // Reviewed-and-next and no Skip.
    //
    //   [ Save  ⌘-Enter ]   ☐ Mark reviewed  v        [ ✕ Revert Changes  z ]
    //
    // The checkbox shows the state the save will write, not an action: checked
    // when the form carries a `reviewed:` tag ("Ignored" for `reviewed:ignore`).
    // It is disabled while a posting is unknown, as the queue card's Reviewed
    // button is. Save waits for a change; on an add card it reads "Add".
    // Revert shows once there is something to revert.
    import {formatKeys} from "#lib/keys/chord.js";
    import ReviewIcon from "./ReviewIcon.svelte";

    let {
        adding,
        dirty,
        busy = false,
        reviewed,
        reviewedLabel,
        unknown,
        onSave,
        onToggleReviewed,
        onRevert,
    }: {
        /** An add card (D36): the button reads "Add". */
        adding: boolean;
        dirty: boolean;
        busy?: boolean;
        reviewed: boolean;
        reviewedLabel: string;
        /** A posting is still unknown: the checkbox can't be changed. */
        unknown: boolean;
        onSave: () => void;
        onToggleReviewed: () => void;
        onRevert: () => void;
    } = $props();

    const saveKeys = formatKeys("mod+Enter")
        .map((token) => token.text)
        .join("");
</script>

<footer
    class="flex min-h-[4.5rem] shrink-0 flex-wrap items-center justify-between gap-3 border-t border-base-content/10 px-5 py-3 sm:px-8"
    data-testid="review-footer"
    data-state="single"
>
    <div class="flex flex-wrap items-center gap-x-5 gap-y-2">
        <button type="button" class="btn gap-2 btn-primary" onclick={onSave} disabled={busy || !dirty} data-testid="review-primary">
            {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
            {adding ? "Add" : "Save"}
            <kbd class="kbd border-primary-content/30 bg-primary-content/10 kbd-xs text-primary-content">{saveKeys}</kbd>
        </button>
        <span class={["flex", unknown && "tooltip tooltip-top"]} data-tip={unknown ? "Categorise the unknown posting first" : undefined}>
            <label class={["flex items-center gap-2 text-sm", unknown ? "cursor-not-allowed text-base-content/45" : "cursor-pointer"]}>
                <input
                    type="checkbox"
                    class="checkbox checkbox-sm checkbox-success"
                    checked={reviewed}
                    disabled={unknown || busy}
                    onchange={onToggleReviewed}
                    data-testid="review-mark-reviewed"
                />
                {reviewedLabel}
                <kbd class="kbd kbd-xs">v</kbd>
            </label>
        </span>
    </div>

    {#if dirty}
        <button type="button" class="btn gap-2 btn-ghost" onclick={onRevert} disabled={busy} data-testid="review-revert">
            <ReviewIcon name="x" /> Revert Changes <kbd class="kbd kbd-xs">z</kbd>
        </button>
    {/if}
</footer>
