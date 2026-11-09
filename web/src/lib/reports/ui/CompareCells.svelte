<!-- The Δ and Δ% cells that replace a statement row's Total while a projection
     is compared with actuals (plans/34, D5). Neutral ink whatever the sign:
     a difference above the projection is as worth reading as one below it,
     and what a sign means depends on the row. -->
<script lang="ts">
    import type {MixedAmount} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {formatTotals} from "#lib/journal/rowModel.js";
    import {compareCells, comparePctTitle, fmtComparePct, type WindowReading} from "./compareColumns";

    let {
        values,
        split,
        reading,
        base,
        styles,
        blank = false,
        cellClass = "",
    }: {
        values: readonly MixedAmount[];
        /** N: the actual block's columns. */
        split: number;
        reading: WindowReading;
        base: string | null;
        styles: ReadonlyMap<string, AmountStyle>;
        /** Two empty cells, for a row with no figure to compare (the balance sheet's tie-out). */
        blank?: boolean;
        /** Classes every cell of the row's figures carries (weight, borders). */
        cellClass?: string;
    } = $props();

    const precision = $derived(base === null ? undefined : styles.get(base)?.precision);
    const cells = $derived(compareCells(values, split, reading, base, precision));
    const lines = $derived(formatTotals(cells.delta, styles));
</script>

{#if blank}
    <td class={cellClass} data-column="delta"></td>
    <td class={cellClass} data-column="delta-pct"></td>
{:else}
    <td class="{cellClass} text-right align-top font-mono whitespace-nowrap tabular-nums" data-column="delta">
        {#if lines.length === 0}
            <span class="text-base-content/40">0</span>
        {:else}
            {#each lines as line (line.text)}
                <div>{line.text}</div>
            {/each}
        {/if}
    </td>
    <td
        class="{cellClass} text-right align-top font-mono whitespace-nowrap text-base-content/70 tabular-nums"
        data-column="delta-pct"
        title={comparePctTitle(cells.pct)}>{fmtComparePct(cells.pct)}</td
    >
{/if}
