<!-- The Insights dashboard: a period control plus a responsive grid of
     period-over-period metric boxes (Boxes 1–6). Owns its own data load from
     the /api/insights endpoint via the insights store; the comparison span is
     bound to the shared ReportParams (insStart/insEnd) so it round-trips in the
     URL like the other tabs. -->
<script lang="ts">
    import {journal} from "#lib/stores/journal.svelte.js";
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import {maIsZero} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import type {InsightsReport} from "#lib/reports/insightsTypes.js";
    import type {ReportParams} from "#lib/reports/ui/params.js";
    import {insights} from "#lib/stores/insights.svelte.js";
    import {dataView} from "#lib/stores/loadState.js";
    import {saas} from "#lib/stores/saas.svelte.js";
    import ChangeList from "./ChangeList.svelte";
    import {baseMetric, deltaLine, extras, fmt, fmtBase, fmtSignedAmount, fmtSignedPct, monthlyAverage, signClass} from "./format";
    import MoversList from "./MoversList.svelte";
    import PeriodControl from "./PeriodControl.svelte";
    import StartupMetrics from "./StartupMetrics.svelte";
    import StatBox from "./StatBox.svelte";
    import {today} from "#lib/reports/periods.js";
    import {METRIC_HELP} from "./metricHelp";
    import {boxVisibility, runwayText, sameSpan, showsStartup} from "./startup";
    import TopTxnsList from "./TopTxnsList.svelte";

    let {params = $bindable(), serverUrl, styles}: {params: ReportParams; serverUrl: string | null; styles: ReadonlyMap<string, AmountStyle>} = $props();

    // Refetch whenever the span changes, the server is first configured, or the
    // journal changes (a refresh, an edit on disk). The startup metrics load in
    // parallel on the same span; each part renders when its own data lands.
    $effect(() => {
        void journal.changes;
        const url = serverUrl;
        if (url === null) return;
        const span = {start: params.insStart, end: params.insEnd};
        void insights.load(url, span);
        void saas.load(url, {...span, asOf: today()});
    });

    /**
     * The startup report, for the hide rules, once it has answered. Until then
     * (and while it is failing) the dashboard reads the journal as a household:
     * Net Worth stays. Its failure gets its own banner below, so a broken
     * /api/saas never takes the rest of Insights down with it.
     */
    // Only a saas report for the span the insights report answers counts: one
    // still held from the previous span reads as "not loaded yet" (FE-1), so
    // it can neither draw old-span boxes nor keep a stale Runway on screen.
    const saasView = $derived(dataView(saas.status, saas.value !== null, sameSpan(saas.query, insights.query)));
    const startup = $derived(saasView === "data" ? saas.value : null);

    /**
     * Whether the previous period is actually backed by data. Every box on this
     * dashboard compares against it, so a previous period the journal only
     * partly covers makes each delta overstate growth (six months of history
     * measured against twelve reads as a doubling).
     */
    function coverageNote(report: InsightsReport): string | null {
        const {journalStart, period} = report;
        if (journalStart === null || journalStart > period.prevEnd) {
            return `No data before ${period.prevEnd} — there is nothing to compare this period against.`;
        }
        if (journalStart > period.prevStart) {
            return `Comparisons are skewed: the journal starts ${journalStart}, so the previous period (${period.prevStart} → ${period.prevEnd}) is only partly covered.`;
        }
        return null;
    }
</script>

