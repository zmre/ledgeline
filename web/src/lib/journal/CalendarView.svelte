<script lang="ts">
    // The journal as a month calendar (plan 40): the same filtered view the
    // table shows — search and account filters included — with the date range
    // held to one month by the filter store (`holdMonth`). Navigating months
    // moves that range, so the insights panel, the footer and the URL follow.
    //
    // Each day lists its transactions one line apiece: the description (or a
    // BUY/SELL pill, symbol and shares for a trade) truncated left, the amount
    // rounded to whole units right. A day with more than fits says "N more",
    // which opens the whole day over the cell. Clicking a transaction opens
    // the editor on it.
    //
    // Not for phones: a day cell at phone width holds a few characters, and a
    // one-day view would only repeat the list. Below the `md` breakpoint
    // TransactionTable never mounts this: no toggle, no `v`, and a `cal=1`
    // filter shows the list (`calendarFits` in calendarModel.ts, plan 42).
    //
    // Rows come from the journal view's block cache (`ensure` over the whole
    // month), so an edit re-reads them exactly as it re-reads the table's.
    import {untrack} from "svelte";
    import {hint} from "#lib/keys/hint.svelte.js";
    import {styleOf} from "#lib/format/amounts.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {editing} from "#lib/stores/editing.svelte.js";
    import {filters, localToday} from "#lib/stores/filters.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {journalView} from "#lib/stores/journalView.svelte.js";
    import {rootPx} from "#lib/theme/scale.js";
    import {calendarItem, fitDay, groupByDay, monthGrid, monthLabel, monthOf, shiftMonth, WEEKDAYS, type CalendarItem} from "./calendarModel";
    import {txnModal} from "./edit/modalState.svelte";

    let {pulse = null}: {pulse?: number | null} = $props();

    /** One item line, in rem: the `h-5` below. */
    const LINE_REM = 1.25;
    /**
     * One week's height, in rem. Fixed: at the widest main column (100rem, so
     * about 14rem a day) a cell is a little taller than square. Ten lines of
     * transactions after the day number.
     */
    const WEEK_REM = 15;

    const today = localToday();
    const month = $derived(monthOf(filters.value.from ?? today));
    const weeks = $derived(monthGrid(month));
    const total = $derived(journalView.total);

    // Ask for every row of the month. Keyed on the view's query (a generation
    // change re-reads stale blocks) and its size, never on the rows themselves.
    $effect(() => {
        void journalView.query;
        const count = journalView.total;
        if (count === null || count === 0) return;
        untrack(() => journalView.ensure(0, count));
    });

    const items = $derived.by((): CalendarItem[] => {
        const count = total ?? 0;
        const styles = journal.reportStyles;
        const opts = {classes: journal.accountClasses, styleOf: (commodity: string) => styleOf(styles, commodity), preferred: journal.dominantCommodity};
        const out: CalendarItem[] = [];
        for (let position = 0; position < count; position += 1) {
            const row = journalView.rowAt(position);
            if (row !== null) out.push(calendarItem(row, opts));
        }
        return out;
    });
    const loading = $derived(total === null || items.length < total);
    const byDay = $derived(groupByDay(items));

    /** Each cell's list area, in px, by its place in the grid. All cells are one height, but measuring each keeps that an outcome, not an assumption. */
    let heights = $state<number[]>([]);
    const capacityAt = (cell: number): number => Math.floor((heights[cell] ?? 0) / (LINE_REM * rootPx()));

    /** The day whose full list is open over its cell, or null. */
    let openDay = $state<string | null>(null);

    function go(delta: number): void {
        openDay = null;
        filters.showMonth(shiftMonth(month, delta));
    }
    function thisMonth(): void {
        openDay = null;
        filters.showMonth(monthOf(today));
    }
    function edit(item: CalendarItem): void {
        if (!editing.canEdit) return;
        openDay = null;
        txnModal.openEdit(item.index);
    }

    registerKeys({
        id: "journal-calendar",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "h", label: "Previous month", group: "Journal", run: () => go(-1)},
            {keys: "ArrowLeft", label: "Previous month", group: "Journal", run: () => go(-1)},
            {keys: "l", label: "Next month", group: "Journal", run: () => go(1)},
            {keys: "ArrowRight", label: "Next month", group: "Journal", run: () => go(1)},
            {keys: "t", label: "This month", group: "Journal", run: thisMonth},
        ],
    });

    const DOT = {error: "bg-error", warning: "bg-warning", info: "bg-info"} as const;
    const TONE = {positive: "text-success", negative: "text-error", neutral: "text-base-content/70"} as const;
</script>

