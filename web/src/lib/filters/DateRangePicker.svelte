<script lang="ts">
    // Date range with quick presets (WP-04). Native date inputs for good
    // mobile UX; all range math goes through presetRange (pure, tested).
    import {filters, localToday, presetRange, type DatePreset} from "$lib/stores/filters.svelte";

    const presets: ReadonlyArray<{id: DatePreset; label: string}> = [
        {id: "thisMonth", label: "This month"},
        {id: "lastMonth", label: "Last month"},
        {id: "last90", label: "Last 90 days"},
        {id: "ytd", label: "Year to date"},
        {id: "thisYear", label: "This year"},
        {id: "lastYear", label: "Last year"},
        {id: "all", label: "All time"},
    ];

    const today = localToday();
    const activePreset = $derived(
        presets.find((p) => {
            const r = presetRange(p.id, today);
            return r.from === filters.value.from && r.to === filters.value.to;
        })
    );

    let dropdown: HTMLDetailsElement | undefined = $state();

    function applyPreset(p: DatePreset): void {
        filters.applyPreset(p);
        if (dropdown !== undefined) dropdown.open = false;
    }

    function setFrom(event: Event): void {
        const v = (event.currentTarget as HTMLInputElement).value;
        filters.setRange(v === "" ? null : v, filters.value.to);
    }

    function setTo(event: Event): void {
        const v = (event.currentTarget as HTMLInputElement).value;
        filters.setRange(filters.value.from, v === "" ? null : v);
    }
</script>

<div class="flex flex-wrap items-center gap-2">
    <details class="dropdown" bind:this={dropdown}>
        <summary class="btn btn-sm">
            {activePreset?.label ?? "Custom range"}
            <svg
                class="h-3 w-3 opacity-60"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                aria-hidden="true"
            >
                <path d="m6 9 6 6 6-6" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
        </summary>
        <ul class="menu dropdown-content bg-base-200 rounded-box z-20 mt-1 w-44 p-2 shadow-lg">
            {#each presets as preset (preset.id)}
                <li>
                    <button type="button" class={activePreset?.id === preset.id ? "menu-active" : ""} onclick={() => applyPreset(preset.id)}>
                        {preset.label}
                    </button>
                </li>
            {/each}
        </ul>
    </details>
    <div class="flex items-center gap-1">
        <input type="date" class="input input-sm w-36" value={filters.value.from ?? ""} onchange={setFrom} aria-label="From date" />
        <span class="text-base-content/60" aria-hidden="true">–</span>
        <input type="date" class="input input-sm w-36" value={filters.value.to ?? ""} onchange={setTo} aria-label="To date" />
    </div>
</div>
