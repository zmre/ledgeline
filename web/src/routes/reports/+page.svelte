<script lang="ts">
    // Reports route (WP-07, now native): tabbed bs/is/cf/nw tables fetched from
    // the ledgeline-engine /api/reports/{tab} endpoints and decoded into the
    // existing domain types, rendered by the unchanged ReportTable. Every
    // control change refetches (keyed on the active tab's params); the last good
    // report stays visible across a refetch. Tab + controls still live in the
    // URL (parsed once on mount, mirrored back with debounced replaceState).
    // Display styles come from the journal wire feed (reportStyles), fetched in
    // parallel — the engine returns exact numbers, not commodity display styles.
    import {onMount} from "svelte";
    import {replaceState} from "$app/navigation";
    import {NativeApiUnavailableError} from "$lib/api/native";
    import {exportXlsx} from "$lib/export/xlsx";
    import ExportButton from "$lib/reports/ui/ExportButton.svelte";
    import ReportControls from "$lib/reports/ui/ReportControls.svelte";
    import ReportTable from "$lib/reports/ui/ReportTable.svelte";
    import ReportTabs from "$lib/reports/ui/ReportTabs.svelte";
    import {defaultReportParams, paramsToSearch, searchToParams, TAB_DEFAULTS, type ReportParams, type ReportTab} from "$lib/reports/ui/params";
    import {reportStyles} from "$lib/reports/ui/styles";
    import {buildReportQuery, reports} from "$lib/stores/reports.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    let params = $state<ReportParams>(defaultReportParams());
    let restored = $state(false);
    let activeTab: ReportTab = defaultReportParams().tab;

    // Restore params from the URL exactly once, at startup.
    onMount(() => {
        if (window.location.search !== "") Object.assign(params, searchToParams(window.location.search, defaultReportParams()));
        activeTab = params.tab; // the restored/initial tab keeps its (URL or default) interval/count
        restored = true;
        return () => {
            if (timer !== null) clearTimeout(timer);
        };
    });

    // Each tab seeds its own interval/count on activation (cash flow wants
    // monthly/12, net worth yearly/5; bs/is ignore these). Depth stays shared.
    $effect(() => {
        const tab = params.tab;
        if (!restored || tab === activeTab) return;
        activeTab = tab;
        const d = TAB_DEFAULTS[tab];
        params.interval = d.interval;
        params.count = d.count;
    });

    // Mirror params → URL, debounced, replaceState (no history entries, no loops).
    let timer: ReturnType<typeof setTimeout> | null = null;
    $effect(() => {
        const search = paramsToSearch(params);
        if (!restored) return;
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
            timer = null;
            if (window.location.search.replace(/^\?/, "") === search) return;
            const url = `${window.location.pathname}?${search}`;
            try {
                // eslint-disable-next-line svelte/no-navigation-without-resolve -- URL is the CURRENT pathname (from window.location), not a route id to resolve
                replaceState(url, {});
            } catch {
                // Router not initialized (tests, embedding) — degrade to the raw History API.
                history.replaceState(history.state, "", url);
            }
        }, 250);
    });

    // Load the journal once a server URL is configured (styles + max depth only — the report itself is native).
    let attemptedUrl: string | null = null;
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== null && url !== attemptedUrl) {
            attemptedUrl = url;
            void journal.refresh();
        }
    });

    // Fetch the native report whenever the active tab's query changes (or the server is first configured).
    const reportQuery = $derived(buildReportQuery(params));
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== null) void reports.load(url, reportQuery);
    });

    const styles = $derived(reportStyles(journal.txns));
    const stylesReady = $derived(journal.txns.length > 0);
    const maxDepth = $derived(journal.accountNames.reduce((max, name) => Math.max(max, name.split(":").length), 1));

    const report = $derived(reports.report);
    const nativeUnavailable = $derived(reports.error instanceof NativeApiUnavailableError);

    /** Commodities the valuation had to skip (net worth) — surfaced as a warning badge. */
    const unpriced = $derived.by(() => (report !== null && !("sections" in report) ? (report.meta?.unpriced ?? []) : []));

    const exportInfo = $derived.by(() => {
        const span = `last ${params.count} ${params.interval} periods ending ${params.end}`;
        switch (params.tab) {
            case "bs":
                return {title: "Balance Sheet", params: `as of ${params.asOf}, depth ${params.depth}`, filename: `balance-sheet-${params.asOf}.xlsx`};
            case "is":
                return {
                    title: "Income Statement",
                    params: `${params.from} to ${params.to}, depth ${params.depth}`,
                    filename: `income-statement-${params.from}-to-${params.to}.xlsx`,
                };
            case "cf":
                return {title: "Cash Flow", params: `${span}, depth ${params.depth}`, filename: `cash-flow-${params.end}.xlsx`};
            case "nw":
                return {title: "Net Worth", params: `${span}, depth ${params.depth}`, filename: `net-worth-${params.end}.xlsx`};
        }
    });
</script>

<svelte:head><title>Ledgeline — Reports</title></svelte:head>

<div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <ReportTabs bind:tab={params.tab} />
        {#if report !== null}
            {@const current = report}
            <ExportButton run={() => exportXlsx(current, {title: exportInfo.title, params: exportInfo.params}, exportInfo.filename)} />
        {/if}
    </div>

    <ReportControls bind:params {maxDepth} />

    {#if unpriced.length > 0}
        <div class="alert alert-warning rounded-box px-3 py-2 text-sm" role="alert" data-testid="unpriced-warning">
            <span>Some holdings are not valued — no market price for: {unpriced.join(", ")}</span>
        </div>
    {/if}

    {#if report !== null && stylesReady}
        <ReportTable {report} {styles} />
    {:else if report === null && reports.status === "error"}
        <div class="alert alert-error rounded-box flex-col items-start gap-2 px-3 py-3 text-sm" role="alert" data-testid="reports-error">
            <span>{nativeUnavailable ? reports.error?.message : `Couldn't load the report: ${reports.error?.message ?? "unknown error"}`}</span>
            {#if !nativeUnavailable}
                <button type="button" class="btn btn-sm" onclick={() => void reports.load(settings.serverUrl ?? "", reportQuery)}>Retry</button>
            {/if}
        </div>
    {:else}
        <div class="flex items-center justify-center py-24" aria-label="Loading reports">
            <span class="loading loading-spinner loading-lg"></span>
        </div>
    {/if}
</div>

{#if journal.status === "error" && journal.error !== null}
    <div class="toast toast-end z-30">
        <div class="alert alert-error">
            <span class="max-w-xs truncate" title={journal.error}>{journal.error}</span>
            <button type="button" class="btn btn-sm" onclick={() => void journal.refresh()}>Retry</button>
        </div>
    </div>
{/if}
