<!-- Chart widget (WP-05): LayerChart pie/line for the filtered period.
     - mode toggle (pie | line), interval select (line only), commodity select when >1 in use
     - one commodity at a time; never sums across commodities
     - colors: dataviz reference dark palette slots 1..6 + muted gray for "(other)",
       validated with the dataviz skill validator against the daisyUI dark surface
       (#191e24): lightness band PASS, chroma PASS, contrast >=3:1 PASS, worst adjacent
       CVD dE 10.3 (floor band) — mitigated per the skill by an always-on legend,
       pad-angle gaps between pie slices, and full tooltips.
     - pie and line rank accounts identically (series.rankedAccounts), so an account
       keeps its hue across modes; slices/series are capped at 6 groups incl. "(other)". -->
<script lang="ts">
    import {LineChart, PieChart, Tooltip} from "layerchart";
    import type {Transaction} from "$lib/domain/types";
    import {commoditiesInUse, formatChartValue, lineData, pieData, styleFor, OTHER, type AccountSelection, type Interval, type PieDatum} from "./series";

    let {txns, depth, accounts, allTxns}: {txns: Transaction[]; depth: number; accounts?: AccountSelection; allTxns?: Transaction[]} = $props();

    // Dark-mode categorical slots 1..6 from the dataviz reference palette (app theme is dark-only).
    const PALETTE = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767"];
    const OTHER_COLOR = "#898781"; // muted — the folded tail is context, not a series identity
    const MAX_GROUPS = 6;

    let mode = $state<"pie" | "line">("pie");
    let interval = $state<Interval>("monthly");
    let chosenCommodity = $state<string | null>(null);

    const commodities = $derived(commoditiesInUse(txns, accounts));
    const commodity = $derived(chosenCommodity !== null && commodities.includes(chosenCommodity) ? chosenCommodity : (commodities[0] ?? "$"));
    const style = $derived(styleFor(txns, commodity));

    const pie = $derived(pieData(txns, {depth, commodity, maxSlices: MAX_GROUPS, accounts, conventionTxns: allTxns}));
    const line = $derived(lineData(txns, {depth, commodity, interval, maxSeries: MAX_GROUPS, accounts, conventionTxns: allTxns}));

    // Color follows the account, not the mode: both datasets come from the same
    // magnitude ranking, so slot assignment by first appearance stays consistent.
    const colorOf: Record<string, string> = $derived.by(() => {
        const colors: Record<string, string> = {[OTHER]: OTHER_COLOR};
        let slot = 0;
        for (const s of line) {
            colors[s.account] ??= PALETTE[slot++ % PALETTE.length];
        }
        for (const d of pie) {
            colors[d.account] ??= PALETTE[slot++ % PALETTE.length];
        }
        return colors;
    });

    // Line chart rows: one row per bucket, x is the bucket index (string buckets,
    // even spacing); every series is zero-filled to the same bucket list.
    interface Row {
        i: number;
        bucket: string;
        values: Record<string, number>;
    }
    const rows: Row[] = $derived.by(() => {
        if (line.length === 0) return [];
        return line[0].points.map((p, i) => {
            const values: Record<string, number> = {};
            for (const s of line) values[s.account] = s.points[i]?.value ?? 0;
            return {i, bucket: p.bucket, values};
        });
    });
    const lineSeries = $derived(
        line.map((s) => ({
            key: s.account,
            label: s.account,
            color: colorOf[s.account] ?? OTHER_COLOR,
            value: (d: Row) => d.values[s.account] ?? 0,
        }))
    );
    // Explicit integer ticks so index-based x labels never land between buckets.
    const xTicks = $derived.by(() => {
        const step = Math.max(1, Math.ceil(rows.length / 6));
        return rows.filter((r) => r.i % step === 0 || r.i === rows.length - 1).map((r) => r.i);
    });
    const bucketLabel = (i: unknown): string => rows[Math.round(Number(i))]?.bucket ?? "";
</script>

<div class="w-full">
    <div class="mb-2 flex flex-wrap items-center gap-2">
        <div class="join" role="group" aria-label="Chart mode">
            <button
                type="button"
                class="btn join-item btn-xs {mode === 'pie' ? 'btn-active' : ''}"
                aria-pressed={mode === "pie"}
                onclick={() => (mode = "pie")}
            >
                Pie
            </button>
            <button
                type="button"
                class="btn join-item btn-xs {mode === 'line' ? 'btn-active' : ''}"
                aria-pressed={mode === "line"}
                onclick={() => (mode = "line")}
            >
                Line
            </button>
        </div>
        {#if mode === "line"}
            <select class="select select-xs w-28" bind:value={interval} aria-label="Interval">
                <option value="daily">Daily</option>
                <option value="weekly">Weekly</option>
                <option value="monthly">Monthly</option>
            </select>
        {/if}
        {#if commodities.length > 1}
            <select class="select select-xs w-24" value={commodity} onchange={(e) => (chosenCommodity = e.currentTarget.value)} aria-label="Commodity">
                {#each commodities as c (c)}
                    <option value={c}>{c}</option>
                {/each}
            </select>
        {/if}
    </div>

    {#if mode === "pie"}
        {#if pie.length === 0}
            <p class="text-base-content/60 py-10 text-center text-sm">No {commodity} activity in the filtered period.</p>
        {:else}
            <div class="h-64 w-full sm:h-72" data-testid="insights-pie">
                <PieChart
                    data={pie}
                    key="account"
                    label="account"
                    value={(d) => Math.abs(d.value)}
                    cRange={pie.map((d) => colorOf[d.account] ?? OTHER_COLOR)}
                    padAngle={0.02}
                    legend={{placement: "right", orientation: "vertical", classes: {root: "hidden sm:block"}}}
                >
                    {#snippet tooltip()}
                        <Tooltip.Root>
                            {#snippet children({data})}
                                {@const d = data as PieDatum}
                                <div class="flex items-center gap-2 text-xs">
                                    <span class="inline-block h-2 w-2 rounded-full" style="background:{colorOf[d.account] ?? OTHER_COLOR}"></span>
                                    <span class="text-base-content/70">{d.account}</span>
                                    <span class="font-semibold">{d.formatted}</span>
                                </div>
                            {/snippet}
                        </Tooltip.Root>
                    {/snippet}
                </PieChart>
            </div>
            <!-- legend fallback for narrow screens (identity is never color-alone) -->
            <ul class="text-base-content/70 mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs sm:hidden">
                {#each pie as d (d.account)}
                    <li class="flex items-center gap-1">
                        <span class="inline-block h-2 w-2 rounded-full" style="background:{colorOf[d.account] ?? OTHER_COLOR}"></span>
                        {d.account}
                    </li>
                {/each}
            </ul>
        {/if}
    {:else if rows.length === 0}
        <p class="text-base-content/60 py-10 text-center text-sm">No {commodity} activity in the filtered period.</p>
    {:else}
        <div class="h-64 w-full sm:h-72" data-testid="insights-line">
            <LineChart
                data={rows}
                x={(d) => d.i}
                series={lineSeries}
                legend
                brush={false}
                points={rows.length <= 31}
                props={{
                    xAxis: {format: bucketLabel, ticks: xTicks},
                    yAxis: {format: (v: number) => formatChartValue(v, commodity, style)},
                    spline: {class: "stroke-2"},
                    tooltip: {
                        header: {format: bucketLabel},
                        item: {format: (v: number) => formatChartValue(v, commodity, style)},
                    },
                }}
            />
        </div>
    {/if}
</div>
