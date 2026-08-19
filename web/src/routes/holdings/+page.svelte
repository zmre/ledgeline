<script lang="ts">
    // Holdings route (WP-10, now native; split into sub-tabs by plans/14): scope
    // bar on top, then a Stocks / Other tab strip, then that tab's screen.
    //
    // Stocks is the original page — collapsible insight section (pie + stat tiles
    // + gainers/losers) and the details table. Other is the account-keyed report
    // for the assets that are neither securities nor cash (a house, a van, a
    // partnership): table + trend, no pie and no gainers/losers, because both say
    // little about three illiquid assets and a pie of "house, car" is a pie of two
    // slices.
    //
    // Both are fetched from the ledgeline-engine /api/holdings[/other][/series]
    // endpoints for the SAME scope and decoded into the domain types, so the UI
    // renders unchanged; scope/as-of changes refetch. The scope AND the tab live
    // in the URL (?asof=&acct=&mode=&gain=&tab=) via the WP-04 replaceState
    // pattern, through ONE writer — see holdings/ui/urlCodec.ts. Display styles
    // for the base commodity come from the journal wire feed (styleFor).
    import {onMount} from "svelte";
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import ErrorToast from "$lib/components/ErrorToast.svelte";
    import {formatAmount, type Dec} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import {exportHoldingsXlsx} from "$lib/export/xlsx";
    import {styleOf} from "$lib/format/amounts";
    import GainersLosers from "$lib/holdings/ui/GainersLosers.svelte";
    import HoldingsPie from "$lib/holdings/ui/HoldingsPie.svelte";
    import HoldingsStats from "$lib/holdings/ui/HoldingsStats.svelte";
    import HoldingsTable from "$lib/holdings/ui/HoldingsTable.svelte";
    import HoldingsTabs from "$lib/holdings/ui/HoldingsTabs.svelte";
    import HoldingsTrend from "$lib/holdings/ui/HoldingsTrend.svelte";
    import OtherHoldingsTable from "$lib/holdings/ui/OtherHoldingsTable.svelte";
    import ScopeBar from "$lib/holdings/ui/ScopeBar.svelte";
    import {startHoldingsUrlSync} from "$lib/holdings/ui/urlSync";
    import {formatUnitsWith, partitionShortPositions, shortPositionNote} from "$lib/holdings/ui/view";
    import {formatChartValue, formatCompactChartValue, styleFor} from "$lib/insights/series";
    import ExportButton from "$lib/reports/ui/ExportButton.svelte";
    import {holdingsData, holdingsScope, holdingsTab, otherHoldingsData} from "$lib/stores/holdings.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {loadJournalWhenReady} from "$lib/stores/serverWatch.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    // Reset the scope and tab from the URL once (fresh visits open at today, on
    // Stocks), then mirror changes back (debounced replaceState). onMount's
    // return value is its cleanup.
    onMount(() => startHoldingsUrlSync());

    // Load the journal once a server URL is configured (base-commodity styles + the scope-bar account list only).
    loadJournalWhenReady();

    // Fetch the native holdings report + trend whenever the scope changes, the
    // server is first configured, or the user reconnects. The nonce is in the
    // key because a reconnect usually leaves the URL identical (the engine
    // restarted on the same port), and keying on the URL alone meant this page
    // never retried after one (FE-5d). The TAB is deliberately absent from this
    // key: it is not part of the scope, so clicking a tab refetches nothing.
    const loadKey = $derived({url: settings.serverUrl, nonce: settings.serverNonce, scope: holdingsScope.value});
    $effect(() => {
        const {url, scope} = loadKey;
        if (url !== null) void holdingsData.load(url, scope);
    });

    // The Other report is fetched the FIRST time its tab is opened (plans/14), so
    // a user who never clicks it pays nothing. The latch only ever goes false →
    // true, which is what keeps the load effect below firing on scope changes and
    // NOT on every return to the tab.
    let otherOpened = $state(false);
    $effect(() => {
        if (holdingsTab.value === "other") otherOpened = true;
    });
    $effect(() => {
        const {url, scope} = loadKey;
        if (url !== null && otherOpened) void otherHoldingsData.load(url, scope);
    });

    const report = $derived(holdingsData.report);
    const trend = $derived(holdingsData.trend);
    const otherReport = $derived(otherHoldingsData.report);
    const otherTrend = $derived(otherHoldingsData.trend);
    // Readiness (`holdingsData.view`) is the ENGINE's report, nothing else. It
    // used to also require `journal.txns.length > 0` — a row count from the
    // separate hledger-web feed — so a working engine plus a failing feed spun
    // forever, and so did a legitimately empty journal on a new user's first
    // run (FE-5c). The base commodity's display style is the only thing that
    // feed contributes here, and `styleFor` already falls back when it has none.

    // Both reports name the same base; the Other one is a fallback so the Other
    // tab still formats money when the STOCK request is the one that failed.
    const base = $derived(report?.base ?? otherReport?.base ?? "$");
    const gainPeriod = $derived(holdingsScope.value.gainPeriod);
    const style = $derived(styleFor(journal.txns, base));
    const format = (qty: Dec): string => formatAmount({commodity: base, qty, style});
    const formatTrendValue = (v: number): string => formatChartValue(v, base, style);
    const formatTrendAxis = (v: number): string => formatCompactChartValue(v, base, style);
    // The scope chooser's options are per-TAB, and each tab's list comes off its
    // OWN report. Neither is derived here: membership turns on the `holdings:`
    // account tag, which the SPA's account declarations (name + type) cannot see,
    // and the engine pins the two lists disjoint.
    //
    // Before a report has loaded there is no honest list, so the bar offers NONE
    // rather than the other tab's set — a chooser listing brokerage accounts as
    // filters for a house is a wrong answer, where an empty one is merely an
    // early one, and it fills in the moment the report lands.
    const accountNames = $derived((holdingsTab.value === "stocks" ? report?.accounts : otherReport?.accounts) ?? []);

    // Display styles for the units the Other tab prints ("1 HOUSE"), resolved
    // ONCE per report rather than per cell: `styleFor` walks the whole journal
    // feed, and a 50k-transaction scan behind every render of every row is not a
    // lookup. The set of commodities here is tiny by construction — a house, a
    // van — so the memo is cheap and rebuilds only when the report or the feed
    // does.
    // Built from a finished list rather than by mutating a Map: this is an
    // immutable snapshot the derivation replaces wholesale, so a SvelteMap would
    // add a reactivity layer over a value that never changes in place.
    const unitStyles = $derived.by(() => {
        const commodities: string[] = [];
        for (const holding of otherReport?.holdings ?? []) {
            for (const commodity of holding.commodities.keys()) {
                if (!commodities.includes(commodity)) commodities.push(commodity);
            }
        }
        return new Map<string, AmountStyle>(commodities.map((commodity) => [commodity, styleFor(journal.txns, commodity)]));
    });
    const formatUnits = $derived(formatUnitsWith((commodity: string) => styleOf(unitStyles, commodity)));

    // The engine reports net-SHORT rows (sold more than was ever bought) so its
    // totals reconcile with the balance sheet, but nobody holds −2 shares, so the
    // table, pie and stat tiles show the real positions only. The totals stay the
    // engine's — never recomputed here — and the note below the table accounts for
    // the difference.
    const positions = $derived(partitionShortPositions(report?.holdings ?? []));
    const hiddenNote = $derived(shortPositionNote(positions.hidden, format));

    let insightsOpen = $state(true);
