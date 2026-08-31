<!-- Chart widget (WP-05): LayerChart pie/line for the filtered period.
     - mode toggle (pie | line), interval select (line only), commodity select when >1 in use
     - one commodity at a time; never sums across commodities
     - colors: the shared categorical palette ($lib/format/palette) + muted gray for
       "(other)". That module documents the validator run and the slot order.
       Secondary encoding, required by the skill at this CVD separation, is the
       always-on legend, pad-angle gaps between pie slices, and full tooltips.
     - one shared magnitude ranking (series.rankedAccounts) drives the colours AND
       whichever dataset is on screen, so an account keeps its hue across modes and
       only the active mode is computed; capped at 6 groups incl. "(other)". -->
<script lang="ts">
    import {LineChart, PieChart, Tooltip} from "layerchart";
    import type {RootCategory} from "$lib/domain/accounts";
    import type {Transaction} from "$lib/domain/types";
    import {colorAt, OTHER_COLOR} from "$lib/format/palette";
    import {
        categoriesInUse,
        commoditiesInUse,
        formatChartValue,
        formatCompactChartValue,
        groupOrder,
        lineData,
        pieData,
        rankedAccounts,
        styleFor,
        OTHER,
        type AccountSelection,
        type DeclaredTypes,
        type Interval,
        type PieDatum,
    } from "./series";

    let {
        txns,
        depth,
        accounts,
        allTxns,
        declared,
    }: {txns: Transaction[]; depth: number; accounts?: AccountSelection; allTxns?: Transaction[]; declared?: DeclaredTypes} = $props();

    const MAX_GROUPS = 6;

    // Human labels for the category scope selector (root account groups).
    const GROUP_LABELS: Record<RootCategory, string> = {
        expense: "Expenses",
        revenue: "Income",
        asset: "Assets",
        liability: "Liabilities",
        equity: "Equity",
        other: "Other",
    };

    let mode = $state<"pie" | "line">("pie");
    let interval = $state<Interval>("monthly");
    let chosenCommodity = $state<string | null>(null);
    // Category scope: null = follow the default, which prefers "expenses" (the
    // most useful journal view) when present. "all" shows every category.
    let chosenGroup = $state<RootCategory | "all" | null>(null);

    const commodities = $derived(commoditiesInUse(txns, accounts));
    const commodity = $derived(chosenCommodity !== null && commodities.includes(chosenCommodity) ? chosenCommodity : (commodities[0] ?? "$"));
    const style = $derived(styleFor(txns, commodity));

    const groups = $derived(categoriesInUse(txns, commodity, accounts, declared));
    // Resolve the active scope: honor an explicit pick that's still available,
    // otherwise default to expenses when present (else all categories).
    const group = $derived.by<RootCategory | "all">(() => {
        if (chosenGroup !== null && (chosenGroup === "all" || groups.includes(chosenGroup))) return chosenGroup;
        return groups.includes("expense") ? "expense" : "all";
    });
    const category = $derived<RootCategory | undefined>(group === "all" ? undefined : group);

    // The magnitude ranking, computed ONCE and shared three ways: it decides the
    // colour slots, and it is handed to whichever of pieData/lineData actually
    // runs so neither repeats the pass. At 150k transactions this scan is 49 ms
    // and each dataset is ~110 ms, so the old shape — both datasets, each
    // ranking again — was 221 ms per filter change to draw one chart.
    const ranked = $derived(rankedAccounts(txns, depth, commodity, accounts, category, declared));

    // Only the mode on screen is computed. `{#if mode === "pie"}` below renders
    // one or the other, and daisyUI does not hide the loser — nothing did, so
    // both were being built regardless of which was visible.
    const pie = $derived(
        mode === "pie" ? pieData(txns, {depth, commodity, maxSlices: MAX_GROUPS, accounts, conventionTxns: allTxns, category, declared, ranked}) : []
    );

    // A pie encodes parts of a whole by AREA, and a negative part has none.
    // Drawing |value| (the old behaviour) turned a −$500 travel refund into a
    // positive wedge worth a fifth of a $2,000 rent pie, and inflated the whole
    // from the true $1,500 net to $2,500 — only the tooltip carried the sign.
    // Netting is not an option either: pieData already nets per account group,
    // so a negative datum IS that category's net credit for the period and has
    // nothing left to net into.
    //
    // So the pie draws the positive parts, whose areas do sum to the whole it
    // claims to partition, and the credits are named underneath with their
    // signed amounts rather than silently redrawn as spending.
    const pieSlices = $derived(pie.filter((d) => d.value > 0));
    const pieCredits = $derived(pie.filter((d) => d.value < 0));
    const line = $derived(
        mode === "line"
            ? lineData(txns, {depth, commodity, interval, maxSeries: MAX_GROUPS, accounts, conventionTxns: allTxns, category, declared, ranked})
            : []
    );

    // Color follows the account, not the mode — now by construction rather than
    // by coincidence. Slots are assigned from the shared RANKING, which is the
    // same list `pieData` and `lineData` each order their output by, so an
    // account keeps its hue when you toggle pie/line even if one mode drops it
    // (a pie omits accounts that net to zero; a line omits accounts with no
    // buckets). This used to iterate the line dataset and then the pie dataset,
    // which needed both to exist.
    //
    // `colorAt` FOLDS past the last slot rather than wrapping; the ranking is
    // capped at MAX_GROUPS so `slot` cannot now exceed 6, but folding is still
    // the right behaviour and this used to be `PALETTE[slot++ %
    // PALETTE.length]`, which handed a 7th account slot 1's blue and made it
    // indistinguishable from the 1st.
    const colorOf: Record<string, string> = $derived.by(() => {
        const colors: Record<string, string> = {[OTHER]: OTHER_COLOR};
        let slot = 0;
        for (const account of groupOrder(ranked, MAX_GROUPS)) {
            colors[account] ??= colorAt(slot++);
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
            <select class="select w-28 select-xs" bind:value={interval} aria-label="Interval">
                <option value="daily">Daily</option>
                <option value="weekly">Weekly</option>
                <option value="monthly">Monthly</option>
            </select>
        {/if}
        {#if commodities.length > 1}
            <select class="select w-24 select-xs" value={commodity} onchange={(e) => (chosenCommodity = e.currentTarget.value)} aria-label="Commodity">
                {#each commodities as c (c)}
                    <option value={c}>{c}</option>
                {/each}
            </select>
        {/if}
        {#if groups.length > 1}
            <select
                class="select w-32 select-xs"
                value={group}
                onchange={(e) => (chosenGroup = e.currentTarget.value as RootCategory | "all")}
                aria-label="Category"
            >
                <option value="all">All categories</option>
                {#each groups as g (g)}
                    <option value={g}>{GROUP_LABELS[g]}</option>
                {/each}
            </select>
        {/if}
    </div>

    {#if mode === "pie"}
        {#if pieSlices.length === 0}
            <p class="py-10 text-center text-sm text-base-content/60">
                {#if pieCredits.length === 0}
                    No {commodity} activity in the filtered period.
                {:else}
                    Every {commodity} category nets to a credit in this period, so there is nothing for a pie to divide.
                {/if}
            </p>
        {:else}
            <div class="h-64 w-full sm:h-72" data-testid="insights-pie">
                <PieChart
                    data={pieSlices}
                    key="account"
                    label="account"
                    value={(d) => d.value}
                    cRange={pieSlices.map((d) => colorOf[d.account] ?? OTHER_COLOR)}
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
            <ul class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-base-content/70 sm:hidden">
                {#each pieSlices as d (d.account)}
                    <li class="flex items-center gap-1">
                        <span class="inline-block h-2 w-2 rounded-full" style="background:{colorOf[d.account] ?? OTHER_COLOR}"></span>
                        {d.account}
                    </li>
                {/each}
            </ul>
        {/if}
        <!-- Categories that net to a credit have no area in a pie; name them instead of drawing them positive. -->
        {#if pieCredits.length > 0}
            <p class="mt-1 px-1 text-xs text-base-content/60" data-testid="insights-pie-credits">
                Not shown (net credit in this period): {pieCredits.map((d) => `${d.account} ${d.formatted}`).join(", ")}.
            </p>
        {/if}
    {:else if rows.length === 0}
        <p class="py-10 text-center text-sm text-base-content/60">No {commodity} activity in the filtered period.</p>
    {:else}
        <div class="h-64 w-full sm:h-72" data-testid="insights-line">
            <LineChart
                data={rows}
                x={(d) => d.i}
                series={lineSeries}
                legend
                brush={false}
                points={rows.length <= 31}
                padding={{top: 8, right: 8, bottom: 56, left: 56}}
                props={{
                    xAxis: {format: bucketLabel, ticks: xTicks},
                    yAxis: {format: (v: number) => formatCompactChartValue(v, commodity, style)},
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
