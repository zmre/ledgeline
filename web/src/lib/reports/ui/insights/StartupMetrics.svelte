<!-- The startup-metrics section of Insights (plans/37), under the six headline
     boxes when the journal has recurring revenue or declares any `kpi:`.

     Every box, list line and note shows only when the engine sent its value: a
     figure the journal can't support is absent, never a $0.00. The hide rules
     for the boxes ABOVE this section (Net Worth → Runway, …) live in
     `startup.ts` `boxVisibility`, applied by InsightsDashboard. -->
<script lang="ts">
    import type {AmountStyle} from "#lib/domain/types.js";
    import type {SaasReport} from "#lib/reports/saasTypes.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import InfoTip from "#lib/components/InfoTip.svelte";
    import ChartPanel from "../ChartPanel.svelte";
    import {baseMetric, burnBand, deltaLine, fmt, fmtPct, fmtRatio, growthLine, pointsLine} from "./format";
    import {METRIC_HELP} from "./metricHelp";
    import MrrBridge from "./MrrBridge.svelte";
    import SegmentTable from "./SegmentTable.svelte";
    import StatBox from "./StatBox.svelte";
    import {burnMultipleLine, closedThroughMonth, hiddenNote, moreMetrics} from "./startup";

    let {report, styles}: {report: SaasReport; styles: ReadonlyMap<string, AmountStyle>} = $props();

    const base = $derived(report.base);
    const current = $derived(report.current);
    const previous = $derived(report.previous);
    const available = $derived(report.available);
    const note = $derived(hiddenNote(available));
    const closedThrough = $derived(closedThroughMonth(report));
    const more = $derived(moreMetrics(current, base, styles));
    const hasSegments = $derived(report.segments.length > 0);
    const unvalued = $derived([...report.unvalued.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
    // Customer counts from billings (Amendment A1.2), when per-customer MRR is hidden.
    const billed = $derived(current.customers === null ? current.billingCustomers : null);
    const inferredMark = $derived(available.inferredCustomers || (billed?.inferred ?? false) ? "*" : "");

    let segmentsOpen = $state(false);
</script>

{#snippet segmentToggle(box: string)}
    {#if hasSegments}
        <button
            type="button"
            class="btn -mt-1 -mr-2 btn-ghost btn-xs"
            aria-expanded={segmentsOpen}
            aria-label={segmentsOpen ? "Hide the segment breakdown" : "Show the segment breakdown"}
            title="Break down by segment"
            onclick={() => (segmentsOpen = !segmentsOpen)}
            data-testid="{box}-segments-toggle"
        >
            <span aria-hidden="true">{segmentsOpen ? "▴" : "▾"}</span>
        </button>
    {/if}
{/snippet}
{#snippet arrToggle()}{@render segmentToggle("insights-box-arr")}{/snippet}
{#snippet customersToggle()}{@render segmentToggle("insights-box-customers")}{/snippet}
{#snippet nrrToggle()}{@render segmentToggle("insights-box-nrr")}{/snippet}

<section class="flex flex-col gap-4" data-testid="startup-metrics" aria-label="Startup metrics">
    <div class="flex items-center gap-3">
        <h2 class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Startup metrics</h2>
        {#if closedThrough !== null}
            <span class="flex items-center gap-1 text-xs text-base-content/50" data-testid="startup-closed-through">
                Closed through {closedThrough}<InfoTip {...METRIC_HELP["closed-through"]} placement="bottom-start" />
            </span>
        {/if}
        <div class="h-px flex-1 bg-base-content/10"></div>
    </div>

    <div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {#if current.arr !== null}
            <!-- Run-rate ARR, labelled with the month it annualises (decision 5). -->
            {@const parts = [
                current.mrr === null ? null : `MRR ${fmt(base, current.mrr, styles)}`,
                current.usage === null || current.usage.m === 0n ? null : `usage ${fmt(base, current.usage, styles)}`,
            ].filter((part) => part !== null)}
            <StatBox
                title={current.month === null ? "ARR" : `ARR (${current.month} × 12)`}
                big={fmt(base, current.arr, styles)}
                extras={parts}
                small={previous.arr === null ? undefined : deltaLine(baseMetric(current.arr, previous.arr, base), base, styles, true)}
                help={METRIC_HELP.arr}
                testid="insights-box-arr"
                aside={arrToggle}
            />
        {/if}
        {#if current.netNewArr !== null}
            <StatBox
                title="Net new ARR"
                big={fmt(base, current.netNewArr, styles)}
                small={current.cmgrPct === null ? undefined : {text: `CMGR ${fmtPct(current.cmgrPct)}/mo`}}
                help={METRIC_HELP.netnewarr}
                testid="insights-box-netnewarr"
            />
        {/if}
        {#if current.customers !== null}
            {@const bridge = current.bridge}
            <StatBox
                title="Customers{inferredMark}"
                big={current.customers.toLocaleString("en-US")}
                extras={bridge === null ? [] : [`${bridge.newLogos} new`, `${bridge.churnedLogos} churned`]}
                small={growthLine(current.customerGrowthPct, true) ?? undefined}
                help={METRIC_HELP.customers}
                testid="insights-box-customers"
                aside={customersToggle}
            />
        {:else if billed !== null}
            <StatBox
                title="Customers (from billings){inferredMark}"
                big={billed.customers.toLocaleString("en-US")}
                extras={[`${billed.newLogos} new`, `${billed.lapsedLogos} lapsed`]}
                small={growthLine(billed.customerGrowthPct, true) ?? undefined}
                help={METRIC_HELP["customers-billings"]}
                testid="insights-box-customers"
            />
        {/if}
        {#if current.nrrPct !== null}
            <StatBox
                title="NRR"
                big={fmtPct(current.nrrPct)}
                extras={["trailing 12 months"]}
                small={current.grrPct === null ? undefined : {text: `GRR ${fmtPct(current.grrPct)}`}}
                help={METRIC_HELP.nrr}
                testid="insights-box-nrr"
                aside={nrrToggle}
            />
        {/if}
        {#if current.burnMultiple !== null}
            {@const band = burnBand(current.burnMultiple)}
            <StatBox
                title="Burn multiple"
                big={fmtRatio(current.burnMultiple)}
                bigClass={band.klass}
                extras={[band.label]}
                small={{text: burnMultipleLine(current, base, styles)}}
                help={METRIC_HELP.burnmultiple}
                testid="insights-box-burnmultiple"
            />
        {/if}
        {#if current.grossMarginPct !== null}
            <StatBox
                title="Gross margin"
                big={fmtPct(current.grossMarginPct)}
                small={pointsLine(current.grossMarginPct, previous.grossMarginPct, true) ?? undefined}
                help={METRIC_HELP.grossmargin}
                testid="insights-box-grossmargin"
            />
        {/if}
    </div>

    {#if segmentsOpen && hasSegments}
        <SegmentTable segments={report.segments} {base} {styles} />
    {/if}

    {#if note !== null}
        <p class="text-sm text-base-content/60" data-testid="startup-customers-hidden">
            {note.before}{#if note.tag}<code>customer:</code>{/if}{note.after}
            <!-- An external docs URL (`STARTUP_DOCS`), not an app route: resolve() doesn't apply. -->
            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
            <a class="link link-hover" href={note.link.href} target="_blank" rel="noopener noreferrer">{note.link.text}</a>{note.tag ? "" : "."}
        </p>
    {/if}

    {#if current.bridge !== null && current.mrr !== null}
        <MrrBridge
            bridge={current.bridge}
            end={current.mrr}
            customers={report.bridgeCustomers}
            {base}
            {styles}
            periodLabel="{report.period.currStart} → {current.month ?? report.period.currEnd}"
        />
    {/if}

    {#if more.length > 0}
        <ChartPanel heading="More metrics" open={settings.startupMoreOpen} onToggle={(open) => (settings.startupMoreOpen = open)} testid="startup-more">
            <dl class="grid grid-cols-1 gap-x-6 gap-y-1 text-sm sm:grid-cols-2 lg:grid-cols-3">
                {#each more as metric (metric.key)}
                    <div class="flex justify-between gap-3 border-b border-base-content/5 py-1">
                        <dt class="flex items-center gap-1 text-base-content/70">
                            {metric.label}<InfoTip {...METRIC_HELP[metric.key]} placement="bottom-start" />
                        </dt>
                        <dd class="font-mono tabular-nums" data-testid="startup-more-{metric.key}">{metric.value}</dd>
                    </div>
                {/each}
            </dl>
        </ChartPanel>
    {/if}

    {#if billed?.windowShort === true}
        <p class="text-xs text-base-content/60" data-testid="startup-window-short-note">Counts may be low until there are 12 months of history.</p>
    {/if}
    {#if inferredMark !== ""}
        <p class="text-xs text-warning/80" data-testid="startup-inferred-note">
            * Some customers inferred from payees: their revenue had no <code>customer:</code> tag, so the transaction's payee names them.
        </p>
    {/if}
    {#if unvalued.length > 0}
        <p class="text-xs text-warning/80" data-testid="startup-unvalued-note">
            Left out of MRR, no price to value it: {unvalued.map(([commodity, qty]) => fmt(commodity, qty, styles)).join(", ")}
        </p>
    {/if}
</section>