</script>

<svelte:head><title>Ledgeline — Holdings</title></svelte:head>

<div class="flex flex-col gap-3">
    <ScopeBar {accountNames} />
    <HoldingsTabs bind:tab={holdingsTab.value} />

    {#if holdingsTab.value === "stocks"}
        <!-- Error first — ordered once, inside <AsyncSection>: tested after the data
             branch and gated on `report === null`, it could never fire once a report
             had loaded, so moving the as-of and hitting a 500 kept the OLD date's
             portfolio on screen (FE-5). -->
        <AsyncSection
            view={holdingsData.view}
            value={report}
            error={holdingsData.error}
            testid="holdings-error"
            label="holdings"
            loadingLabel="Loading holdings"
            onRetry={() => void holdingsData.load(settings.serverUrl ?? "", holdingsScope.value)}
        >
            {#snippet children(report)}
                {#if positions.shown.length > 0}
                    <section class="collapse-arrow bg-base-200 collapse" data-testid="holdings-insights">
                        <input type="checkbox" bind:checked={insightsOpen} aria-label="Toggle holdings insights" />
                        <div class="collapse-title flex min-h-0 items-center justify-between gap-2 py-3 pr-10">
                            <h2 class="text-sm font-semibold tracking-tight">Insights</h2>
                            <span class="text-sm">
                                <span class="text-base-content/60 mr-1">Market value</span>
                                <span class="font-semibold">{format(report.totals.marketValue)}</span>
                            </span>
                        </div>
                        <div class="collapse-content flex flex-col gap-4">
                            <HoldingsStats totals={report.totals} holdings={positions.shown} {format} {gainPeriod} />
                            <div class="grid grid-cols-1 items-center gap-4 lg:grid-cols-2">
                                <div>
                                    <HoldingsPie holdings={positions.shown} {format} />
                                </div>
                                <GainersLosers {report} {format} />
                            </div>
                            {#if trend !== null}
                                <HoldingsTrend {trend} formatValue={formatTrendValue} formatAxis={formatTrendAxis} />
                            {/if}
                        </div>
                    </section>
                {/if}

                {#if report.warnings.length > 0}
                    <div class="alert alert-warning rounded-box items-start px-3 py-2 text-sm" role="alert" data-testid="holdings-warnings">
                        <ul class="list-inside list-disc">
                            {#each report.warnings as warning (warning.symbol + warning.kind)}
                                <li>{warning.message}</li>
                            {/each}
                        </ul>
                    </div>
                {/if}

                {#if positions.shown.length === 0}
                    <div class="card bg-base-200" data-testid="holdings-empty">
                        <div class="card-body items-center py-16 text-center">
                            <h2 class="card-title">No stock holdings in scope</h2>
                            <p class="text-base-content/60">
                                No non-currency commodities are held by the selected accounts as of {report.asOf}. Widen the scope or pick a later date.
                            </p>
                        </div>
                    </div>
                {:else}
                    <div class="flex justify-end">
                        <!-- The export is the full engine report, short rows included: a spreadsheet whose rows sum to its own totals. -->
                        <ExportButton
                            run={() => exportHoldingsXlsx(report, {title: "Holdings", params: `As of ${report.asOf}`}, `holdings-${report.asOf}.xlsx`)}
                        />
                    </div>
                    <HoldingsTable holdings={positions.shown} totals={report.totals} {format} {gainPeriod} />
                {/if}
                <!-- Why the visible rows do not add up to the totals row above them. -->
                {#if hiddenNote !== null}
                    <p class="text-base-content/60 px-1 text-xs" data-testid="holdings-hidden-note">{hiddenNote}</p>
                {/if}
            {/snippet}
        </AsyncSection>
    {:else}
        <!-- Same tri-state, same shared chain, its own testid — see the FE-5 note above. -->
        <AsyncSection
            view={otherHoldingsData.view}
            value={otherReport}
            error={otherHoldingsData.error}
            testid="other-holdings-error"
            label="other holdings"
            loadingLabel="Loading other holdings"
            onRetry={() => void otherHoldingsData.load(settings.serverUrl ?? "", holdingsScope.value)}
        >
            {#snippet children(report)}
                {#if report.holdings.length > 0}
                    <OtherHoldingsTable holdings={report.holdings} totals={report.totals} base={report.base} {format} {formatUnits} {gainPeriod} />
                    <!-- HoldingsTrend unchanged: the Other series is the stock series'
                         wire shape byte for byte, which is exactly why the engine reuses it. -->
                    {#if otherTrend !== null}
                        <HoldingsTrend trend={otherTrend} formatValue={formatTrendValue} formatAxis={formatTrendAxis} />
                    {/if}
                {/if}

                <!-- Below the table, not above it: an unpriced asset is a row that
                     contributes to no total, so the warning explains the em-dashes
                     the reader has just looked at. -->
                {#if report.warnings.length > 0}
                    <div class="alert alert-warning rounded-box items-start px-3 py-2 text-sm" role="alert" data-testid="other-holdings-warnings">
                        <ul class="list-inside list-disc">
                            {#each report.warnings as warning (warning.account + warning.kind)}
                                <li>{warning.message}</li>
                            {/each}
                        </ul>
                    </div>
                {/if}

                {#if report.holdings.length === 0}
                    <div class="card bg-base-200" data-testid="other-holdings-empty">
                        <div class="card-body items-center py-16 text-center">
                            <h2 class="card-title">No other holdings in scope</h2>
                            <p class="text-base-content/60">
                                No non-stock, non-cash assets are held by the selected accounts as of {report.asOf}. Tag an account
                                <code class="text-base-content/80">holdings: other</code> to list it here, or widen the scope.
                            </p>
                        </div>
                    </div>
                {/if}
            {/snippet}
        </AsyncSection>
    {/if}
</div>

<ErrorToast message={journal.status === "error" ? journal.error : null} onRetry={() => void journal.refresh({force: true})} />
