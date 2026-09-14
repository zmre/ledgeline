<!-- Insights panel (WP-05): collapsible box at the top of the journal view.
     Two views behind a tab strip, and the whole box still closes:

       Activity (default) — the period P&L. Receives the ALREADY-FILTERED
         transactions as a prop (the journal store / +page.svelte owns filtering
         — WP-03/04); everything here is $derived, so mode/interval/depth
         changes never refetch.
       Balances — cash and short-term liabilities as of today, from the WHOLE
         journal and ignoring the filter bar entirely (see BalancesView for why).

     Collapsed state persists in settings.insightsOpen and the active tab in
     settings.insightsTab; the header row stays visible with the active view's
     headline figure even when collapsed. -->
<script lang="ts">
    import type {AccountDecl} from "$lib/domain/accountTypes";
    import {formatAmount, toNumber} from "$lib/domain/money";
    import type {Transaction} from "$lib/domain/types";
    import {signClass} from "$lib/format/sign";
    import {accountBalances, balanceCommodities, summarize} from "$lib/reports/cashBalances";
    import {today} from "$lib/reports/periods";
    import {settings, type InsightsTab} from "$lib/stores/settings.svelte";
    import BalancesView from "./BalancesView.svelte";
    import BigNumbers from "./BigNumbers.svelte";
    import ChartWidget from "./ChartWidget.svelte";
    import DepthSlider from "./DepthSlider.svelte";
    import {bigNumbers, commoditiesInUse, maxAccountDepth, styleFor, type AccountSelection, type DeclaredTypes} from "./series";

    // txns: the filtered view to summarize/chart. allTxns: the whole journal —
    // used to detect the journal's sign conventions so they stay stable across
    // filter changes (see series.signConventions), and as the only correct input
    // to the cumulative balances. decls: the raw declarations, which carry the
    // `bsterm:` tag `declared` (a type map) cannot.
    let {
        txns,
        accounts,
        allTxns,
        declared,
        decls = [],
    }: {txns: Transaction[]; accounts?: AccountSelection; allTxns?: Transaction[]; declared?: DeclaredTypes; decls?: readonly AccountDecl[]} = $props();

    const TABS: {id: InsightsTab; label: string}[] = [
        {id: "activity", label: "Activity"},
        {id: "balances", label: "Balances"},
    ];

    const journalTxns = $derived(allTxns ?? txns);

    const primary = $derived(commoditiesInUse(txns, accounts)[0] ?? "$");
    const net = $derived(bigNumbers(txns, primary, accounts, allTxns, declared).net);

    // The collapsed header's figure follows the active tab, because that is the
    // one number worth a scan for whichever view you left it on. Both are cheap
    // for the same reason: the activity net is the value BigNumbers computes
    // anyway, and the balances net comes off the memo the `negative-cash` check
    // rule already fills once per journal swap (lib/reports/cashBalances).
    const headline = $derived.by(() => {
        if (settings.insightsTab === "activity") {
            return {label: "Net", qty: net, text: formatAmount({commodity: primary, qty: net, style: styleFor(txns, primary)})};
        }
        const balances = accountBalances(journalTxns, decls, today());
        const commodity = balanceCommodities(balances)[0] ?? primary;
        const {net: netCash} = summarize(balances, commodity);
        return {label: "Net cash", qty: netCash, text: formatAmount({commodity, qty: netCash, style: styleFor(journalTxns, commodity)})};
    });

    // Default depth matches the reports page (defaultReportParams().depth). The
    // slider is bound to this same `depth` the chart consumes, so the bar and the
    // chart never drift.
    let depth = $state(2);
    const max = $derived(maxAccountDepth(txns, accounts));
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
                <BalancesView allTxns={journalTxns} {decls} />
            {:else}
                <BigNumbers {txns} {accounts} {allTxns} {declared} />
                <ChartWidget {txns} {accounts} {allTxns} {depth} {declared} />
                <!-- Keyed on max: the slider can mount while txns are still loading (max=1),
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
