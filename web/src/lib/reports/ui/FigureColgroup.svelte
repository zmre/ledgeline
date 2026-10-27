<!-- The shared column template of a multi-column statement (see statementGrid.ts):
     a line-name column that takes the leftover width, one fixed column per
     period, and an optional Total. Every table in the stack renders this same
     colgroup under `table-fixed`, which is what makes their columns line up. -->
<script lang="ts">
    import {PERIOD_REM, TOTAL_REM} from "./statementGrid";

    let {periods, total}: {periods: number; total: boolean} = $props();
</script>

<colgroup>
    <col />
    {#each {length: periods}, i (i)}
        <col style="width: {PERIOD_REM}rem" />
    {/each}
    {#if total}
        <col style="width: {TOTAL_REM}rem" />
    {/if}
</colgroup>
