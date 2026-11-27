<!-- The Stocks tab's value-over-time chart: HoldingsTrend plus the Value /
     Performance switch and the benchmark overlay.

     The switch (plan 47 A1b) is a two-button segmented control in the chart's
     heading row, each button tabbable and `aria-pressed`, and `v` flips it. It
     writes `holdingsChartView`, which the page's one URL writer mirrors as
     `?view=perf`. Both views come from the same series, so switching fetches
     nothing.

     Compare is Performance-only (A1d): benchmarks are cumulative returns, so
     they are drawn only beside the portfolio's own return. In Value mode no
     overlay is drawn and nothing is fetched, and pressing Compare switches to
     Performance and opens the picker.

     Progressive by construction. The portfolio line is drawn from `trend`,
     which the page already has; the benchmark request goes out only once it is
     on screen (this component does not exist until then), as ONE request for
     every ticked box, and adds all their lines when it lands. Ticking another
     box asks for just that one. A failure is a hint beside its own box in the
     picker — the base chart is never waiting on, or blanked by, Yahoo Finance.
     A reloaded `trend` (Refresh, Update prices, a journal edit) re-requests
     every line, since each is seeded from the portfolio's value.

     Lines are requested for `scope`'s window (`trendQuery`), the same one the
     page fetched `trend` for, and `benchmarkOverlays` drops any line whose
     dates do not match the chart's point for point — so a response that raced
     a scope change can never be drawn against the wrong dates. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import type {ChartView} from "#lib/holdings/params.js";
    import type {HoldingsScope, HoldingsSeries} from "#lib/holdings/types.js";
    import {benchmarkCatalog, benchmarkLines, catalogOrder} from "#lib/stores/benchmarks.svelte.js";
    import {holdingsChartView} from "#lib/stores/holdings.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import BenchmarkPicker from "./BenchmarkPicker.svelte";
    import HoldingsTrend from "./HoldingsTrend.svelte";
    import {benchmarkOverlays} from "./trendView";

    let {
        trend,
        scope,
        serverUrl,
        formatValue,
        formatAxis,
        formatFlow,
        liveHint = null,
        gain = null,
    }: {
        trend: HoldingsSeries;
        /** The scope `trend` was fetched for. */
        scope: HoldingsScope;
        serverUrl: string | null;
        formatValue: (n: number) => string;
        formatAxis?: (n: number) => string;
        formatFlow?: (n: number) => string;
        /** HoldingsTrend's `liveHint`. */
        liveHint?: string | null;
        /** HoldingsTrend's `gain`: the stat tiles' period gain, which colours the Value line. */
        gain?: number | null;
    } = $props();

    const view = $derived(holdingsChartView.value);
    const perf = $derived(view === "perf");
    const setView = (next: ChartView): void => {
        holdingsChartView.value = next;
    };

    registerKeys({
        id: "holdings-chart-view",
        priority: PRIORITY.page,
        bindings: [
            {
                keys: "v",
                label: "Chart value or performance",
                group: "Holdings",
                run: () => setView(holdingsChartView.value === "perf" ? "value" : "perf"),
            },
        ],
    });

    const VIEWS: readonly {view: ChartView; label: string; title: string}[] = [
        {view: "value", label: "Value", title: "Market value over time; ▲/▼ mark net buying and selling (v)"},
        {view: "perf", label: "Performance", title: "Return since the first point; buying and selling do not move it (v)"},
    ];

    // The engine owns the catalog; until it arrives nothing is requested, and a
    // persisted symbol it no longer lists is simply not asked for (the server
    // would refuse the whole batch over one unknown symbol).
    $effect(() => {
        if (serverUrl !== null) benchmarkCatalog.ensure(serverUrl);
    });
    const catalog = $derived(benchmarkCatalog.value ?? []);
    const picked = $derived(catalogOrder(benchmarkCatalog.value, settings.benchmarks));

    // Re-sync when the chart, its window or the ticked set changes. `trend` is
    // the cache key: a new window's requests wait for that window's base line,
    // and a reloaded one (same window, fresh values) drops the held lines.
    $effect(() => {
        const url = serverUrl;
        const current = scope;
        const base = trend;
        const symbols = picked;
        // Value mode draws no benchmark, so it asks Yahoo for none.
        if (url === null || !perf) return;
        untrack(() => benchmarkLines.sync(url, current, base, symbols));
    });

    const overlays = $derived(
        !perf
            ? []
            : benchmarkOverlays(trend, picked, (symbol) => {
                  const state = benchmarkLines.entries.get(symbol);
                  return state?.status === "ready" ? state.line : null;
              })
    );
</script>

<HoldingsTrend {trend} period={scope.gainPeriod} {formatValue} {formatAxis} {formatFlow} {overlays} {liveHint} {view} {gain}>
    {#snippet controls()}
        <div class="flex items-center gap-2">
            <div class="join" role="group" aria-label="Chart shows" data-testid="chart-view">
                {#each VIEWS as v (v.view)}
                    <button
                        type="button"
                        class="btn join-item btn-xs {view === v.view ? 'btn-neutral' : 'btn-ghost'}"
                        aria-pressed={view === v.view}
                        title={v.title}
                        onclick={() => setView(v.view)}
                        data-testid="chart-view-{v.view}"
                    >
                        {v.label}
                    </button>
                {/each}
            </div>
            <BenchmarkPicker
                active={perf}
                onActivate={() => setView("perf")}
                {catalog}
                {picked}
                states={benchmarkLines.entries}
                onToggle={(symbol, on) => settings.toggleBenchmark(symbol, on)}
                onRetry={(symbol) => {
                    if (serverUrl !== null) benchmarkLines.retry(serverUrl, scope, trend, symbol);
                }}
            />
        </div>
    {/snippet}
</HoldingsTrend>
