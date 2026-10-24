<!-- The band above a statement's column headings that says which columns are
     actuals and which are projected (plans/28, Phase 5). Rendered only when the
     statement has forecast columns, so a Reports-tab table is unchanged.

     `leading` and `trailing` are the cells either side of the figure columns
     (the line-name column, and a Total), so the band lines up with the
     headings under it. -->
<script lang="ts">
    import {projectedSpans} from "./projectedColumns";

    let {columns, projectedFrom, trailing = 0}: {columns: number; projectedFrom: number | null; trailing?: number} = $props();

    const spans = $derived(projectedSpans(columns, projectedFrom));
</script>

{#if spans !== null}
    <tr class="text-[0.6rem] tracking-widest uppercase" data-testid="projected-band">
        <th class="w-full font-normal"><span class="sr-only">Period</span></th>
        {#if spans.actual > 0}
            <th colspan={spans.actual} class="text-center font-normal text-base-content/40">Actual</th>
        {/if}
        <th colspan={spans.projected} class="border-l-2 border-l-primary/50 text-center font-semibold text-primary" data-testid="projected-label">Projected</th>
        {#if trailing > 0}
            <th colspan={trailing}></th>
        {/if}
    </tr>
{/if}
