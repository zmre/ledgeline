<!-- Insights panel (WP-05): collapsible box at the top of the journal view.
     Two views behind a tab strip, and the whole box still closes:

       Activity (default) — the period P&L for the FILTERED view, from the
         engine's journal summary (`journalView.summary`, which +page.svelte
         keeps in step with the filter) and its chart route.
       Balances — cash and short-term liabilities as of today, from the WHOLE
         journal and ignoring the filter bar entirely (see BalancesView for why).

     Collapsed state persists in settings.insightsOpen and the active tab in
     settings.insightsTab; the header row stays visible with the active view's
     headline figure even when collapsed. -->
<script lang="ts">
    import {formatAmount, toNumber} from "#lib/domain/money.js";
    import {DEFAULT_AMOUNT_STYLE, ZERO} from "#lib/format/amounts.js";
    import {signClass} from "#lib/format/sign.js";
    import {today} from "#lib/reports/periods.js";
    import {cashBalances} from "#lib/stores/cashBalances.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {journalView} from "#lib/stores/journalView.svelte.js";
    import {settings, type InsightsTab} from "#lib/stores/settings.svelte.js";
    import BalancesView from "./BalancesView.svelte";
    import BigNumbers from "./BigNumbers.svelte";
    import ChartWidget from "./ChartWidget.svelte";
    import DepthSlider from "./DepthSlider.svelte";

    const TABS: {id: InsightsTab; label: string}[] = [
        {id: "activity", label: "Activity"},
        {id: "balances", label: "Balances"},
    ];

    // The filtered view's figures; null while the first answer for a new
    // filter is in flight. Sign conventions are the engine's, detected over the
    // whole journal so they hold still as the filter moves.
    const summary = $derived(journalView.summary);
    const primary = $derived(summary?.perCommodity[0] ?? null);

    // The Balances view's data: the engine's answer for the whole journal as of
    // today, refetched whenever the journal generation moves (an edit, a file
    // change). Fetched only while that tab is the active one — it is the only
    // reader, and the collapsed header shows the ACTIVE tab's figure.
    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        if (serverUrl === null || generation === null || settings.insightsTab !== "balances") return;
        void cashBalances.load(serverUrl, {asOf: today(), generation});
    });
    const balancesView = $derived(cashBalances.value);

    // The collapsed header's figure follows the active tab, because that is the
    // one number worth a scan for whichever view you left it on.
    const headline = $derived.by(() => {
        if (settings.insightsTab === "activity") {
            const qty = primary?.net ?? ZERO;
            return {label: "Net", qty, text: formatAmount({commodity: primary?.commodity ?? "$", qty, style: primary?.style ?? DEFAULT_AMOUNT_STYLE})};
        }
        const summary = balancesView?.summaries[0];
        if (summary === undefined)
            return {
                label: "Net cash",
                qty: ZERO,
                text: balancesView === null ? "…" : formatAmount({commodity: primary?.commodity ?? "$", qty: ZERO, style: DEFAULT_AMOUNT_STYLE}),
            };
        return {label: "Net cash", qty: summary.net, text: formatAmount({commodity: summary.commodity, qty: summary.net, style: summary.style})};
    });

    // Default depth matches the reports page (defaultReportParams().depth). The
    // slider is bound to this same `depth` the chart consumes, so the bar and the
    // chart never drift.
    let depth = $state(2);
    const max = $derived(summary?.maxDepth ?? 1);
</script>

<section class="collapse-arrow collapse bg-base-200" data-testid="insights-panel">
    <input
        type="checkbox"
        checked={settings.insightsOpen}
        onchange={(e) => (settings.insightsOpen = e.currentTarget.checked)}
        aria-label="Toggle insights panel"
    />
    <div class="collapse-title flex min-h-0 items-center justify-between gap-2 py-3 pr-10">
        <h2 class="text-sm font-semibold tracking-tight">Insights</h2>
        <span class="text-sm">
            <span class="mr-1 text-base-content/60">{headline.label}</span>
            <span class="font-semibold {signClass(toNumber(headline.qty))}">{headline.text}</span>
        </span>
    </div>
    <!-- `{#if settings.insightsOpen}` because daisyUI's `collapse` is CSS ONLY:
         collapsing sets the content's height, it does not unmount anything. A
         collapsed panel therefore went on mounting BigNumbers and ChartWidget
         and recomputing both on every filter change — ~250 ms of whole-journal
         passes at 150k transactions, for a panel the user had deliberately shut.
         `max` is guarded with them because `maxAccountDepth` is another full
         pass and only the slider reads it.

         The HEADER above stays outside the guard on purpose: showing the active
         view's headline figure while collapsed is the point of collapsing rather
         than hiding.

         The TAB STRIP is inside the content rather than beside the title for a
         mechanical reason, not a stylistic one: daisyUI's collapse lays its
         checkbox over the whole title row, so a button up there would toggle the
         panel instead of switching views. -->
    <div class="collapse-content flex flex-col gap-4">
        {#if settings.insightsOpen}
            <div role="tablist" class="tabs tabs-box w-fit tabs-xs">
                {#each TABS as tab (tab.id)}
                    <button
                        type="button"
                        role="tab"
                        class="tab {settings.insightsTab === tab.id ? 'tab-active' : ''}"
                        aria-selected={settings.insightsTab === tab.id}
                        onclick={() => (settings.insightsTab = tab.id)}
                    >
                        {tab.label}
                    </button>
                {/each}
            </div>
            {#if settings.insightsTab === "balances"}
                {#if balancesView !== null}
                    <BalancesView view={balancesView} />
                {:else if cashBalances.status === "error"}
                    <p class="py-6 text-center text-sm text-error">Couldn't load balances: {cashBalances.error?.message ?? "unknown error"}</p>
                {:else}
                    <div class="flex justify-center py-6" aria-label="Loading balances"><span class="loading loading-spinner"></span></div>
                {/if}
            {:else}
                <BigNumbers {summary} />
                <ChartWidget {summary} {depth} />
                <!-- Keyed on max: the slider can mount while the summary is still loading (max=1),
                     and the browser clamps the input's value to that max without updating the
                     bound state; remounting once the real max arrives re-applies `depth`
                     (same guard as reports/ui/ReportControls.svelte). -->
                {#key max}
                    <DepthSlider bind:depth {max} />
                {/key}
            {/if}
        {/if}
    </div>
</section>
