<!-- Per-report controls (WP-07), driven by the active tab's ControlsConfig.
     Mutates fields of the bound params object; date edits are ignored until
     the input holds a full valid ISO date (type=date emits "" while clearing). -->
<script lang="ts">
    import DepthSlider from "$lib/insights/DepthSlider.svelte";
    import {MAX_COUNT, TAB_CONTROLS, type ReportParams} from "./params";

    let {params = $bindable(), maxDepth}: {params: ReportParams; maxDepth: number} = $props();

    const config = $derived(TAB_CONTROLS[params.tab]);

    const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
    function setDate(key: "asOf" | "from" | "to" | "end", value: string): void {
        if (ISO_DATE.test(value)) params[key] = value;
    }
    function setCount(value: string): void {
        const n = Number(value);
        if (Number.isInteger(n)) params.count = Math.min(MAX_COUNT, Math.max(1, n));
    }
</script>

{#snippet dateField(label: string, key: "asOf" | "from" | "to" | "end")}
    <label class="form-control">
        <span class="label-text mb-1 block text-xs text-base-content/70">{label}</span>
        <input type="date" class="input w-40 input-sm" value={params[key]} onchange={(e) => setDate(key, e.currentTarget.value)} aria-label={label} />
    </label>
{/snippet}

<div class="flex flex-wrap items-end gap-x-4 gap-y-2 rounded-box bg-base-200 px-3 py-2">
    {#if config.asOf}
        {@render dateField("As of", "asOf")}
    {/if}
    {#if config.range}
        {@render dateField("From", "from")}
        {@render dateField("To", "to")}
    {/if}
    {#if config.end}
        {@render dateField("End", "end")}
    {/if}
    {#if config.interval}
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Interval</span>
            <select class="select w-32 select-sm" bind:value={params.interval} aria-label="Interval">
                <option value="monthly">Monthly</option>
                <option value="quarterly">Quarterly</option>
                <option value="yearly">Yearly</option>
            </select>
        </label>
    {/if}
    {#if config.count}
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Periods</span>
            <input
                type="number"
                class="input w-20 input-sm"
                min="1"
                max={MAX_COUNT}
                value={params.count}
                onchange={(e) => setCount(e.currentTarget.value)}
                aria-label="Number of periods"
            />
        </label>
    {/if}
    {#if config.depth}
        <!-- Keyed on maxDepth: the slider can mount while accounts are still loading (max=1),
             and the browser clamps the input's value to that max without updating the bound
             state; remounting once the real max arrives re-applies the bound depth. -->
        {#key maxDepth}
            <DepthSlider bind:depth={params.depth} max={maxDepth} />
        {/key}
    {/if}
</div>
