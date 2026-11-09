<!-- A stand-in for `#lib/components/PeriodLineChart` that renders each bucket
     as a button calling `onPeriodClick` — for testing what an adapter does with
     a click, which jsdom cannot drive through layerchart's tooltip layer (with
     no layout, its pointer search finds no bucket). Mock the chart with this
     in a test file of its own: `vi.mock` replaces it for the whole file. -->
<script lang="ts">
    let {labels, onPeriodClick}: {labels: readonly string[]; onPeriodClick?: (index: number) => void} = $props();
</script>

{#each labels as label, i (i)}
    <button type="button" data-period={i} onclick={() => onPeriodClick?.(i)}>{label}</button>
{/each}
