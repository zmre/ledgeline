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
    import {declaredTypes} from "$lib/domain/accountTypes";
    import {exportBudgetXlsx, exportXlsx} from "$lib/export/xlsx";
    import InsightsDashboard from "$lib/reports/ui/insights/InsightsDashboard.svelte";
    import BudgetSummary from "$lib/reports/ui/BudgetSummary.svelte";
    import ExportButton from "$lib/reports/ui/ExportButton.svelte";
    import ReportControls from "$lib/reports/ui/ReportControls.svelte";
    import ReportTable from "$lib/reports/ui/ReportTable.svelte";
    import ReportTabs from "$lib/reports/ui/ReportTabs.svelte";
    import SubscriptionsPanel from "$lib/reports/ui/subscriptions/SubscriptionsPanel.svelte";
    import {
        budgetPresetRange,
        defaultReportParams,
        DEFAULT_BUDGET_PRESET,
        paramsToSearch,
        searchToParams,
        TAB_DEFAULTS,
        TAB_LABELS,
        type ReportParams,
        type ReportTab,
    } from "$lib/reports/ui/params";
    import {reportStyles} from "$lib/reports/ui/styles";
    import {dataView} from "$lib/stores/loadState";
    import {budgetSpan, buildReportQuery, reports, sameReportQuery, type AnyReport} from "$lib/stores/reports.svelte";
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

    // Each tab seeds its own defaults on activation (cash flow wants monthly/12,
    // net worth yearly/5, budget year-to-date; bs/is ignore interval/count).
    $effect(() => {
        const tab = params.tab;
        if (!restored || tab === activeTab) return;
        activeTab = tab;
        const d = TAB_DEFAULTS[tab];
        params.interval = d.interval;
        params.count = d.count;
        if (tab === "budget") {
            const range = budgetPresetRange(DEFAULT_BUDGET_PRESET);
            params.from = range.from;
            params.to = range.to;
        }
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

    // Tabs that own their data + controls: they load from their own stores, so the
    // shared reports store, the controls bar, and the export button all sit out.
    const SELF_HOSTED: ReportTab[] = ["insights", "subs"];
    const selfHosted = $derived(SELF_HOSTED.includes(params.tab));

    // Fetch the native report whenever the active tab's query changes, the server
    // is first configured, or the user reconnects. The nonce is in the key
    // because a reconnect usually leaves the URL identical (the engine restarted
    // on the same port), and keying on the URL alone meant this page never
    // retried after one (FE-5d).
    const reportQuery = $derived(buildReportQuery(params));
    const loadKey = $derived({url: settings.serverUrl, nonce: settings.serverNonce, query: reportQuery});
    $effect(() => {
        const {url, query} = loadKey;
        if (url !== null && !selfHosted) void reports.load(url, query);
    });

    // Display styles are a nicety, never a gate: they come from the SEPARATE
    // hledger-web feed, so requiring them (`journal.txns.length > 0`) spun
    // forever whenever the engine answered and that feed did not — and on a
    // legitimately empty journal, for every new user (FE-5c). `formatTotals`
    // already falls back per commodity when the map has no entry.
    const styles = $derived(reportStyles(journal.txns));
    const declared = $derived(declaredTypes(journal.accountDecls));
    const maxDepth = $derived(journal.accountNames.reduce((max, name) => Math.max(max, name.split(":").length), 1));

    const report = $derived(reports.report);
    const nativeUnavailable = $derived(reports.error instanceof NativeApiUnavailableError);

    // `bs`/`is` are both SectionedReport and `cf`/`nw` are both PeriodReport, so
    // shape alone cannot say which tab a report belongs to — which is how a
    // balance sheet came to be rendered, and exported, under the P&L's label
    // (FE-1). The store tags each report with the query it came from; nothing is
    // shown unless that tag names the tab now being viewed.
    const loadedTab = $derived(reports.query?.tab ?? null);
    const view = $derived(dataView(reports.status, report !== null, loadedTab === params.tab));
    const shown = $derived(view === "data" ? report : null);

    // Discriminate the report shape: budget (kind:"budget") → BudgetSummary; bs/is/cf/nw → ReportTable.
    const budgetReport = $derived(shown !== null && "kind" in shown ? shown : null);
    const tableReport = $derived(shown !== null && !("kind" in shown) ? shown : null);

    /**
     * The report the export button may act on: the one being shown AND answering
     * exactly the query the controls currently describe. `exportInfo` builds the
     * filename and the title from `params`, so anything less lets the user
     * download `income-statement-….xlsx`, titled "Income Statement", holding
     * balance-sheet figures.
     */
    const exportable = $derived.by(() => {
        const held = reports.query;
        return shown !== null && held !== null && sameReportQuery(held, reportQuery) ? shown : null;
    });

    /** Commodities the valuation had to skip (net worth) — surfaced as a warning badge. */
    const unpriced = $derived.by(() => (tableReport !== null && !("sections" in tableReport) ? (tableReport.meta?.unpriced ?? []) : []));

    const exportInfo = $derived.by(() => {
        const span = `last ${params.count} ${params.interval} periods ending ${params.end}`;
        switch (params.tab) {
            case "insights":
            case "subs":
                // Neither has an XLSX export yet; the button is hidden for them.
                return {title: TAB_LABELS[params.tab], params: "", filename: "report.xlsx"};
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
            case "budget":
                return {
                    title: "Budget",
                    params: `${params.from} to ${params.to}, depth ${params.depth}`,
                    filename: `budget-${params.from}-to-${params.to}.xlsx`,
                };
        }
    });

    /** Export the current report — budget uses its own workbook builder; the rest share exportXlsx. */
    function runExport(current: AnyReport): Promise<void> {
        const meta = {title: exportInfo.title, params: exportInfo.params};
        return "kind" in current ? exportBudgetXlsx(current, meta, exportInfo.filename, declared) : exportXlsx(current, meta, exportInfo.filename);
    }
</script>

<svelte:head><title>Ledgeline — Reports</title></svelte:head>

<div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <ReportTabs bind:tab={params.tab} />
        {#if exportable !== null && !selfHosted}
            {@const current = exportable}
            <ExportButton run={() => runExport(current)} />
        {/if}
    </div>

    {#if !selfHosted}
        <ReportControls bind:params {maxDepth} />
    {/if}

    {#if unpriced.length > 0}
        <div class="alert alert-warning rounded-box px-3 py-2 text-sm" role="alert" data-testid="unpriced-warning">
            <span>Some holdings are not valued — no market price for: {unpriced.join(", ")}</span>
        </div>
    {/if}

    <!-- The error branch comes BEFORE the data branches and asks only about
         `status`. Tested after them, and additionally gated on `report === null`,
         it could never fire once any report had loaded — so a failed refetch just
         kept serving the previous answer under the new controls' label (FE-5). -->
    {#if params.tab === "insights"}
        <InsightsDashboard bind:params serverUrl={settings.serverUrl} {styles} />
    {:else if params.tab === "subs"}
        <SubscriptionsPanel serverUrl={settings.serverUrl} {styles} />
    {:else if view === "error"}
        <div class="alert alert-error rounded-box flex-col items-start gap-2 px-3 py-3 text-sm" role="alert" data-testid="reports-error">
            <span>{nativeUnavailable ? reports.error?.message : `Couldn't load the report: ${reports.error?.message ?? "unknown error"}`}</span>
            {#if !nativeUnavailable}
                <button type="button" class="btn btn-sm" onclick={() => void reports.load(settings.serverUrl ?? "", reportQuery)}>Retry</button>
            {/if}
        </div>
    {:else if budgetReport !== null}
        <!-- budgetSpan, not params: the bars cover whole months, so the journal
             link has to cover the same span or the rows won't add up to the bar. -->
        <BudgetSummary report={budgetReport} {styles} {declared} from={budgetSpan(params.from, params.to).from} to={budgetSpan(params.from, params.to).to} />
    {:else if tableReport !== null}
        <ReportTable report={tableReport} {styles} />
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
            <button type="button" class="btn btn-sm" onclick={() => void journal.refresh({force: true})}>Retry</button>
        </div>
    </div>
{/if}
