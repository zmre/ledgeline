<!-- xlsx export trigger (WP-07). The caller's `run` does the actual export
     (the export helpers lazy-load exceljs on first click), so the button
     shows a spinner while the chunk loads + the file builds; failures land
     in a dismissible error toast.

     In the iOS app (plan 42) it is also the page's "Export to Excel" menu
     action, so every screen with an export offers it in the native menu with
     no code of its own; the file goes to the share sheet (lib/export/xlsx.ts). -->
<script lang="ts">
    import {registerNativeActions} from "#lib/native/bridge.svelte.js";

    let {run, label = "Export report as xlsx"}: {run: () => Promise<void>; label?: string} = $props();

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

    registerNativeActions(() => [{id: "export", label: "Export to Excel", icon: "square.and.arrow.up", run: () => (busy ? undefined : void onExport())}], {
        order: 20,
    });
</script>

<button type="button" class="btn btn-primary btn-sm" onclick={() => void onExport()} disabled={busy} aria-label={label}>
    {#if busy}
        <span class="loading loading-xs loading-spinner"></span>
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
