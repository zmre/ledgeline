<!-- The shared column template of a multi-column statement (see statementGrid.ts):
     a line-name column that takes the leftover width, one fixed column per
     period, and an optional Total. Every table in the stack renders this same
     colgroup under `table-fixed`, which is what makes their columns line up. -->
<script lang="ts">
    import {COMPARE_PERIOD_REM, DELTA_PCT_REM, DELTA_REM, PERIOD_REM, TOTAL_REM} from "./statementGrid";

    /** `compare`: the Δ and Δ% columns of a projection comparison (plan 34), in place of a Total. */
    /** `budget`: the P&L's Budget column after the Total (plan 50 D12). */
    let {periods, total, compare = false, budget = false}: {periods: number; total: boolean; compare?: boolean; budget?: boolean} = $props();
</script>

<colgroup>
    <col />
    {#each {length: periods}, i (i)}
        <col style="width: {compare ? COMPARE_PERIOD_REM : PERIOD_REM}rem" />
    {/each}
    {#if total}
        <col style="width: {TOTAL_REM}rem" />
    {/if}
    {#if budget}
        <col style="width: {TOTAL_REM}rem" />
    {/if}
    {#if compare}
        <col style="width: {DELTA_REM}rem" />
        <col style="width: {DELTA_PCT_REM}rem" />
    {/if}
</colgroup>