{#snippet line(item: CalendarItem)}
    <button
        type="button"
        class={[
            "flex h-5 w-full min-w-0 shrink-0 items-center gap-1 rounded px-1 text-left text-xs leading-5",
            editing.canEdit ? "hover:bg-base-200" : "cursor-default",
            pulse === item.index && "animate-pulse bg-primary/15",
        ]}
        title={item.title}
        onclick={() => edit(item)}
        data-testid="calendar-item"
        data-txn={item.index}
    >
        {#if item.severity !== null}
            <span class="block h-1.5 w-1.5 shrink-0 rounded-full {DOT[item.severity]}" role="img" aria-label={item.severity}></span>
        {/if}
        {#if item.trade !== null}
            <span class={["badge shrink-0 badge-xs font-semibold", item.trade.side === "buy" ? "badge-info" : "badge-success"]} data-testid="calendar-trade"
                >{item.trade.side === "buy" ? "BUY" : "SELL"}</span
            >
        {/if}
        <span class="min-w-0 grow truncate">{item.label}</span>
        {#if item.amount !== null}
            <span class="shrink-0 font-mono whitespace-nowrap tabular-nums {TONE[item.tone]}" data-testid="calendar-amount">{item.amount}</span>
        {/if}
    </button>
{/snippet}

<div class="flex shrink-0 flex-col gap-1" data-testid="journal-calendar">
    <div class="flex items-center gap-2">
        <button type="button" class="btn btn-square btn-ghost btn-sm" aria-label="Previous month" title={hint("Previous month", "h")} onclick={() => go(-1)}
            >‹</button
        >
        <h2 class="min-w-36 text-center text-base font-semibold" aria-live="polite" data-testid="calendar-month">{monthLabel(month)}</h2>
        <button type="button" class="btn btn-square btn-ghost btn-sm" aria-label="Next month" title={hint("Next month", "l")} onclick={() => go(1)}>›</button>
        <button type="button" class="btn btn-ghost btn-xs" disabled={month === monthOf(today)} title={hint("This month", "t")} onclick={thisMonth}>Today</button
        >
        {#if loading}
            <span class="loading loading-xs loading-spinner" aria-label="Loading transactions"></span>
        {:else if total === 0}
            <span class="text-xs text-base-content/60">No transactions match the current filters.</span>
        {/if}
    </div>

    <!-- Every week is WEEK_REM tall, whatever the window or the page above it:
         a share of the leftover height made a busy month (and its taller
         insights panel) draw smaller cells than an empty one. Only the width
         follows the window; the page scrolls for the rest. -->
    <div class="grid grid-cols-7 rounded-lg border border-base-300" style="grid-template-rows: auto repeat({weeks.length}, {WEEK_REM}rem)">
        {#each WEEKDAYS as weekday (weekday)}
            <div class="border-b border-base-300 bg-base-200 px-2 py-1 text-xs font-semibold text-base-content/70">{weekday}</div>
        {/each}
        {#each weeks as week, w (week[0])}
            {#each week as day, d (day)}
                {@const cell = w * 7 + d}
                {@const inMonth = monthOf(day) === month}
                {@const list = inMonth ? (byDay.get(day) ?? []) : []}
                {@const fit = fitDay(list.length, capacityAt(cell))}
                {@const bottom = w >= weeks.length - 2}
                <div
                    class={[
                        "relative flex min-h-0 min-w-0 flex-col border-base-300 p-0.5",
                        d < 6 && "border-r",
                        w < weeks.length - 1 && "border-b",
                        !inMonth && "bg-base-200/40",
                    ]}
                    data-day={day}
                >
                    <div class="flex h-5 shrink-0 items-center px-1 text-xs">
                        <span
                            class={[
                                "tabular-nums",
                                !inMonth && "text-base-content/35",
                                day === today && "rounded-full bg-primary px-1.5 font-semibold text-primary-content",
                            ]}>{Number(day.slice(8))}</span
                        >
                    </div>
                    <div class="flex min-h-0 grow flex-col overflow-hidden" bind:clientHeight={heights[cell]}>
                        {#each list.slice(0, fit.shown) as item (item.index)}
                            {@render line(item)}
                        {/each}
                        {#if fit.more > 0}
                            <button
                                type="button"
                                class="h-5 shrink-0 rounded px-1 text-left text-xs leading-5 text-base-content/60 hover:bg-base-200 hover:text-base-content"
                                onclick={() => (openDay = day)}
                                data-testid="calendar-more">{fit.more} more</button
                            >
                        {/if}
                    </div>
                    {#if openDay === day}
                        <div
                            class={[
                                "absolute z-20 flex max-h-80 w-72 max-w-[80vw] flex-col gap-0.5 overflow-y-auto rounded-box border border-base-300 bg-base-100 p-1.5 shadow-lg",
                                d >= 4 ? "right-0" : "left-0",
                                bottom ? "bottom-0" : "top-0",
                            ]}
                            role="dialog"
                            aria-label="Transactions on {day}"
                            use:dismissible={{onDismiss: () => (openDay = null), outside: true}}
                            data-testid="calendar-day-list"
                        >
                            <div class="flex items-center justify-between px-1 pb-0.5 text-xs font-semibold">
                                <span>{day}</span>
                                <button type="button" class="btn btn-ghost btn-xs" aria-label="Close" onclick={() => (openDay = null)}>✕</button>
                            </div>
                            {#each list as item (item.index)}
                                {@render line(item)}
                            {/each}
                        </div>
                    {/if}
                </div>
            {/each}
        {/each}
    </div>
</div>