<div class="flex flex-col gap-4" data-testid="insights-dashboard">
    <!-- While a new period loads, the previous period's boxes stay on screen
         (blanking them would flash the whole dashboard on every change) but are
         dimmed and marked busy, with a spinner here, so they never pass for the
         answer to the period the control now reads. The spinner is hidden under
         reduced motion; the "Updating…" text carries the state on its own. -->
    <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <PeriodControl bind:start={params.insStart} bind:end={params.insEnd} />
        {#if insights.refreshing}
            <span class="flex items-center gap-2 text-xs text-base-content/60" role="status" data-testid="insights-refreshing">
                <span class="loading loading-sm loading-spinner motion-reduce:hidden" aria-hidden="true"></span>
                Updating…
            </span>
        {/if}
    </div>

    <!-- Error before data (FE-5) — ordered once, inside <AsyncSection>: with the
         data branch first, moving the period and hitting a failure left the
         PREVIOUS period's boxes on screen under the new period's dates. -->
    <div
        class={["flex flex-col gap-4 transition-opacity duration-200 motion-reduce:transition-none", insights.refreshing && "opacity-60"]}
        aria-busy={insights.refreshing}
        data-testid="insights-content"
    >
        <AsyncSection
            view={insights.view}
            value={insights.value}
            error={insights.error}
            testid="insights-error"
            label="insights"
            loadingLabel="Loading insights"
            onRetry={() => void insights.load(serverUrl ?? "", {start: params.insStart, end: params.insEnd})}
        >
            {#snippet children(report)}
                {@const coverage = coverageNote(report)}
                {@const base = report.base}
                {@const period = report.period}
                {@const inv = report.investment}
                <!-- Cost of living is averaged at the display boundary (engine keeps totals exact). -->
                {@const colCur = monthlyAverage(report.costOfLiving.currentTotal.get(base), report.costOfLiving.monthsCurrent)}
                {@const colPrev = monthlyAverage(report.costOfLiving.previousTotal.get(base), report.costOfLiving.monthsPrevious)}
                <!-- plans/37 decision 4: boxes that don't apply hide, and the grid reflows. -->
                {@const shows = boxVisibility(report, startup)}
                {@const runway = shows.runway && startup !== null ? runwayText(startup.current, base, styles) : null}
                <div class="text-xs text-base-content/60">
                    Current <span class="font-medium text-base-content/80">{period.currStart} → {period.currEnd}</span>
                    vs previous <span class="font-medium text-base-content/80">{period.prevStart} → {period.prevEnd}</span>
                </div>

                {#if coverage !== null}
                    <div class="alert rounded-box px-3 py-2 text-sm alert-warning" role="alert" data-testid="insights-coverage-warning">
                        <span>{coverage}</span>
                    </div>
                {/if}

                <div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
                    <!-- Box 1: Revenue -->
                    <StatBox
                        title="Revenue"
                        big={fmtBase(report.revenue.current, base, styles)}
                        extras={extras(report.revenue.current, base, styles)}
                        small={deltaLine(report.revenue, base, styles, true)}
                        help={METRIC_HELP.revenue}
                        testid="insights-box-revenue"
                    />
                    <!-- Box 2: Expenses -->
                    <StatBox
                        title="Expenses"
                        big={fmtBase(report.expenses.current, base, styles)}
                        extras={extras(report.expenses.current, base, styles)}
                        small={deltaLine(report.expenses, base, styles, false)}
                        help={METRIC_HELP.expenses}
                        testid="insights-box-expenses"
                    />
                    <!-- Box 3: Net Worth (end of period), or Runway in its slot for a
                     business or a negative net worth. -->
                    {#if shows.netWorth}
                        <StatBox
                            title="Net Worth"
                            big={fmtBase(report.netWorth.current, base, styles)}
                            extras={extras(report.netWorth.current, base, styles)}
                            small={deltaLine(report.netWorth, base, styles, true)}
                            help={METRIC_HELP.networth}
                            testid="insights-box-networth"
                        />
                    {:else if runway !== null}
                        <StatBox title="Runway" big={runway.big} small={{text: runway.small}} help={METRIC_HELP.runway} testid="insights-box-runway" />
                    {/if}

                    <!-- Box 4: Average Monthly Cost of Living -->
                    <StatBox
                        title="Avg Monthly Costs"
                        big={fmt(base, colCur, styles)}
                        small={deltaLine(baseMetric(colCur, colPrev, base), base, styles, false)}
                        help={METRIC_HELP.costofliving}
                        testid="insights-box-costofliving"
                    />

                    <!-- Box 5: Investment Performance (current big; previous as the small line), only with holdings -->
                    {#if shows.investment}
                        <StatBox
                            title="Investment Performance"
                            big={`${fmtSignedAmount(inv.current.gain, base, styles)} (${fmtSignedPct(inv.current.gainPct)})`}
                            bigClass={signClass(inv.current.gain === null ? null : inv.current.gain.m)}
                            small={{
                                text: `Prev: ${fmtSignedAmount(inv.previous.gain, base, styles)} (${fmtSignedPct(inv.previous.gainPct)})`,
                                klass: signClass(inv.previous.gain === null ? null : inv.previous.gain.m),
                            }}
                            help={METRIC_HELP.investment}
                            testid="insights-box-investment"
                        />
                    {/if}

                    <!-- Box 6: Cash Balance (end of period) -->
                    <StatBox
                        title="Cash Balance"
                        big={fmtBase(report.cashBalance.current, base, styles)}
                        extras={extras(report.cashBalance.current, base, styles)}
                        small={deltaLine(report.cashBalance, base, styles, true)}
                        help={METRIC_HELP.cash}
                        testid="insights-box-cash"
                    />
                </div>

                <!-- Its own tri-state on its own resource: a failed /api/saas is
                 a banner here, never the whole dashboard. Quiet while loading,
                 because most journals have no startup section to wait for. -->
                <AsyncSection
                    view={saasView}
                    value={saas.value}
                    error={saas.error}
                    testid="startup-error"
                    label="startup metrics"
                    loadingLabel="Loading startup metrics"
                    quietLoading
                    onRetry={() => void saas.load(serverUrl ?? "", {start: params.insStart, end: params.insEnd, asOf: today()})}
                >
                    {#snippet children(held)}
                        {#if showsStartup(held.available)}
                            <StartupMetrics report={held} {styles} />
                        {/if}
                    {/snippet}
                </AsyncSection>

                <!-- Boxes 7–10: the "biggest / top" lists (current period). `hasPrevious`
             lets an empty list say "not enough history" rather than "no changes". -->
                <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
                    <ChangeList
                        title="Biggest Expense Changes"
                        rows={report.expenseChanges}
                        {base}
                        {styles}
                        goodWhenUp={false}
                        hasPrevious={!maIsZero(report.expenses.previous)}
                        help={METRIC_HELP.expensechanges}
                        testid="insights-box-expensechanges"
                    />
                    {#if shows.movers}
                        <MoversList rows={report.movers} {base} {styles} periodStart={period.currStart} testid="insights-box-movers" />
                    {/if}
                    <ChangeList
                        title="Biggest Revenue Changes"
                        rows={report.revenueChanges}
                        {base}
                        {styles}
                        goodWhenUp={true}
                        hasPrevious={!maIsZero(report.revenue.previous)}
                        help={METRIC_HELP.revenuechanges}
                        testid="insights-box-revenuechanges"
                    />
                    <TopTxnsList rows={report.topTxns} {base} {styles} testid="insights-box-toptxns" />
                </div>
            {/snippet}
        </AsyncSection>
    </div>
</div>
