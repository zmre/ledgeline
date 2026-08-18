<script lang="ts">
    // Virtualized transaction list (WP-03): simple fixed-pitch windowing over a
    // scroll container (see computeWindow in rowModel.ts) — the rendered-row
    // count is bounded by viewport/pitch + overscan, independent of txns.length,
    // which keeps 50k+ rows smooth. Desktop renders a daisyUI table with a
    // sticky header + spacer rows; narrow widths (<640px) render card-per-txn.
    import {untrack} from "svelte";
    import {statusPatch} from "$lib/api/editMapping";
    import type {Transaction} from "$lib/domain/types";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {editing} from "$lib/stores/editing.svelte";
    import {filters} from "$lib/stores/filters.svelte";
    import {problems} from "$lib/stores/problems.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {listCursor} from "$lib/ui/listCursor.svelte";
    import ColumnMenu from "./ColumnMenu.svelte";
    import TransactionRow from "./TransactionRow.svelte";
    import {measureAccountColumn} from "./accountColumn.svelte";
    import {txnModal} from "./edit/modalState.svelte";
    import {rowActions} from "./rowAction.svelte";
    import {centerOffset, computeWindow, halfPageRows, nearestOffset, nextStatus} from "./rowModel";

    let {txns}: {txns: Transaction[]} = $props();

    const ROW_PITCH = 40; // h-10 table rows
    const CARD_PITCH = 104; // h-24 card (96px) + mb-2 (8px)
    const CARD_HEIGHT = 96; // the card itself, without its 8px margin
    const CARD_INSET = 8; // the card wrapper's p-2
    const OVERSCAN = 12;

    let scroller = $state<HTMLDivElement | null>(null);
    let scrollTop = $state(0);
    let viewportHeight = $state(600);
    let containerWidth = $state(1024);
    let headHeight = $state(0);

    const mode = $derived(containerWidth < 640 ? "card" : "table");
    const pitch = $derived(mode === "card" ? CARD_PITCH : ROW_PITCH);
    const rowHeight = $derived(mode === "card" ? CARD_HEIGHT : ROW_PITCH);
    const topInset = $derived(mode === "card" ? CARD_INSET : 0);
    // The sticky <thead> paints over the top of the viewport in table mode; card
    // mode has no header at all.
    const headroom = $derived(mode === "card" ? 0 : headHeight);
    const win = $derived(computeWindow(scrollTop, viewportHeight, pitch, txns.length, OVERSCAN));
    const visible = $derived(txns.slice(win.start, win.end));
    const columns = $derived(settings.columns);
    const colCount = $derived(Object.values(columns).filter(Boolean).length);

    // The keyboard cursor.
    //
    // Pure visual state, NOT DOM focus. A roving tabindex over a virtualized list
    // is a focus-loss machine: scroll the cursored row out and Svelte unmounts
    // it, the browser drops focus to <body>, and the keybindings die with nothing
    // on screen to explain why. `<tr>` is also not interactive, so making it
    // focusable is exactly the shape that trips `a11y_interactive_supports_focus`
    // in a codebase with no `svelte-ignore` comments.
    const cursor = listCursor<Transaction>(
        () => txns,
        (txn) => txn.index
    );
    /** The transaction `x` has armed for deletion, or null. */
    let armedDelete = $state<Transaction | null>(null);

    const announcement = $derived.by(() => {
        const txn = cursor.item;
        if (txn === null) return "";
        return `Row ${cursor.index + 1} of ${txns.length}. ${txn.date}, ${txn.description || "no description"}.`;
    });

    /**
     * Bring row `position` into view.
     *
     * `nearest` for cursor moves — j and k must not throw the page around.
     * `center` for arriving from somewhere else (the problems drawer), where
     * context around the target is what you want.
     */
    function reveal(position: number, how: "center" | "nearest"): void {
        if (position < 0) return;
        const top =
            how === "center"
                ? centerOffset(position, pitch, viewportHeight, topInset)
                : nearestOffset(scrollTop, position, pitch, rowHeight, viewportHeight, txns.length, headroom, topInset);
        if (top === scrollTop) return;
        if (scroller !== null) scroller.scrollTop = top;
        scrollTop = top;
    }

    function moveCursor(delta: number): void {
        cursor.move(delta);
        reveal(cursor.index, "nearest");
    }

    /** Open an inline editor on the cursored row. Reveal FIRST, so the row is mounted to hear it. */
    function requestRowAction(action: "description" | "category"): void {
        const txn = cursor.item;
        if (txn === null || !editing.canEdit) return;
        reveal(cursor.index, "nearest");
        rowActions.open(txn.index, action);
    }

    async function cycleStatus(): Promise<void> {
        const txn = cursor.item;
        if (txn === null || !editing.canEdit) return;
        const result = await editing.patch(txn.index, statusPatch(nextStatus(txn.status)));
        // The engine reassigns tindex on write, so re-anchor by position.
        cursor.reanchor();
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }

    /** `x` arms, a second `x` confirms. Two-step, matching the popup's delete and IfBlockCard's remove. */
    function armOrConfirmDelete(): void {
        const txn = cursor.item;
        if (txn === null || !editing.canEdit) return;
        if (armedDelete !== null && armedDelete.index === txn.index) void confirmDelete();
        else armedDelete = txn;
    }

    async function confirmDelete(): Promise<void> {
        const txn = armedDelete;
        if (txn === null) return;
        armedDelete = null;
        const result = await editing.remove(txn.index);
        cursor.reanchor();
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }

    // `enabled` rather than conditional registration, so the help sheet hides
    // these on a read-only engine AND the keys stay unclaimed there.
    const canEdit = (): boolean => editing.canEdit;

    registerKeys({
        id: "journal-table",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: "Next transaction", group: "Journal", run: () => moveCursor(1)},
            {keys: "ArrowDown", label: "Next transaction", group: "Journal", run: () => moveCursor(1)},
            {keys: "k", label: "Previous transaction", group: "Journal", run: () => moveCursor(-1)},
            {keys: "ArrowUp", label: "Previous transaction", group: "Journal", run: () => moveCursor(-1)},
            // PageDown/PageUp are bound alongside the Ctrl chords deliberately:
            // Ctrl-U is view-source in Chrome and not reliably preventable, so
            // there has to be an uncontested path. Harmless in the WKWebView the
            // app ships in, real in `just dev`.
            {keys: "ctrl+d", label: "Half page down", group: "Journal", run: () => moveCursor(halfPageRows(viewportHeight, pitch))},
            {keys: "PageDown", label: "Half page down", group: "Journal", run: () => moveCursor(halfPageRows(viewportHeight, pitch))},
            {keys: "ctrl+u", label: "Half page up", group: "Journal", run: () => moveCursor(-halfPageRows(viewportHeight, pitch))},
            {keys: "PageUp", label: "Half page up", group: "Journal", run: () => moveCursor(-halfPageRows(viewportHeight, pitch))},
            {keys: "g g", label: "First transaction", group: "Journal", run: () => (cursor.first(), reveal(cursor.index, "nearest"))},
            {keys: "G", label: "Last transaction", group: "Journal", run: () => (cursor.last(), reveal(cursor.index, "nearest"))},
            {keys: "Escape", label: "Clear the cursor", group: "Journal", run: () => (armedDelete === null ? cursor.clear() : (armedDelete = null))},
            {
                keys: "Enter",
                label: "Edit transaction",
                group: "Journal",
                enabled: canEdit,
                run: () => cursor.item !== null && txnModal.openEdit(cursor.item),
            },
            {keys: "e", label: "Edit description", group: "Journal", enabled: canEdit, run: () => requestRowAction("description")},
            {keys: "c", label: "Edit category", group: "Journal", enabled: canEdit, run: () => requestRowAction("category")},
            {keys: "s", label: "Cycle status", group: "Journal", enabled: canEdit, run: () => void cycleStatus()},
            {keys: "x", label: "Delete transaction", group: "Journal", enabled: canEdit, run: armOrConfirmDelete},
            {keys: "n", label: "Add transaction", group: "Journal", enabled: canEdit, run: () => txnModal.openAdd()},
        ],
    });

    /** Keep the cursor in step with the mouse: clicking a row's button cursors that row. */
    function syncCursorFromTarget(target: EventTarget | null): void {
        const row = (target as HTMLElement | null)?.closest?.("[data-txn]");
        if (row === null || row === undefined) return;
        const index = Number(row.getAttribute("data-txn"));
        const position = txns.findIndex((txn) => txn.index === index);
        if (position !== -1) cursor.to(position);
    }

    // EFFECT 1 — when the FILTER changes, jump back to the top: it is a
    // different list now, so the old scroll position means nothing.
    //
    // Keyed on `filters.value`, NOT on `txns`. `txns` is a fresh array after
    // every refresh — including the one `editing.patch` fires on success — so
    // keying on it sent the user back to row 0 after every inline edit. Nobody
    // noticed because clicking is slow; it becomes the first complaint the
    // moment a keystroke can cycle a status. `filters.value` is replaced
    // wholesale on every filter change, so its identity is a valid key.
    $effect(() => {
        void filters.value;
        if (scroller !== null) scroller.scrollTop = 0;
        scrollTop = 0;
        cursor.clear();
        armedDelete = null;
    });

    // WP-08: problems-drawer navigation. Scroll a txn's row into (centered)
    // view and pulse it briefly so the eye lands on the right record.
    let pulseIndex = $state<number | null>(null);
    let pulseTimer: ReturnType<typeof setTimeout> | undefined;

    export function scrollToTxn(index: number): void {
        const position = txns.findIndex((txn) => txn.index === index);
        if (position === -1) return;
        reveal(position, "center");
        // Arriving from the drawer leaves you on that row, so `j` continues from
        // there rather than starting over at the top.
        cursor.to(position);
        pulseIndex = index;
        clearTimeout(pulseTimer);
        pulseTimer = setTimeout(() => (pulseIndex = null), 2000);
    }

    // EFFECT 2 — re-reveal the cursor when the layout mode flips.
    //
    // Pitch goes 40 → 104 across the 640px breakpoint, so the same `scrollTop`
    // afterwards points at a completely different row. That is a pre-existing
    // bug the cursor happens to make fixable: with a cursor there is finally a
    // "here" to return to.
    $effect(() => {
        void mode;
        reveal(
            untrack(() => cursor.index),
            "nearest"
        );
    });

    // EFFECT 3 — consume focus requests from the problems store (declared AFTER
    // the two effects above so a drawer jump wins when they land in one flush).
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
        <!-- The delete confirm lives in the TOOLBAR, not in the row: this is
             outside the virtualized area so it cannot unmount mid-confirm, and
             unlike a row-local "Delete?" it can say WHAT is about to go. It also
             gives keyboard delete a mouse path for free — there was no delete
             affordance outside the transaction popup at all. -->
        {#if armedDelete !== null}
            <div class="border-error/40 flex items-center gap-2 rounded border px-2 py-1 text-sm" role="alert">
                <span>
                    Delete <span class="font-mono">{armedDelete.date}</span>
                    {armedDelete.description || "(no description)"}?
                </span>
                <button type="button" class="btn btn-error btn-xs" onclick={() => void confirmDelete()}>Delete</button>
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (armedDelete = null)}>Keep</button>
            </div>
        {/if}
        <ColumnMenu />
    </div>
    <!-- There is no DOM focus on the cursored row (see the cursor's comment), so
         nothing would otherwise be announced as it moves. -->
    <div class="sr-only" aria-live="polite" aria-atomic="true">{announcement}</div>
    <div
        bind:this={scroller}
        bind:clientHeight={viewportHeight}
        bind:clientWidth={containerWidth}
        onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
        onfocusin={(event) => syncCursorFromTarget(event.target)}
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
                <thead class="bg-base-200 sticky top-0 z-10" bind:clientHeight={headHeight}>
                    <tr class="text-base-content/70">
                        {#if columns.date}<th class="text-left">Date</th>{/if}
                        {#if columns.status}<th class="text-left">Status</th>{/if}
                        {#if columns.description}<th class="text-left">Description</th>{/if}
                        <!-- One measurement for the whole column: `table-fixed`
                             makes every accounts cell this wide, so the chips
                             below fit themselves to real pixels without a
                             ResizeObserver per row. -->
                        {#if columns.accounts}<th class="text-left" use:measureAccountColumn>Accounts</th>{/if}
                        {#if columns.amount}<th class="text-right">Amount</th>{/if}
                    </tr>
                </thead>
                <tbody>
                    {#if win.padTop > 0}
                        <tr aria-hidden="true" style="height: {win.padTop}px"><td colspan={colCount} class="p-0"></td></tr>
                    {/if}
                    {#each visible as txn (txn.index)}
                        <TransactionRow
                            {txn}
                            {columns}
                            mode="row"
                            flags={problems.byTxn.get(txn.index)}
                            pulse={pulseIndex === txn.index}
                            cursor={cursor.key === txn.index}
                        />
                    {/each}
                    {#if win.padBottom > 0}
                        <tr aria-hidden="true" style="height: {win.padBottom}px"><td colspan={colCount} class="p-0"></td></tr>
                    {/if}
                </tbody>
            </table>
        {:else}
            <div class="p-2" style="padding-top: {win.padTop + 8}px; padding-bottom: {win.padBottom + 8}px">
                {#each visible as txn (txn.index)}
                    <TransactionRow
                        {txn}
                        {columns}
                        mode="card"
                        flags={problems.byTxn.get(txn.index)}
                        pulse={pulseIndex === txn.index}
                        cursor={cursor.key === txn.index}
                    />
                {/each}
            </div>
        {/if}
    </div>
</section>
