<script lang="ts">
    // Virtualized transaction list (WP-03): simple fixed-pitch windowing over a
    // scroll container (see computeWindow in rowModel.ts) — the rendered-row
    // count is bounded by viewport/pitch + overscan, independent of txns.length,
    // which keeps 50k+ rows smooth. Desktop renders a daisyUI table with a
    // sticky header + spacer rows; narrow widths (<640px) render card-per-txn.
    import type {Transaction} from "$lib/domain/types";
    import {editing} from "$lib/stores/editing.svelte";
    import {problems} from "$lib/stores/problems.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import ColumnMenu from "./ColumnMenu.svelte";
    import TransactionRow from "./TransactionRow.svelte";
    import {txnModal} from "./edit/modalState.svelte";
    import {computeWindow} from "./rowModel";

    let {txns}: {txns: Transaction[]} = $props();

    const ROW_PITCH = 40; // h-10 table rows
    const CARD_PITCH = 104; // h-24 card (96px) + mb-2 (8px)
    const OVERSCAN = 12;

    let scroller = $state<HTMLDivElement | null>(null);
    let scrollTop = $state(0);
    let viewportHeight = $state(600);
    let containerWidth = $state(1024);

    const mode = $derived(containerWidth < 640 ? "card" : "table");
    const pitch = $derived(mode === "card" ? CARD_PITCH : ROW_PITCH);
    const win = $derived(computeWindow(scrollTop, viewportHeight, pitch, txns.length, OVERSCAN));
    const visible = $derived(txns.slice(win.start, win.end));
    const columns = $derived(settings.columns);
    const colCount = $derived(Object.values(columns).filter(Boolean).length);

    // When the dataset changes (filter/refresh), jump back to the top so the
    // window matches what the user expects to see.
    $effect(() => {
        void txns;
        if (scroller !== null) scroller.scrollTop = 0;
        scrollTop = 0;
    });

    // WP-08: problems-drawer navigation. Scroll a txn's row into (centered)
    // view and pulse it briefly so the eye lands on the right record.
    let pulseIndex = $state<number | null>(null);
    let pulseTimer: ReturnType<typeof setTimeout> | undefined;

    export function scrollToTxn(index: number): void {
        const position = txns.findIndex((txn) => txn.index === index);
        if (position === -1) return;
        const top = Math.max(0, position * pitch - Math.max(0, viewportHeight - pitch) / 2);
        if (scroller !== null) scroller.scrollTop = top;
        scrollTop = top;
        pulseIndex = index;
        clearTimeout(pulseTimer);
        pulseTimer = setTimeout(() => (pulseIndex = null), 2000);
    }

    // Consume focus requests from the problems store (declared AFTER the
    // scroll-to-top effect above so it wins when both fire in one flush).
    $effect(() => {
        const request = problems.focusRequest;
        if (request === null) return;
        scrollToTxn(request.txnIndex);
        problems.clearFocus();
    });
</script>

<section class="flex min-h-0 grow flex-col">
    <div class="flex items-center justify-between gap-2 pb-1">
        {#if editing.canEdit}
            <button type="button" class="btn btn-primary btn-sm gap-1" onclick={() => txnModal.openAdd()}>
                <span class="text-base leading-none">+</span> Add transaction
            </button>
        {:else}
            <span></span>
        {/if}
        <ColumnMenu />
    </div>
    <div
        bind:this={scroller}
        bind:clientHeight={viewportHeight}
        bind:clientWidth={containerWidth}
        onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
        class="border-base-300 min-h-0 grow overflow-y-auto rounded-lg border"
    >
        {#if txns.length === 0}
            <div class="text-base-content/60 p-8 text-center text-sm">No transactions match the current filters.</div>
        {:else if mode === "table"}
            <table class="table-sm table table-fixed">
                <colgroup>
                    {#if columns.date}<col class="w-24" />{/if}
                    {#if columns.status}<col class="w-16" />{/if}
                    {#if columns.description}<col />{/if}
                    {#if columns.accounts}<col />{/if}
                    {#if columns.amount}<col class="w-36" />{/if}
                </colgroup>
                <thead class="bg-base-200 sticky top-0 z-10">
                    <tr class="text-base-content/70">
                        {#if columns.date}<th class="text-left">Date</th>{/if}
                        {#if columns.status}<th class="text-left">Status</th>{/if}
                        {#if columns.description}<th class="text-left">Description</th>{/if}
                        {#if columns.accounts}<th class="text-left">Accounts</th>{/if}
                        {#if columns.amount}<th class="text-right">Amount</th>{/if}
                    </tr>
                </thead>
                <tbody>
                    {#if win.padTop > 0}
                        <tr aria-hidden="true" style="height: {win.padTop}px"><td colspan={colCount} class="p-0"></td></tr>
                    {/if}
                    {#each visible as txn (txn.index)}
                        <TransactionRow {txn} {columns} mode="row" flags={problems.byTxn.get(txn.index)} pulse={pulseIndex === txn.index} />
                    {/each}
                    {#if win.padBottom > 0}
                        <tr aria-hidden="true" style="height: {win.padBottom}px"><td colspan={colCount} class="p-0"></td></tr>
                    {/if}
                </tbody>
            </table>
        {:else}
            <div class="p-2" style="padding-top: {win.padTop + 8}px; padding-bottom: {win.padBottom + 8}px">
                {#each visible as txn (txn.index)}
                    <TransactionRow {txn} {columns} mode="card" flags={problems.byTxn.get(txn.index)} pulse={pulseIndex === txn.index} />
                {/each}
            </div>
        {/if}
    </div>
</section>
