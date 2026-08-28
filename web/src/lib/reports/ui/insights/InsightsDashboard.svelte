<!-- The Insights dashboard: a period control plus a responsive grid of
     period-over-period metric boxes (Boxes 1–6). Owns its own data load from
     the /api/insights endpoint via the insights store; the comparison span is
     bound to the shared ReportParams (insStart/insEnd) so it round-trips in the
     URL like the other tabs. -->
<script lang="ts">
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {maIsZero, sub, toNumber, type Dec} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import type {InsightsReport, MetricDelta} from "$lib/reports/insightsTypes";
    import type {ReportParams} from "$lib/reports/ui/params";
    import {insights} from "$lib/stores/insights.svelte";
    import ChangeList from "./ChangeList.svelte";
    import {deltaLine, extras, fmt, fmtBase, fmtSignedAmount, fmtSignedPct, monthlyAverage, signClass} from "./format";
    import MoversList from "./MoversList.svelte";
    import PeriodControl from "./PeriodControl.svelte";
    import StatBox from "./StatBox.svelte";
    import TopTxnsList from "./TopTxnsList.svelte";

    let {params = $bindable(), serverUrl, styles}: {params: ReportParams; serverUrl: string | null; styles: ReadonlyMap<string, AmountStyle>} = $props();

    // Refetch whenever the span changes (or the server is first configured).
    $effect(() => {
        const url = serverUrl;
        if (url === null) return;
        void insights.load(url, {start: params.insStart, end: params.insEnd});
    });

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

    /** A single-commodity MetricDelta over the base, for the cost-of-living averages. */
    function baseMetric(current: Dec, previous: Dec, base: string): MetricDelta {
        const delta = sub(current, previous);
        const pct = previous.m === 0n ? null : ((toNumber(current) - toNumber(previous)) / Math.abs(toNumber(previous))) * 100;
        return {current: new Map([[base, current]]), previous: new Map([[base, previous]]), delta: new Map([[base, delta]]), pct};
    }
</script>

<div class="flex flex-col gap-4" data-testid="insights-dashboard">
    <PeriodControl bind:start={params.insStart} bind:end={params.insEnd} />

    <!-- Error before data (FE-5) — ordered once, inside <AsyncSection>: with the
         data branch first, moving the period and hitting a failure left the
         PREVIOUS period's boxes on screen under the new period's dates. -->
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
                    testid="insights-box-revenue"
                />
                <!-- Box 2: Expenses -->
                <StatBox
                    title="Expenses"
                    big={fmtBase(report.expenses.current, base, styles)}
                    extras={extras(report.expenses.current, base, styles)}
                    small={deltaLine(report.expenses, base, styles, false)}
                    testid="insights-box-expenses"
                />
                <!-- Box 3: Net Worth (end of period) -->
                <StatBox
                    title="Net Worth"
                    big={fmtBase(report.netWorth.current, base, styles)}
                    extras={extras(report.netWorth.current, base, styles)}
                    small={deltaLine(report.netWorth, base, styles, true)}
                    testid="insights-box-networth"
                />

                <!-- Box 4: Average Monthly Cost of Living -->
                <StatBox
                    title="Avg Monthly Costs"
                    big={fmt(base, colCur, styles)}
                    small={deltaLine(baseMetric(colCur, colPrev, base), base, styles, false)}
                    testid="insights-box-costofliving"
                />

                <!-- Box 5: Investment Performance (current big; previous as the small line) -->
                <StatBox
                    title="Investment Performance"
                    big={`${fmtSignedAmount(inv.current.gain, base, styles)} (${fmtSignedPct(inv.current.gainPct)})`}
                    bigClass={signClass(inv.current.gain === null ? null : inv.current.gain.m)}
                    small={{
                        text: `Prev: ${fmtSignedAmount(inv.previous.gain, base, styles)} (${fmtSignedPct(inv.previous.gainPct)})`,
                        klass: signClass(inv.previous.gain === null ? null : inv.previous.gain.m),
                    }}
                    testid="insights-box-investment"
                />

                <!-- Box 6: Cash Balance (end of period) -->
                <StatBox
                    title="Cash Balance"
                    big={fmtBase(report.cashBalance.current, base, styles)}
                    extras={extras(report.cashBalance.current, base, styles)}
                    small={deltaLine(report.cashBalance, base, styles, true)}
                    testid="insights-box-cash"
                />
            </div>

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
                    testid="insights-box-expensechanges"
                />
                <MoversList rows={report.movers} {base} {styles} periodStart={period.currStart} testid="insights-box-movers" />
                <ChangeList
                    title="Biggest Revenue Changes"
                    rows={report.revenueChanges}
                    {base}
                    {styles}
                    goodWhenUp={true}
                    hasPrevious={!maIsZero(report.revenue.previous)}
                    testid="insights-box-revenuechanges"
                />
                <TopTxnsList rows={report.topTxns} {base} {styles} testid="insights-box-toptxns" />
            </div>
        {/snippet}
    </AsyncSection>
</div>
