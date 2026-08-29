<!-- The insights comparison-period control: a preset dropdown plus explicit
     start/end date inputs. Binds `start`/`end` (the whole span); the engine
     splits it at its midpoint. Editing a date makes the preset "Custom". -->
<script lang="ts">
    import type {ISODate} from "$lib/domain/types";
    import {activeInsightsPreset, insightsPresetRange, INSIGHTS_PRESETS, type InsightsPreset} from "$lib/reports/ui/params";

    let {start = $bindable(), end = $bindable()}: {start: ISODate; end: ISODate} = $props();

    const activePreset = $derived(activeInsightsPreset(start, end));

    const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

    function onPreset(value: string): void {
        if (value === "custom") return;
        const range = insightsPresetRange(value as InsightsPreset);
        start = range.start;
        end = range.end;
    }
    function setStart(value: string): void {
        if (ISO_DATE.test(value)) start = value;
    }
    function setEnd(value: string): void {
        if (ISO_DATE.test(value)) end = value;
    }
</script>

<div class="flex flex-wrap items-end gap-x-4 gap-y-2 rounded-box bg-base-200 px-3 py-2">
    <label class="form-control">
        <span class="label-text mb-1 block text-xs text-base-content/70">Compare</span>
        <select class="select w-56 select-sm" value={activePreset} onchange={(e) => onPreset(e.currentTarget.value)} aria-label="Comparison preset">
            {#each INSIGHTS_PRESETS as preset (preset.id)}
                <option value={preset.id}>{preset.label}</option>
            {/each}
            {#if activePreset === "custom"}
                <option value="custom">Custom</option>
            {/if}
        </select>
    </label>
    <label class="form-control">
        <span class="label-text mb-1 block text-xs text-base-content/70">Start</span>
        <input type="date" class="input w-40 input-sm" value={start} onchange={(e) => setStart(e.currentTarget.value)} aria-label="Comparison start" />
    </label>
    <label class="form-control">
        <span class="label-text mb-1 block text-xs text-base-content/70">End</span>
        <input type="date" class="input w-40 input-sm" value={end} onchange={(e) => setEnd(e.currentTarget.value)} aria-label="Comparison end" />
    </label>
</div>
