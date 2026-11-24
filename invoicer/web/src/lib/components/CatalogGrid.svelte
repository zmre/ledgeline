<!-- The catalog's table as a grid (invoicer PLAN 4c, ARCHITECTURE §3.6): one
     row per item, a field per column by role (code, description, unit, rate,
     the books cell with its account picker and deferral popover, any other
     column as text). The cells are the raw markdown the note holds, so a
     hand-written cell is never lost.

     Keyboard: Tab through the cells; Alt+ArrowUp / Alt+ArrowDown moves the
     focused row; each row also has Move up, Move down and Delete. A pointer
     can drag a row by its handle. Add item goes after the focused row. A
     cell with a problem is marked and named below the grid. -->
<script lang="ts">
    import {tick} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import {plain} from "#lib/arithmetic.js";
    import {moveRow, removeRow, type GridForm, type Role} from "#lib/editor.js";
    import {addCatalogRow, setCatalogCell, type CellProblem} from "#lib/library.js";
    import DeferralPopover from "./DeferralPopover.svelte";

    let {
        table,
        accounts = [],
        problems = [],
        readonly = false,
        onChange,
    }: {
        table: GridForm;
        accounts?: string[];
        /** Cells to mark: this app's own checks and the server's last refusal. */
        problems?: CellProblem[];
        readonly?: boolean;
        onChange: (table: GridForm) => void;
    } = $props();

    let focusedRow = $state<number | null>(null);
    let dragFrom = $state<number | null>(null);
    let gridEl = $state<HTMLTableSectionElement | null>(null);
    let announce = $state("");

    const LABEL: Record<Role, string> = {
        description: "description",
        quantity: "quantity",
        rate: "rate",
        amount: "amount",
        date: "date",
        unit: "unit",
        item: "code",
        books: "account",
        other: "column",
    };

    const headerText = (header: string, role: Role): string => (role === "books" ? "Account" : plain(header.replace(/^\*(.*)\*$/, "$1")));

    const problemAt = (row: number, column: number): CellProblem | undefined => problems.find((p) => p.row === row && p.column === column);
    const sorted = $derived([...problems].sort((a, b) => a.row - b.row || (a.column ?? -1) - (b.column ?? -1)));

    async function focusRow(row: number, column: number): Promise<void> {
        await tick();
        const target = gridEl?.querySelector<HTMLElement>(`tr[data-row="${row}"] [data-col="${column}"] input`);
        (target ?? gridEl?.querySelector<HTMLElement>(`tr[data-row="${row}"] button[data-move="up"]`))?.focus();
    }

    function move(from: number, to: number, column = 0): void {
        if (to < 0 || to >= table.rows.length) return;
        onChange(moveRow(table, from, to));
        focusedRow = to;
        announce = `Row ${from + 1} moved to ${to + 1}`;
        void focusRow(to, column);
    }

    function onRowKeydown(event: KeyboardEvent, row: number): void {
        if (readonly || !event.altKey || (event.key !== "ArrowUp" && event.key !== "ArrowDown")) return;
        event.preventDefault();
        const cell = (event.target as HTMLElement | null)?.closest<HTMLElement>("[data-col]");
        move(row, event.key === "ArrowUp" ? row - 1 : row + 1, Number(cell?.dataset.col ?? 0));
    }

    function add(): void {
        const {table: next, at} = addCatalogRow(table, focusedRow);
        onChange(next);
        focusedRow = at;
        void focusRow(at, 0);
    }

    const update = (row: number, column: number, text: string): void => onChange(setCatalogCell(table, row, column, text));
</script>

