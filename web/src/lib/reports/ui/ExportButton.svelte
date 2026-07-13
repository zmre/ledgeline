<!-- xlsx export trigger (WP-07). The caller's `run` does the actual export
     (the export helpers lazy-load exceljs on first click), so the button
     shows a spinner while the chunk loads + the file builds; failures land
     in a dismissible error toast. -->
<script lang="ts">
    let {run}: {run: () => Promise<void>} = $props();

    let busy = $state(false);
    let error = $state<string | null>(null);

    async function onExport(): Promise<void> {
        busy = true;
        error = null;
        try {
            await run();
        } catch (cause) {
            error = cause instanceof Error ? cause.message : String(cause);
        } finally {
            busy = false;
        }
    }
</script>

<button type="button" class="btn btn-primary btn-sm" onclick={() => void onExport()} disabled={busy} aria-label="Export report as xlsx">
    {#if busy}
        <span class="loading loading-spinner loading-xs"></span>
    {/if}
    Export .xlsx
</button>

{#if error !== null}
    <div class="toast toast-end z-30">
        <div class="alert alert-error">
            <span class="max-w-xs truncate" title={error}>Export failed: {error}</span>
            <button type="button" class="btn btn-sm" onclick={() => (error = null)}>Dismiss</button>
        </div>
    </div>
{/if}
