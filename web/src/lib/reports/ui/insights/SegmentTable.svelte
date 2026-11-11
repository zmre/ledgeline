<!-- MRR by dimension value (plans/37): one row per value of the chosen key,
     straight from the engine's `segments` (MRR desc, `(unassigned)` last), with
     MRR at the current half's month, its change from the previous half's,
     customers, ARPA and NRR. A picker appears when more than one key is
     offered. -->
<script lang="ts">
    import InfoTip from "#lib/components/InfoTip.svelte";
    import {sub} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {sentimentClass} from "#lib/format/sign.js";
    import type {SegmentBreakdown} from "#lib/reports/saasTypes.js";
    import {fmt, fmtPct, fmtSignedAmount} from "./format";
    import {METRIC_HELP} from "./metricHelp";

    let {
        segments,
        base,
        styles,
        testid = "startup-segments",
    }: {segments: SegmentBreakdown[]; base: string; styles: ReadonlyMap<string, AmountStyle>; testid?: string} = $props();

    // The first key until the user picks another; a key the next span no longer
    // offers falls back to the first rather than rendering nothing.
    let picked = $state<string | null>(null);
    const shown = $derived(segments.find((segment) => segment.key === picked) ?? segments[0]);
</script>

{#if shown !== undefined}
    <div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
        <div class="card-body gap-2 p-4">
            <div class="flex flex-wrap items-center justify-between gap-2">
                <span class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">MRR by {shown.key}</span>
                {#if segments.length > 1}
                    <select
                        class="select-bordered select select-xs"
                        aria-label="Break down by"
                        value={shown.key}
                        onchange={(e) => (picked = e.currentTarget.value)}
                        data-testid="{testid}-key"
                    >
                        {#each segments as segment (segment.key)}
                            <option value={segment.key}>{segment.key}</option>
                        {/each}
                    </select>
                {/if}
            </div>
            <div class="overflow-x-auto">
                <table class="table table-sm">
                    <thead>
                        <tr>
                            <th>{shown.key}</th>
                            <th class="text-right">MRR</th>
                            <th class="text-right">Δ</th>
                            <th class="text-right">Customers</th>
                            <th class="text-right">
                                <span class="inline-flex items-center gap-1">ARPA<InfoTip {...METRIC_HELP["segment-arpa"]} placement="bottom-end" /></span>
                            </th>
                            <th class="text-right">
                                <span class="inline-flex items-center gap-1">NRR<InfoTip {...METRIC_HELP["segment-nrr"]} placement="bottom-end" /></span>
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {#each shown.rows as row (row.value)}
                            {@const change = sub(row.mrrCurr, row.mrrPrev)}
                            <tr data-testid="{testid}-row">
                                <td class={row.value === "(unassigned)" ? "text-base-content/60 italic" : ""}>{row.value}</td>
                                <td class="text-right font-mono tabular-nums">{fmt(base, row.mrrCurr, styles)}</td>
                                <td class="text-right font-mono tabular-nums {sentimentClass(change.m, true)}">{fmtSignedAmount(change, base, styles)}</td>
                                <td class="text-right tabular-nums">{row.customersCurr ?? "—"}</td>
                                <td class="text-right font-mono tabular-nums">{row.arpa === null ? "—" : fmt(base, row.arpa, styles)}</td>
                                <td class="text-right tabular-nums">{fmtPct(row.nrrPct)}</td>
                            </tr>
                        {/each}
                    </tbody>
                </table>
            </div>
        </div>
    </div>
{/if}
