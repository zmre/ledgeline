<!-- xlsx export trigger (WP-07). exportXlsx lazy-loads exceljs on first click,
     so the button shows a spinner while the chunk loads + the file builds. -->
<script lang="ts">
    import {exportXlsx} from "$lib/export/xlsx";
    import type {PeriodReport, SectionedReport} from "$lib/reports/types";

    let {
        report,
        title,
        params,
        filename,
    }: {
        report: SectionedReport | PeriodReport;
        title: string;
        params: string;
        filename: string;
    } = $props();

    let busy = $state(false);
    let error = $state<string | null>(null);

    async function onExport(): Promise<void> {
        busy = true;
        error = null;
        try {
            await exportXlsx(report, {title, params}, filename);
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
