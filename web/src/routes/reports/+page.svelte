<script lang="ts">
    // Reports route (WP-07): tabbed bs/is/cf/nw tables computed from the journal
    // store by the pure WP-06 engine — every control change recomputes via
    // $derived, no refetch. Tab + controls live in the URL (parsed once on
    // mount, then mirrored back with debounced replaceState — same pattern as
    // filters/urlSync.ts). All dates are INCLUSIVE (engine semantics).
    import {onMount} from "svelte";
    import {replaceState} from "$app/navigation";
    import {exportXlsx} from "$lib/export/xlsx";
    import {balanceSheet} from "$lib/reports/balanceSheet";
    import {cashFlow} from "$lib/reports/cashFlow";
    import {incomeStatement} from "$lib/reports/incomeStatement";
    import {netWorth} from "$lib/reports/netWorth";
    import {buildPriceDb} from "$lib/reports/prices";
    import ExportButton from "$lib/reports/ui/ExportButton.svelte";
    import ReportControls from "$lib/reports/ui/ReportControls.svelte";
    import ReportTable from "$lib/reports/ui/ReportTable.svelte";
    import ReportTabs from "$lib/reports/ui/ReportTabs.svelte";
    import {defaultReportParams, paramsToSearch, searchToParams, type ReportParams} from "$lib/reports/ui/params";
    import {reportStyles} from "$lib/reports/ui/styles";
    import {journal} from "$lib/stores/journal.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    let params = $state<ReportParams>(defaultReportParams());
    let restored = $state(false);

    // Restore params from the URL exactly once, at startup.
    onMount(() => {
        if (window.location.search !== "") Object.assign(params, searchToParams(window.location.search, defaultReportParams()));
        restored = true;
        return () => {
            if (timer !== null) clearTimeout(timer);
        };
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

    // Load the journal once a server URL is configured (same pattern as the journal route).
    let attemptedUrl: string | null = null;
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== null && url !== attemptedUrl) {
            attemptedUrl = url;
            void journal.refresh();
        }
    });

    const styles = $derived(reportStyles(journal.txns));
    const priceDb = $derived(buildPriceDb(journal.prices));
    const maxDepth = $derived(journal.accountNames.reduce((max, name) => Math.max(max, name.split(":").length), 1));

    const report = $derived.by(() => {
        switch (params.tab) {
            case "bs":
                return balanceSheet(journal.txns, {asOf: params.asOf, depth: params.depth});
            case "is":
                return incomeStatement(journal.txns, {from: params.from, to: params.to, depth: params.depth});
            case "cf":
                return cashFlow(journal.txns, {end: params.end, interval: params.interval, count: params.count, depth: params.depth});
            case "nw":
                return netWorth(journal.txns, priceDb, {end: params.end, interval: params.interval, count: params.count});
        }
    });

    /** Commodities the valuation had to skip (net worth) — surfaced as a warning badge. */
    const unpriced = $derived("sections" in report ? [] : (report.meta?.unpriced ?? []));

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
                return {title: "Net Worth", params: span, filename: `net-worth-${params.end}.xlsx`};
        }
    });
</script>

<svelte:head><title>Ledgeline — Reports</title></svelte:head>

<div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <ReportTabs bind:tab={params.tab} />
        <ExportButton run={() => exportXlsx(report, {title: exportInfo.title, params: exportInfo.params}, exportInfo.filename)} />
    </div>

    <ReportControls bind:params {maxDepth} />

    {#if unpriced.length > 0}
        <div class="alert alert-warning rounded-box px-3 py-2 text-sm" role="alert" data-testid="unpriced-warning">
            <span>Some holdings are not valued — no market price for: {unpriced.join(", ")}</span>
        </div>
    {/if}

    {#if journal.status === "loading" && journal.txns.length === 0}
        <div class="flex items-center justify-center py-24" aria-label="Loading reports">
            <span class="loading loading-spinner loading-lg"></span>
        </div>
    {:else}
        <ReportTable {report} {styles} />
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
