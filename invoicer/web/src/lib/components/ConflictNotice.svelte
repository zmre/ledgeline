<!-- The invoice changed on disk while it had unsaved edits here (another
     editor, git, a sync). Nothing is overwritten on either side until the
     user picks: their file (discarding the edits here), or these edits saved
     over it. Their text is one click away to compare. -->
<script lang="ts">
    let {
        theirs,
        busy = false,
        onReload,
        onKeepMine,
    }: {
        /** The file as it is now on disk. */
        theirs: string | null;
        busy?: boolean;
        onReload: () => void;
        onKeepMine: () => void;
    } = $props();
</script>

<div class="alert flex-col items-start alert-warning" role="alert" data-testid="conflict-notice">
    <div>
        <p class="font-medium">This invoice changed on disk while you were editing it.</p>
        <p class="text-sm">Your edits here aren't saved. Keep theirs and lose yours, or save yours over theirs.</p>
    </div>
    <div class="flex flex-wrap gap-2">
        <button type="button" class="btn btn-sm" disabled={busy} onclick={onReload} data-testid="conflict-reload">Reload theirs (discard mine)</button>
        <button type="button" class="btn btn-outline btn-sm" disabled={busy} onclick={onKeepMine} data-testid="conflict-keep"
            >Keep mine (save over theirs)</button
        >
    </div>
    {#if theirs !== null}
        <details class="w-full text-sm">
            <summary class="cursor-pointer">Their file</summary>
            <pre class="mt-1 max-h-64 overflow-auto rounded-box bg-base-100 p-2 text-xs">{theirs}</pre>
        </details>
    {/if}
</div>
