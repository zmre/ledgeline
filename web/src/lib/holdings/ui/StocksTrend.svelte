<!-- The Stocks tab's value-over-time chart: HoldingsTrend plus the benchmark
     overlay.

     Progressive by construction. The portfolio line is drawn from `trend`,
     which the page already has; benchmark requests go out only once it is on
     screen (this component does not exist until then) and only for the ticked
     boxes. Each benchmark's line appears when its own response lands, and a
     failure is a hint in the picker — the base chart is never waiting on, or
     blanked by, Yahoo Finance.
     A reloaded `trend` (Refresh, Update prices, a journal edit) re-requests
     every line, since each is seeded from the portfolio's value.

     Lines are requested for `scope`'s window (`trendQuery`), the same one the
     page fetched `trend` for, and `benchmarkOverlays` drops any line whose
     dates do not match the chart's point for point — so a response that raced
     a scope change can never be drawn against the wrong dates. -->
<script lang="ts">
    import {untrack} from "svelte";
    import type {HoldingsScope, HoldingsSeries} from "$lib/holdings/types";
    import {benchmarkLines} from "$lib/stores/benchmarks.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import BenchmarkPicker from "./BenchmarkPicker.svelte";
    import HoldingsTrend from "./HoldingsTrend.svelte";
    import {benchmarkOverlays} from "./trendView";

    let {
        trend,
        scope,
        serverUrl,
        formatValue,
        formatAxis,
    }: {
        trend: HoldingsSeries;
        /** The scope `trend` was fetched for. */
        scope: HoldingsScope;
        serverUrl: string | null;
        formatValue: (n: number) => string;
        formatAxis?: (n: number) => string;
    } = $props();

    const picked = $derived(settings.benchmarks);

    // Re-sync when the chart, its window or the ticked set changes. `trend` is
    // the cache key: a new window's requests wait for that window's base line,
    // and a reloaded one (same window, fresh values) drops the held lines.
    $effect(() => {
        const url = serverUrl;
        const current = scope;
        const base = trend;
        const symbols = picked;
        if (url === null) return;
        untrack(() => benchmarkLines.sync(url, current, base, symbols));
    });

    const overlays = $derived(
        benchmarkOverlays(trend, picked, (symbol) => {
            const state = benchmarkLines.entries.get(symbol);
            return state?.status === "ready" ? state.line : null;
        })
    );
</script>

<HoldingsTrend {trend} period={scope.gainPeriod} {formatValue} {formatAxis} {overlays}>
    {#snippet controls()}
        <BenchmarkPicker
            {picked}
            states={benchmarkLines.entries}
            onToggle={(symbol, on) => settings.toggleBenchmark(symbol, on)}
            onRetry={(symbol) => {
                if (serverUrl !== null) benchmarkLines.retry(serverUrl, scope, trend, symbol);
            }}
        />
    {/snippet}
</HoldingsTrend>
