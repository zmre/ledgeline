<!-- Holdings value-over-time (post-MVP): portfolio market value at each of the
     last 12 month-ends for the current scope, from the native /api/holdings/series
     endpoint (decoded into HoldingsSeries). One series → no legend box; the heading names it
     (dataviz single-series rule). Color is the dataviz dark palette slot 1
     (#3987e5), already validated against the daisyUI dark surface for the pie/line
     charts. x is the bucket index for even spacing (string month labels via the
     axis formatter); numeric y/tooltip go through the base commodity's display
     style. Basis is intentionally not overlaid yet — it's null whenever any held
     lot is tainted/unpriced (honest-totals rule), so it would be blank for most
     real portfolios; a dashed, labeled basis line is the obvious follow-up. -->
<script lang="ts">
    import {LineChart} from "layerchart";
    import {toNumber} from "$lib/domain/money";
    import type {HoldingsSeries} from "$lib/holdings/types";

    let {trend, formatValue}: {trend: HoldingsSeries; formatValue: (n: number) => string} = $props();

    const VALUE_COLOR = "#3987e5"; // dataviz dark palette slot 1 (validated against the daisyUI dark surface — see HoldingsPie)

    interface Row {
        i: number;
        label: string;
        value: number;
    }
    const rows = $derived<Row[]>(trend.points.map((p, i) => ({i, label: p.label, value: toNumber(p.marketValue)})));
    const allZero = $derived(rows.every((r) => r.value === 0));

    // Explicit integer ticks so index-based x labels never land between buckets.
    const xTicks = $derived.by(() => {
        const step = Math.max(1, Math.ceil(rows.length / 6));
        return rows.filter((r) => r.i % step === 0 || r.i === rows.length - 1).map((r) => r.i);
    });
    const labelOf = (i: unknown): string => rows[Math.round(Number(i))]?.label ?? "";
</script>

<div class="w-full">
    <h3 class="text-base-content/70 mb-1 text-xs font-semibold tracking-tight">
        Value over time <span class="text-base-content/40 font-normal">· last 12 months</span>
    </h3>
    {#if allZero}
        <p class="text-base-content/60 py-8 text-center text-sm">No priced holdings in the last 12 months.</p>
    {:else}
        <div class="h-56 w-full sm:h-64" data-testid="holdings-trend">
            <LineChart
                data={rows}
                x={(d) => d.i}
                series={[{key: "value", label: "Market value", color: VALUE_COLOR, value: (d: Row) => d.value}]}
                points={rows.length <= 31}
                brush={false}
                props={{
                    xAxis: {format: labelOf, ticks: xTicks},
                    yAxis: {format: formatValue},
                    spline: {class: "stroke-2"},
                    tooltip: {header: {format: labelOf}, item: {format: formatValue}},
                }}
            />
        </div>
    {/if}
</div>