<section class="flex flex-col gap-2" aria-label="Catalog items" data-testid="catalog-grid">
    <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table table-sm">
            <thead>
                <tr>
                    <th class="w-8"><span class="sr-only">Move</span></th>
                    {#each table.columns as column, c (c)}
                        <th class={column.role === "rate" ? "text-right" : ""}>{headerText(column.header, column.role)}</th>
                    {/each}
                    <th><span class="sr-only">Row actions</span></th>
                </tr>
            </thead>
            <tbody bind:this={gridEl}>
                {#each table.rows as cells, r (r)}
                    <tr
                        data-row={r}
                        data-testid="catalog-row"
                        class={dragFrom === r ? "opacity-50" : ""}
                        onfocusin={() => (focusedRow = r)}
                        onkeydown={(event) => onRowKeydown(event, r)}
                        ondragover={(event) => {
                            if (dragFrom !== null) event.preventDefault();
                        }}
                        ondrop={(event) => {
                            event.preventDefault();
                            if (dragFrom !== null && dragFrom !== r) move(dragFrom, r);
                            dragFrom = null;
                        }}
                    >
                        <td class="w-8 align-top">
                            {#if !readonly}
                                <span
                                    class="cursor-grab text-base-content/40 select-none"
                                    draggable="true"
                                    role="presentation"
                                    title="Drag to move (or Alt+↑ / Alt+↓)"
                                    ondragstart={(event) => {
                                        dragFrom = r;
                                        event.dataTransfer?.setData("text/plain", String(r));
                                    }}
                                    ondragend={() => (dragFrom = null)}>⠿</span
                                >
                            {/if}
                        </td>
                        {#each table.columns as column, c (c)}
                            {@const label = `Row ${r + 1} ${LABEL[column.role]}`}
                            {@const problem = problemAt(r, c)}
                            <td data-col={c} class="align-top">
                                {#if column.role === "books"}
                                    <div class="flex items-start gap-1">
                                        <div class="min-w-56 flex-1 {problem ? 'rounded-field outline outline-error' : ''}">
                                            <AccountInput
                                                bind:value={() => cells[c], (v) => update(r, c, v)}
                                                accountNames={accounts}
                                                {label}
                                                placeholder="default"
                                                size="xs"
                                                disabled={readonly}
                                            />
                                        </div>
                                        {#if !readonly}
                                            <DeferralPopover cell={cells[c]} {accounts} label="Row {r + 1} deferral" onApply={(text) => update(r, c, text)} />
                                        {/if}
                                    </div>
                                {:else}
                                    <input
                                        class="input input-xs {column.role === 'rate' ? 'w-28 text-right tabular-nums' : ''} {column.role === 'item'
                                            ? 'w-28 font-mono'
                                            : ''} {column.role === 'unit' ? 'w-16' : ''} {column.role === 'description' || column.role === 'other'
                                            ? 'w-full min-w-48'
                                            : ''} {problem ? 'input-error' : ''}"
                                        aria-label={label}
                                        aria-invalid={problem ? "true" : undefined}
                                        title={problem?.message}
                                        value={cells[c]}
                                        disabled={readonly}
                                        oninput={(event) => update(r, c, event.currentTarget.value)}
                                        data-role={column.role}
                                    />
                                {/if}
                            </td>
                        {/each}
                        <td class="align-top whitespace-nowrap">
                            {#if !readonly}
                                <button
                                    type="button"
                                    class="btn btn-square btn-ghost btn-xs"
                                    aria-label="Move row {r + 1} up"
                                    data-move="up"
                                    disabled={r === 0}
                                    onclick={() => move(r, r - 1)}>↑</button
                                >
                                <button
                                    type="button"
                                    class="btn btn-square btn-ghost btn-xs"
                                    aria-label="Move row {r + 1} down"
                                    data-move="down"
                                    disabled={r === table.rows.length - 1}
                                    onclick={() => move(r, r + 1)}>↓</button
                                >
                                <button
                                    type="button"
                                    class="btn btn-square btn-ghost btn-xs"
                                    aria-label="Delete row {r + 1}"
                                    onclick={() => {
                                        onChange(removeRow(table, r));
                                        focusedRow = null;
                                    }}>✕</button
                                >
                            {/if}
                        </td>
                    </tr>
                {:else}
                    <tr><td colspan={table.columns.length + 2} class="text-base-content/60">No items yet.</td></tr>
                {/each}
            </tbody>
        </table>
    </div>
    <div class="sr-only" aria-live="polite">{announce}</div>

    {#if !readonly}
        <div>
            <button type="button" class="btn btn-outline btn-xs" onclick={add} data-testid="add-catalog-item">Add item</button>
        </div>
    {/if}

    {#if sorted.length > 0}
        <ul class="flex flex-col gap-0.5 text-sm text-error" data-testid="catalog-problems">
            {#each sorted as problem, at (at)}
                <li>Row {problem.row + 1}: {problem.message}</li>
            {/each}
        </ul>
    {/if}
</section>
