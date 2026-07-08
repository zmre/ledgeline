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

    let {txns, accounts}: {txns: Transaction[]; accounts?: AccountSelection} = $props();

    const primary = $derived(commoditiesInUse(txns, accounts)[0] ?? "$");
    const net = $derived(bigNumbers(txns, primary, accounts).net);
    const netFormatted = $derived(formatAmount({commodity: primary, qty: net, style: styleFor(txns, primary)}));

    let depth = $state(2);
    const max = $derived(maxAccountDepth(txns, accounts));
    const clampedDepth = $derived(Math.min(depth, max));
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
        <BigNumbers {txns} {accounts} />
        <ChartWidget {txns} {accounts} depth={clampedDepth} />
        <DepthSlider bind:depth {max} />
    </div>
</section>
