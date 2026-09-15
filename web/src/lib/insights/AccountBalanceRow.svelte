<!-- One account in the Balances list: name over "As of", amount right-aligned on
     the first line. Two lines rather than a table because the amounts are the
     scan target and a table would put the dates in a column competing for that
     attention.

     # Why this is its own component

     Only so each row can MEASURE its own name cell. The insights box is far
     wider than the journal's accounts column, and `AccountLabel`'s unmeasured
     fallback is a thirty-character budget tuned for that column — so
     `liabilities:creditcards:chase-sapphire` came out as
     `lia:creditcards:chase-sapphire` in a panel with several hundred spare
     pixels. Measuring hands the label the room it actually has, and it only
     abbreviates once that room runs out (see AccountLabel's own notes).

     Rows differ from one another here — `justify-between` gives the name cell
     whatever the amount beside it did not take, and amounts are different
     widths — so this genuinely is per-row and not one number for the list. The
     cost is bounded: this list is short and does not virtualize, unlike the
     journal table, which is why THAT one shares a single observer on its
     column header (`journal/accountColumn.svelte.ts`).

     This cannot oscillate. The name cell is `flex-1 min-w-0`, i.e.
     `flex-basis: 0%`, so its width comes from the container and the amount
     beside it and never from its own content — a longer label cannot widen the
     box that decided how long the label may be. -->
<script lang="ts">
    import AccountLabel from "$lib/components/AccountLabel.svelte";
    import type {BalanceRow} from "$lib/reports/cashBalances";

    let {
        entry,
        formatted,
        negativeIsWrong,
        dotColor,
    }: {
        entry: BalanceRow;
        /** The already-formatted amount; formatting needs the commodity and style the list owns. */
        formatted: string;
        /** True in the Cash list, where a negative balance means overdrawn. False for liabilities, where it is normal. */
        negativeIsWrong: boolean;
        /** Pie-slice colour for this account, when it has one. */
        dotColor?: string;
    } = $props();

    /** The name renders at `text-sm`; the measurer has to be told, or it answers for 12px. */
    const NAME_FONT_SIZE_PX = 14;

    /**
     * Coarse fallback for the frame before the observer reports, and for engines
     * without one (jsdom, server rendering). Wider than `ACCOUNT_LABEL_BUDGET`
     * because this panel is wider than the column that number was tuned for;
     * where it overshoots, CSS clips from the left and the leaf still survives.
     */
    const FALLBACK_BUDGET = 44;

    let nameWidth = $state(0);

    /** Publish this row's name-cell content width. Same shape, and same guards, as `measureAccountColumn`. */
    function measureName(node: HTMLElement): {destroy(): void} | undefined {
        if (typeof ResizeObserver === "undefined") return undefined;
        const observer = new ResizeObserver((entries) => {
            const width = entries[0]?.contentRect?.width;
            if (typeof width !== "number" || !Number.isFinite(width) || width <= 0) return;
            // Sub-pixel noise would re-render the label for no visible change.
            if (Math.abs(width - nameWidth) < 1) return;
            nameWidth = width;
        });
        try {
            observer.observe(node);
        } catch {
            // An engine with the constructor that rejects the target falls back
            // to characters rather than taking the panel down.
            return undefined;
        }
        return {
            destroy(): void {
                observer.disconnect();
            },
        };
    }

    const negative = $derived(entry.qty.m < 0n);
</script>

<li class="flex items-baseline justify-between gap-3 px-1 py-1">
    <span class="flex min-w-0 flex-1 flex-col">
        <span class="flex min-w-0 items-baseline gap-1.5">
            {#if dotColor !== undefined}
                <span class="inline-block h-2 w-2 shrink-0 rounded-full" style="background:{dotColor}"></span>
            {/if}
            <span class="min-w-0 flex-1 text-sm" use:measureName>
                <AccountLabel name={entry.account} maxWidth={nameWidth > 0 ? nameWidth : undefined} fontSizePx={NAME_FONT_SIZE_PX} budget={FALLBACK_BUDGET} />
            </span>
        </span>
        <span class="text-xs text-base-content/50">
            {#if entry.asOf === null}
                No activity
            {:else}
                <!-- `&nbsp;` and not a plain space: Svelte trims whitespace at an element
                     boundary, so a leading " · " written inside the span rendered as
                     "2026-04-05· checked", with the space eaten. -->
                As of {entry.asOf}{#if entry.confirmed}<span title="Confirmed by a balance assertion in the journal">&nbsp;· checked</span>{/if}
            {/if}
        </span>
    </span>
    <span class="shrink-0 font-mono text-sm tabular-nums {negative && negativeIsWrong ? 'font-semibold text-error' : ''}">{formatted}</span>
</li>
