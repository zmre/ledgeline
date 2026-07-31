<script lang="ts">
    // An item Ledgeline carries but will not rewrite: a comment run, an
    // `include`, a `source`, or a conditional construct the engine declined to
    // classify.
    //
    // It is SHOWN, and it is MOVABLE, and it has no edit control at all. Hiding
    // it would be worse than useless — the user's file would appear to contain
    // less than it does, and a reorder would move rules past something invisible
    // — while an edit control over a construct whose parts depend on each other
    // is how an `if` table silently re-points every one of its rows.
    //
    // The raw text is the file's own bytes for that item, which is also exactly
    // what gets written back: this card's item is echoed to the engine as
    // `{kind:"keep", id}`, so what is on screen is what is on disk.
    import type {ItemSummary} from "../model";

    let {
        summary,
        position,
        total,
        disabled,
        onMoveUp,
        onMoveDown,
    }: {
        summary: ItemSummary;
        position: number;
        total: number;
        disabled: boolean;
        onMoveUp: () => void;
        onMoveDown: () => void;
    } = $props();
</script>

<div class="card bg-base-200/50 border-base-content/10 border border-dashed opacity-80" data-testid="imports-locked-item">
    <div class="card-body gap-2 p-3">
        <div class="flex items-center gap-2">
            <div class="join">
                <button
                    type="button"
                    class="btn btn-xs join-item"
                    disabled={disabled || position === 1}
                    onclick={onMoveUp}
                    aria-label="Move item {position} up"
                >
                    ↑
                </button>
                <button
                    type="button"
                    class="btn btn-xs join-item"
                    disabled={disabled || position === total}
                    onclick={onMoveDown}
                    aria-label="Move item {position} down"
                >
                    ↓
                </button>
            </div>
            <span class="text-base-content/60 truncate font-mono text-xs">{summary.title}</span>
            {#if summary.advanced}
                <span class="badge badge-ghost badge-sm gap-1 whitespace-nowrap" data-testid="imports-locked-badge">🔒 advanced — edit in terminal</span>
            {:else}
                <span class="badge badge-ghost badge-sm whitespace-nowrap">kept as-is</span>
            {/if}
        </div>
        {#if summary.detail !== ""}
            <p class="text-base-content/60 text-xs">{summary.detail}</p>
        {/if}
        {#if summary.text !== ""}
            <pre class="bg-base-300/40 max-h-40 overflow-auto rounded p-2 text-xs"><code>{summary.text.replace(/\n+$/, "")}</code></pre>
        {/if}
        {#if summary.truncated}
            <p class="text-base-content/50 text-xs">Only the first part is shown; the whole item is kept.</p>
        {/if}
    </div>
</div>
