<script lang="ts">
    // Date range with quick presets (WP-04). Native date inputs for good
    // mobile UX; all range math goes through presetRange (pure, tested).
    import type {ISODate} from "$lib/domain/types";
    import {filters, type DatePreset} from "$lib/stores/filters.svelte";

    const presets: ReadonlyArray<{id: DatePreset; label: string}> = [
        {id: "thisMonth", label: "This month"},
        {id: "lastMonth", label: "Last month"},
        {id: "last90", label: "Last 90 days"},
        {id: "ytd", label: "Year to date"},
        {id: "thisYear", label: "This year"},
        {id: "lastYear", label: "Last year"},
        {id: "all", label: "All time"},
    ];

    // The store tracks which preset produced the range (null = hand-picked).
    const activePreset = $derived(presets.find((p) => p.id === (filters.value.preset ?? null)));

    let dropdown: HTMLDetailsElement | undefined = $state();

    /**
     * Coalescing window for the two typed date fields.
     *
     * A native `<input type="date">` fires `change` for every INTERMEDIATE valid
     * date, and typing a year does that four times: `0002-…`, `0020-…`,
     * `0202-…`, `2026-…`. Undebounced, the first three each re-derived the whole
     * journal view over a two-thousand-year range before the user had finished
     * the field — three full recomputes (279 ms each at 150k transactions in
     * node) that nobody asked for and nobody sees.
     *
     * 250 ms rather than the search box's 300: a date field emits a handful of
     * events, not a stream, and committing should feel prompt when the value
     * came from the native picker (one event, so the whole delay is visible).
     * The input keeps its own typed text throughout — `value=` is only written
     * back when the store changes — so nothing the user is looking at waits.
     */
    const RANGE_DEBOUNCE_MS = 250;

    let timer: ReturnType<typeof setTimeout> | null = null;
    /** The range the user has typed but that has not been committed yet, or null. */
    let queued: {from: ISODate | null; to: ISODate | null} | null = null;

    function cancelQueued(): void {
        if (timer !== null) clearTimeout(timer);
        timer = null;
        queued = null;
    }

    function commitQueued(): void {
        timer = null;
        if (queued === null) return;
        const {from, to} = queued;
        queued = null;
        filters.setRange(from, to);
    }

    /**
     * Queue a range edit. Both ends travel together and BOTH read from `queued`
     * first: tabbing from one field to the other inside the window would
     * otherwise have the second edit overwrite the first with the store's stale
     * value.
     */
    function queueRange(from: ISODate | null, to: ISODate | null): void {
        queued = {from, to};
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(commitQueued, RANGE_DEBOUNCE_MS);
    }

    function applyPreset(p: DatePreset): void {
        // A preset is a deliberate single click, so it applies at once — and it
        // must DROP any queued typing, or a half-typed range would land on top
        // of the preset a quarter-second after it was chosen.
        cancelQueued();
        filters.applyPreset(p);
        if (dropdown !== undefined) dropdown.open = false;
    }

    function setFrom(event: Event): void {
        const v = (event.currentTarget as HTMLInputElement).value;
        queueRange(v === "" ? null : v, queued?.to ?? filters.value.to);
    }

    function setTo(event: Event): void {
        const v = (event.currentTarget as HTMLInputElement).value;
        queueRange(queued?.from ?? filters.value.from, v === "" ? null : v);
    }

    // Leaving the page with an edit still queued must not silently discard it:
    // the user typed a date and saw the field keep it. Flush instead of drop.
    $effect(() => () => {
        if (timer !== null) {
            clearTimeout(timer);
            commitQueued();
        }
    });
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
        <ul class="menu dropdown-content z-20 mt-1 w-44 rounded-box bg-base-200 p-2 shadow-lg">
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
        <input type="date" class="input w-36 input-sm" value={filters.value.from ?? ""} onchange={setFrom} aria-label="From date" />
        <span class="text-base-content/60" aria-hidden="true">–</span>
        <input type="date" class="input w-36 input-sm" value={filters.value.to ?? ""} onchange={setTo} aria-label="To date" />
    </div>
</div>
