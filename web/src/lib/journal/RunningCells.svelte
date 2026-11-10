<script lang="ts">
    // The Change and Balance cells of a pinned running balance (plan 36): two
    // `<td>`s in table mode, one compact line in card mode.
    //
    //   - Change: the pinned subtree's net in the chosen commodity on this row.
    //     Blank when the row has nothing there; "posts YYYY-MM-DD" beneath when
    //     a `date:` tag counts the leg on another day.
    //   - Balance: the subtree's balance after the row. A row that does not
    //     change it shows the balance CARRIED from earlier, dimmed, so the
    //     column still reads as a timeline whatever the account filter shows.
    //   - Assertion marks after the balance: ✓, or ✗ with the detail in a
    //     tooltip. Pass/fail is the engine checker's verdict, never a
    //     comparison with this column (a `=` on a child asserts only the child).
    //     Clicking ✗ narrows the date range to the gap.
    import {formatAmount} from "#lib/domain/money.js";
    import type {ISODate} from "#lib/domain/types.js";
    import type {RunningAssertion, RunningCell} from "#lib/journalView/types.js";
    import {filters} from "#lib/stores/filters.svelte.js";
    import {running} from "#lib/stores/running.svelte.js";
    import {assertionDetail, assertionLabel, gapRange} from "./runningBalance";

    let {
        cell,
        pinned,
        txnIndex,
        txnDate,
        mode = "row",
    }: {
        /** The row's cell, or null while the rows for this pin have not arrived. */
        cell: RunningCell | null;
        /** The pinned subtree root. */
        pinned: string;
        txnIndex: number;
        txnDate: ISODate;
        mode?: "row" | "card";
    } = $props();

    const carried = $derived(cell !== null && cell.change === null);
    const internal = $derived(cell !== null && cell.change !== null && cell.change.qty.m === 0n);
    /**
     * Marks in the chosen commodity only — plus a `==` failure in any other,
     * which means a commodity that should be zero is not. The engine already
     * sends just these; filtering again costs nothing and keeps the rule here.
     */
    const marks = $derived(
        cell === null ? [] : cell.assertions.filter((a) => a.asserted.commodity === cell.balance.commodity || (!a.ok && a.op.startsWith("==")))
    );

    /** Narrow the date range to where the error must be; the account filter and search stay as they are. */
    function narrow(assertion: RunningAssertion): void {
        const points = running.for(pinned)?.assertions[assertion.asserted.commodity];
        if (points === undefined) return;
        const range = gapRange(points, txnIndex, txnDate);
        if (range !== null) filters.setRange(range.from, range.to);
    }
</script>

{#snippet assertionMarks()}
    {#each marks as assertion, i (i)}
        {@const detail = assertionDetail(assertion)}
        {#if assertion.ok}
            <span class="tooltip tooltip-left shrink-0 text-success" data-tip={detail} data-testid="running-assertion-ok">
                <span aria-label={detail}>{assertionLabel(assertion, pinned)}</span>
            </span>
        {:else}
            <span class="tooltip tooltip-left shrink-0" data-tip="{detail}. Click to narrow the dates to the gap.">
                <button
                    type="button"
                    class="cursor-pointer font-semibold text-error"
                    aria-label="Narrow to gap: {detail}"
                    data-testid="running-assertion-fail"
                    onclick={() => narrow(assertion)}>{assertionLabel(assertion, pinned)}</button
                >
            </span>
        {/if}
    {/each}
{/snippet}

{#if mode === "row"}
    <td class="py-1 text-right font-mono text-sm tabular-nums" data-testid="running-change">
        {#if cell !== null && cell.change !== null}
            <div
                class={["whitespace-nowrap", cell.change.qty.m < 0n && "text-error", internal && "text-base-content/60"]}
                title={internal ? `Internal: moves within ${pinned}` : undefined}
            >
                {formatAmount(cell.change)}
            </div>
        {/if}
        {#if cell?.postedOn != null}
            <div class="text-xs whitespace-nowrap text-base-content/60" data-testid="running-posted-on">posts {cell.postedOn}</div>
        {/if}
    </td>
    <td class="py-1 font-mono text-sm tabular-nums" data-testid="running-balance">
        {#if cell !== null}
            <div class="flex items-center justify-end gap-1.5 whitespace-nowrap">
                <span
                    class={carried ? "text-base-content/50" : undefined}
                    data-carried={carried ? "" : undefined}
                    title={carried ? `Carried: this row doesn't change ${pinned} in ${cell.balance.commodity}` : undefined}>{formatAmount(cell.balance)}</span
                >
                {@render assertionMarks()}
            </div>
        {/if}
    </td>
{:else if cell !== null}
    <span class="flex shrink-0 items-center gap-1 font-mono text-xs whitespace-nowrap tabular-nums" data-testid="running-balance">
        <span class="text-base-content/60">Bal</span>
        <span class={carried ? "text-base-content/50" : undefined} data-carried={carried ? "" : undefined}>{formatAmount(cell.balance)}</span>
        {@render assertionMarks()}
    </span>
{/if}
