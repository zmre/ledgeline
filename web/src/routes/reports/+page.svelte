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
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import ErrorToast from "$lib/components/ErrorToast.svelte";
    import {exportBalanceSheetXlsx, exportIncomeStatementXlsx, exportXlsx} from "$lib/export/xlsx";
    import InsightsDashboard from "$lib/reports/ui/insights/InsightsDashboard.svelte";
    import BalanceSheetView from "$lib/reports/ui/BalanceSheetView.svelte";
    import ExportButton from "$lib/reports/ui/ExportButton.svelte";
    import IncomeStatementView from "$lib/reports/ui/IncomeStatementView.svelte";
    import ReportControls from "$lib/reports/ui/ReportControls.svelte";
    import ReportTable from "$lib/reports/ui/ReportTable.svelte";
    import ReportTabs from "$lib/reports/ui/ReportTabs.svelte";
    import SubscriptionsPanel from "$lib/reports/ui/subscriptions/SubscriptionsPanel.svelte";
    import {
        budgetRedirect,
        defaultReportParams,
        paramsToSearch,
        searchToParams,
        TAB_DEFAULTS,
        TAB_LABELS,
        type ReportParams,
        type ReportTab,
    } from "$lib/reports/ui/params";
    import type {FlowsPanel} from "$lib/reports/ui/sankeyModel";
    import {reportStyles} from "$lib/reports/ui/styles";
    import {flows, loadFlowsWhenWatched} from "$lib/stores/flows.svelte";
    import {dataView} from "$lib/stores/loadState";
    import {buildReportQuery, reports, sameReportQuery, type AnyReport} from "$lib/stores/reports.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {loadJournalWhenReady} from "$lib/stores/serverWatch.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {searchMirror} from "$lib/url/searchSync";

    let params = $state<ReportParams>(defaultReportParams());
    let restored = $state(false);
    let activeTab: ReportTab = defaultReportParams().tab;

    // Restore params from the URL exactly once, at startup.
    onMount(() => {
        const search = window.location.search;
        // Budget used to be a tab here and is its own route now, so a bookmark
        // still naming it is forwarded rather than quietly landing on Insights.
        // `replaceState` because the old URL is not a place to go Back to.
        const forward = budgetRedirect(search);
        if (forward !== null) {
            // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/budget") IS the route id; the query string is appended
            void goto(`${resolve("/budget")}${forward}`, {replaceState: true});
            return;
        }
        if (search !== "") Object.assign(params, searchToParams(search, defaultReportParams()));
        activeTab = params.tab; // the restored/initial tab keeps its (URL or default) interval/count
        restored = true;
        return () => mirror.stop();
    });

    // Each tab seeds its own defaults on activation (cash flow wants monthly/12,
    // net worth yearly/5; bs/is ignore interval/count).
    $effect(() => {
        const tab = params.tab;
        if (!restored || tab === activeTab) return;
        activeTab = tab;
        const d = TAB_DEFAULTS[tab];
        params.interval = d.interval;
        params.count = d.count;
    });

    // Mirror params → URL, debounced, replaceState (no history entries, no loops).
    // Reading `params` before the `restored` guard is deliberate: the effect has
    // to depend on them even on the run where it declines to write.
    const mirror = searchMirror();
    $effect(() => {
        const search = paramsToSearch(params);
        if (!restored) return;
        mirror.write(search);
    });

    // Load the journal once a server URL is configured (styles + max depth only — the report itself is native).
    loadJournalWhenReady();

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
    const maxDepth = $derived(journal.accountNames.reduce((max, name) => Math.max(max, name.split(":").length), 1));

    const report = $derived(reports.report);

    // `bs`/`is` are both SectionedReport and `cf`/`nw` are both PeriodReport, so
    // shape alone cannot say which tab a report belongs to — which is how a
    // balance sheet came to be rendered, and exported, under the P&L's label
    // (FE-1). The store tags each report with the query it came from; nothing is
    // shown unless that tag names the tab now being viewed. This is the one
    // surface that needs `dataView`'s third argument, so it calls it directly
    // rather than taking the resource's own `view`.
    const loadedTab = $derived(reports.query?.tab ?? null);
    const view = $derived(dataView(reports.status, report !== null, loadedTab === params.tab));
    const shown = $derived(view === "data" ? report : null);

    /**
     * The two Sankey diagrams above the P&L's boxes.
     *
     * Fetched only on the `is` tab, and shown only alongside a statement
     * answering the SAME window: the same FE-1 discipline `exportable` applies
     * to the workbook, for the same reason. The graphs decompose the very
     * figures in the boxes below them, so a diagram drawn for June under a
     * statement reading December is not stale, it is wrong.
     */
    const flowsQuery = $derived({from: params.from, to: params.to});
    // The gate (a configured server, the P&L tab, and at least one panel
    // expanded) lives in `loadFlowsWhenWatched`. It reads the open flags and
    // the reconnect nonce itself, so expanding a panel after both were shut
    // fetches immediately and a reconnect retries.
    loadFlowsWhenWatched(() => ({tab: params.tab, query: flowsQuery}));
    const flowsPanel = $derived.by((): FlowsPanel | null => {
        if (params.tab !== "is") return null;
        const held = flows.query;
        const matches = held !== null && held.from === flowsQuery.from && held.to === flowsQuery.to;
        return {
            view: dataView(flows.status, flows.value !== null, matches),
            report: flows.value,
            error: flows.error,
            retry: () => void flows.load(settings.serverUrl ?? "", flowsQuery),
        };
    });

    // Discriminate the report shape by its decoder-applied `kind` tag:
    // "balanceSheet" → BalanceSheetView, "incomeStatement" →
    // IncomeStatementView, untagged (cf/nw) → ReportTable. The tag exists
    // because THREE report types carry a `sections` array (SectionedReport,
    // BalanceSheetReport, IncomeStatementReport), so shape alone cannot tell
    // them apart — the FE-1 mistake, two shapes further on.

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

    /**
     * Commodities the valuation had to skip — surfaced as a warning banner.
     *
     * `meta` is carried by every report that HAS a valuation step: net worth,
     * the balance sheet and now the income statement. This used to be gated on
     * "is a PeriodReport", so a balance sheet holding an unpriced GLD position
     * said nothing at all, which is the one thing a valued report must never do
     * about a commodity it silently left out of the total.
     *
     * `"meta" in shown` rather than a per-kind branch, deliberately: it is the
     * property that MAKES the claim, so a fourth valued report gets the banner by
     * existing rather than by someone remembering to add a case here.
     */
    const unpriced = $derived.by(() => (shown !== null && "meta" in shown ? (shown.meta?.unpriced ?? []) : []));

    const exportInfo = $derived.by(() => {
        const span = `last ${params.count} ${params.interval} periods ending ${params.end}`;
        switch (params.tab) {
            case "insights":
            case "subs":
                // Neither has an XLSX export yet; the button is hidden for them.
                return {title: TAB_LABELS[params.tab], params: "", filename: "report.xlsx"};
            case "bs":
                // No depth: the tab has no such control and asks the engine for
                // an unclamped report, so stamping one on the workbook would
                // advertise a setting the reader cannot see or reproduce.
                return {title: "Balance Sheet", params: `as of ${params.asOf}`, filename: `balance-sheet-${params.asOf}.xlsx`};
            case "is":
                // No depth, for the reason `bs` has none: the tab dropped that
                // control and asks the engine for an unclamped report, so stamping
                // one on the workbook would advertise a setting the reader cannot
                // see or reproduce.
                return {
                    title: "Income Statement",
                    params: `${params.from} to ${params.to}`,
                    filename: `income-statement-${params.from}-to-${params.to}.xlsx`,
                };
            case "cf":
                return {title: "Cash Flow", params: `${span}, depth ${params.depth}`, filename: `cash-flow-${params.end}.xlsx`};
            case "nw":
                return {title: "Net Worth", params: `${span}, depth ${params.depth}`, filename: `net-worth-${params.end}.xlsx`};
        }
    });

    /** Export the current report — the two tagged shapes own their builders; cf/nw share exportXlsx. */
    function runExport(current: AnyReport): Promise<void> {
        const meta = {title: exportInfo.title, params: exportInfo.params};
        if (!("kind" in current)) return exportXlsx(current, meta, exportInfo.filename);
        switch (current.kind) {
            case "balanceSheet":
                return exportBalanceSheetXlsx(current, meta, exportInfo.filename);
            case "incomeStatement":
                return exportIncomeStatementXlsx(current, meta, exportInfo.filename);
        }
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
        <div class="alert rounded-box px-3 py-2 text-sm alert-warning" role="alert" data-testid="unpriced-warning">
            <span>Some holdings are not valued — no market price for: {unpriced.join(", ")}</span>
        </div>
    {/if}

    <!-- The self-hosted tabs render their own <AsyncSection>; everything else
         shares this one, which puts the error branch BEFORE the data branches
         and asks only about `status`. Tested after them, and additionally gated
         on `report === null`, it could never fire once any report had loaded —
         so a failed refetch just kept serving the previous answer under the new
         controls' label (FE-5). -->
    {#if params.tab === "insights"}
        <InsightsDashboard bind:params serverUrl={settings.serverUrl} {styles} />
    {:else if params.tab === "subs"}
        <SubscriptionsPanel serverUrl={settings.serverUrl} {styles} />
    {:else}
        <AsyncSection
            {view}
            value={shown}
            error={reports.error}
            testid="reports-error"
            label="the report"
            loadingLabel="Loading reports"
            onRetry={() => void reports.load(settings.serverUrl ?? "", reportQuery)}
        >
            {#snippet children(current)}
                {#if "kind" in current}
                    {#if current.kind === "balanceSheet"}
                        <BalanceSheetView report={current} {styles} />
                    {:else}
                        <IncomeStatementView report={current} {styles} flows={flowsPanel} />
                    {/if}
                {:else}
                    <ReportTable report={current} {styles} />
                {/if}
            {/snippet}
        </AsyncSection>
    {/if}
</div>

<ErrorToast message={journal.status === "error" ? journal.error : null} onRetry={() => void journal.refresh({force: true})} />
