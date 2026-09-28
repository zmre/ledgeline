<!-- The Stocks chart's benchmark checkboxes: a compact "Compare" dropdown in
     the chart's heading row, one checkbox per catalog benchmark.

     Each row carries its own state beside its label — a spinner while its
     history loads, a short error hint (and a retry) when it could not be had —
     so one unreachable index never hides the others' status. The button shows
     how many are ticked, so a closed dropdown still says the chart has more on
     it than the portfolio; the legend under the chart names them. -->
<script lang="ts">
    import {BENCHMARKS} from "$lib/holdings/benchmarks";
    import type {BenchmarkState} from "$lib/stores/benchmarks.svelte";

    let {
        picked,
        states,
        onToggle,
        onRetry,
    }: {
        /** Ticked symbols. */
        picked: readonly string[];
        /** Each requested benchmark's load state for the current window. */
        states: ReadonlyMap<string, BenchmarkState>;
        onToggle: (symbol: string, on: boolean) => void;
        onRetry: (symbol: string) => void;
    } = $props();

    const failing = $derived(picked.filter((symbol) => states.get(symbol)?.status === "error").length);
</script>

<details class="dropdown dropdown-end" data-testid="benchmark-picker">
    <summary class="btn gap-1 btn-ghost font-normal btn-xs" title="Compare against what the same money would have done in an index fund">
        Compare
        {#if picked.length > 0}<span class="badge badge-xs badge-neutral">{picked.length}</span>{/if}
        {#if failing > 0}<span class="text-warning" aria-label="{failing} comparison could not load">!</span>{/if}
    </summary>
    <div class="dropdown-content z-10 mt-1 w-72 rounded-box bg-base-200 p-2 shadow-lg">
        <p class="mb-1 px-1 text-xs text-base-content/60">Your contributions and withdrawals, on the same dates, into a fund instead. Dividends reinvested.</p>
        <ul class="flex flex-col">
            {#each BENCHMARKS as benchmark (benchmark.symbol)}
                {@const checked = picked.includes(benchmark.symbol)}
                {@const state = checked ? states.get(benchmark.symbol) : undefined}
                <li class="flex flex-col rounded px-1 py-0.5 hover:bg-base-300/50">
                    <label class="flex cursor-pointer items-center gap-2 text-sm">
                        <input
                            type="checkbox"
                            class="checkbox checkbox-xs"
                            {checked}
                            onchange={(e) => onToggle(benchmark.symbol, e.currentTarget.checked)}
                            data-testid="benchmark-{benchmark.symbol}"
                        />
                        <span class="grow">{benchmark.label}</span>
                        {#if state?.status === "loading"}
                            <span class="loading loading-xs loading-spinner text-base-content/50" aria-label="Loading {benchmark.label}"></span>
                        {:else if state?.status === "ready" && state.line.stale}
                            <span class="text-xs text-base-content/50" title="Couldn't refresh from Yahoo Finance; drawn from prices saved earlier">cached</span
                            >
                        {/if}
                    </label>
                    {#if state?.status === "error"}
                        <p class="ml-6 text-xs text-warning" data-testid="benchmark-{benchmark.symbol}-error">
                            {state.message}
                            <button type="button" class="link" onclick={() => onRetry(benchmark.symbol)}>Retry</button>
                        </p>
                    {/if}
                </li>
            {/each}
        </ul>
    </div>
</details>
