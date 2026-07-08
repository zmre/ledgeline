<!-- Account-depth slider (WP-05): daisyUI range with tick labels, 1..max. -->
<script lang="ts">
    let {depth = $bindable(), max}: {depth: number; max: number} = $props();

    const ticks = $derived(Array.from({length: Math.max(1, max)}, (_, i) => i + 1));
</script>

<label class="block w-full max-w-xs">
    <span class="label-text text-base-content/70 mb-1 block text-xs">Account depth</span>
    <input type="range" min="1" max={Math.max(1, max)} step="1" bind:value={depth} class="range range-primary range-xs w-full" aria-label="Account depth" />
    <div class="text-base-content/60 mt-1 flex justify-between px-0.5 font-mono text-[0.65rem]" aria-hidden="true">
        {#each ticks as t (t)}
            <span class={t === depth ? "text-base-content font-bold" : ""}>{t}</span>
        {/each}
    </div>
</label>
