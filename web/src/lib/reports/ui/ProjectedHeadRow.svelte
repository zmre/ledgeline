<!-- The band above a statement's column headings that says which columns are
     actuals and which are projected (plans/28, Phase 5). Rendered only when the
     statement has forecast columns, so a Reports-tab table is unchanged.

     `leading` and `trailing` are the cells either side of the figure columns
     (the line-name column, and a Total), so the band lines up with the
     headings under it.

     `fixed`: the statement is on the shared fixed-layout column template
     (statementGrid.ts), where the line-name column's width comes from the
     colgroup. A `w-full` on this — often the table's FIRST row — would claim the
     whole table for it under `table-layout: fixed`. -->
<script lang="ts">
    import {projectedSpans} from "./projectedColumns";

    let {columns, projectedFrom, trailing = 0, fixed = false}: {columns: number; projectedFrom: number | null; trailing?: number; fixed?: boolean} = $props();

    const spans = $derived(projectedSpans(columns, projectedFrom));
</script>

{#if spans !== null}
    <tr class="text-[0.6rem] tracking-widest uppercase" data-testid="projected-band">
        <th class="{fixed ? '' : 'w-full'} font-normal"><span class="sr-only">Period</span></th>
        {#if spans.actual > 0}
            <th colspan={spans.actual} class="text-center font-normal text-base-content/40">Actual</th>
        {/if}
        <th colspan={spans.projected} class="border-l-2 border-l-primary/50 text-center font-semibold text-primary" data-testid="projected-label">Projected</th>
        {#if trailing > 0}
            <th colspan={trailing}></th>
        {/if}
    </tr>
{/if}
