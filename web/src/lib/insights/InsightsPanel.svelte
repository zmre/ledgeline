<!-- Insights panel (WP-05): collapsible box at the top of the journal view.
     Receives the ALREADY-FILTERED transactions as a prop (the journal store /
     +page.svelte owns filtering — WP-03/04); everything here is $derived, so
     mode/interval/depth changes never refetch.
     Collapsed state persists in settings.insightsOpen; the header row stays
     visible with the period net even when collapsed. -->
<script lang="ts">
    import {formatAmount, toNumber} from "$lib/domain/money";
    import type {Transaction} from "$lib/domain/types";
    import {settings} from "$lib/stores/settings.svelte";
    import BigNumbers from "./BigNumbers.svelte";
    import ChartWidget from "./ChartWidget.svelte";
    import DepthSlider from "./DepthSlider.svelte";
    import {bigNumbers, commoditiesInUse, maxAccountDepth, styleFor, type AccountSelection} from "./series";

    // txns: the filtered view to summarize/chart. allTxns: the whole journal —
    // used only to detect the journal's sign conventions so they stay stable
    // across filter changes (see series.signConventions).
    let {txns, accounts, allTxns}: {txns: Transaction[]; accounts?: AccountSelection; allTxns?: Transaction[]} = $props();

    const primary = $derived(commoditiesInUse(txns, accounts)[0] ?? "$");
    const net = $derived(bigNumbers(txns, primary, accounts, allTxns).net);
    const netFormatted = $derived(formatAmount({commodity: primary, qty: net, style: styleFor(txns, primary)}));

    // Default depth matches the reports page (defaultReportParams().depth). The
    // slider is bound to this same `depth` the chart consumes, so the bar and the
    // chart never drift.
    let depth = $state(2);
    const max = $derived(maxAccountDepth(txns, accounts));
</script>

<section class="collapse-arrow bg-base-200 collapse" data-testid="insights-panel">
    <input
        type="checkbox"
        checked={settings.insightsOpen}
        onchange={(e) => (settings.insightsOpen = e.currentTarget.checked)}
        aria-label="Toggle insights panel"
    />
    <div class="collapse-title flex min-h-0 items-center justify-between gap-2 py-3 pr-10">
        <h2 class="text-sm font-semibold tracking-tight">Insights</h2>
        <span class="text-sm">
            <span class="text-base-content/60 mr-1">Net</span>
            <span class="font-semibold {toNumber(net) < 0 ? 'text-error' : 'text-success'}">{netFormatted}</span>
        </span>
    </div>
    <div class="collapse-content flex flex-col gap-4">
        <BigNumbers {txns} {accounts} {allTxns} />
        <ChartWidget {txns} {accounts} {allTxns} {depth} />
        <!-- Keyed on max: the slider can mount while txns are still loading (max=1),
             and the browser clamps the input's value to that max without updating the
             bound state; remounting once the real max arrives re-applies `depth`
             (same guard as reports/ui/ReportControls.svelte). -->
        {#key max}
            <DepthSlider bind:depth {max} />
        {/key}
    </div>
</section>
