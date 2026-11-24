<!-- The bill table as a grid (invoicer PLAN 4a, ARCHITECTURE §3.3, §6.3): one
     row per table row, a field per column by role, the raw markdown cells the
     file holds. Discounts, subtotals and taxes are positional rows (I3), so
     the row-type buttons add rows and reordering moves them; each adjustment
     says what it applies to.

     Amounts are the server's (`POST …/compute`). While an answer is on its
     way the grid shows its own display-only arithmetic (lib/arithmetic.ts),
     muted, italic and labelled "calculating", and replaces it when the server
     answers.

     Keyboard: Tab through the cells; Alt+ArrowUp / Alt+ArrowDown moves the
     focused row; each row also has Move up, Move down and Delete buttons. A
     pointer can drag a row by its handle. New rows go after the focused row,
     else above the final total (rows below it can't be booked). -->
<script lang="ts">
    import {tick} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import Combobox from "#ui/components/Combobox.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import {formatMoney, plain, type LocalTable} from "#lib/arithmetic.js";
    import {
        baseText,
        columnOf,
        fillFromCatalog,
        finalTotal,
        insertCells,
        insertRow,
        moveRow,
        removeRow,
        setCell,
        type CatalogItem,
        type GridForm,
        type NewRowKind,
        type OpenDeposits,
        type Role,
        type RowView,
        type TableView,
        type Words,
    } from "#lib/editor.js";
    import DeferralPopover from "./DeferralPopover.svelte";

    let {
        table,
        view = null,
        local,
        pending = false,
        accounts = [],
        catalog = [],
        words,
        commodity = "$",
        readonly = false,
        loadDeposits,
        onChange,
    }: {
        table: GridForm;
        /** The server's computed table for `table` (null while none has answered, or it doesn't read). */
        view?: TableView | null;
        /** The display-only amounts for `table`, shown while `pending`. */
        local: LocalTable;
        /** The server hasn't answered for the table as it is now. */
        pending?: boolean;
        accounts?: string[];
        catalog?: CatalogItem[];
        words: Words;
        commodity?: string;
        readonly?: boolean;
        /** The customer's open deposits, for Apply deposit. */
        loadDeposits?: () => Promise<OpenDeposits>;
        onChange: (table: GridForm) => void;
    } = $props();

    let focusedRow = $state<number | null>(null);
    let dragFrom = $state<number | null>(null);
    let gridEl = $state<HTMLTableSectionElement | null>(null);
    let announce = $state("");

    let deposits = $state<OpenDeposits | null>(null);
    let depositsOpen = $state(false);
    let depositsError = $state<string | null>(null);

    // The server's rows describe `table` only when they line up with it.
    const serverRows = $derived<RowView[] | null>(!pending && view !== null && view.rows.length === table.rows.length ? view.rows : null);

    const LABEL: Record<Role, string> = {
        description: "Description",
        quantity: "Quantity",
        rate: "Rate",
        amount: "Amount",
        date: "Date",
        unit: "Unit",
        item: "Code",
        books: "Account",
        other: "Column",
    };

    const catalogByDescription = $derived<ComboOption[]>(
        catalog.map((c) => ({value: c.description, label: c.description, detail: `${c.code}${c.rate ? ` · ${c.rate}` : ""}`}))
    );
    const catalogByCode = $derived<ComboOption[]>(catalog.map((c) => ({value: c.code, label: c.code, detail: c.description})));

    const kindOf = (row: number): string => serverRows?.[row]?.kind ?? local.rows[row]?.kind ?? "item";

    /** The row's amount: the server's, else the display-only one (flagged). */
    function amountOf(row: number): {text: string; optimistic: boolean} {
        if (serverRows !== null) return {text: serverRows[row]?.amount ?? "", optimistic: false};
        const cents = local.rows[row]?.cents ?? null;
        return {text: cents === null ? "" : formatMoney(cents, commodity), optimistic: true};
    }

    /** Whether the amount cell is computed (read-only): totals, percentages, qty × rate. */
    function computedAmount(row: number): boolean {
        if (serverRows !== null) return serverRows[row]?.computed ?? false;
        const kind = local.rows[row]?.kind;
        if (kind === "total" || kind === "adjustment") return true;
        const q = table.rows[row]?.[columnOf(table, "quantity")] ?? "";
        const r = table.rows[row]?.[columnOf(table, "rate")] ?? "";
        return kind === "item" && plain(q) !== "" && plain(r) !== "";
    }

    function base(row: number): string | null {
        if (serverRows !== null) {
            const r = serverRows[row];
            return r === undefined ? null : baseText(serverRows, r);
        }
        const r = local.rows[row];
        if (r?.kind !== "adjustment" || r.appliesTo === null) return null;
        const target = local.rows[r.appliesTo];
        const name =
            target.kind === "total"
                ? plain(table.rows[r.appliesTo][columnOf(table, "rate")] ?? "") || "the total"
                : plain(table.rows[r.appliesTo][columnOf(table, "description")] ?? "") || `row ${r.appliesTo + 1}`;
        return `of ${name}${target.cents !== null ? `, ${formatMoney(target.cents, commodity)}` : ""}`;
    }

    const update = (row: number, role: Role, text: string): void => onChange(setCell(table, row, role, text));

    async function focusRow(row: number, column: number): Promise<void> {
        await tick();
        const target = gridEl?.querySelector<HTMLElement>(
            `tr[data-row="${row}"] [data-col="${column}"] input, tr[data-row="${row}"] [data-col="${column}"] textarea`
        );
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

    function add(kind: NewRowKind): void {
        const {table: next, at} = insertRow(table, kind, focusedRow, words, commodity);
        onChange(next);
        focusedRow = at;
        void focusRow(at, Math.max(0, columnOf(next, kind === "subtotal" ? "rate" : "description")));
    }

    async function openDeposits(): Promise<void> {
        if (loadDeposits === undefined) return;
        depositsOpen = true;
        depositsError = null;
        deposits = null;
        try {
            deposits = await loadDeposits();
        } catch (error) {
            depositsError = error instanceof Error ? error.message : String(error);
        }
    }

    function applyDeposit(row: {description: string; amount: string; books: string}): void {
        const total = finalTotal(table, words);
        onChange(insertCells(table, total >= 0 ? total : table.rows.length, {description: row.description, amount: row.amount, books: row.books}));
        depositsOpen = false;
    }

    function chooseCatalog(row: number, item: CatalogItem | undefined): void {
        if (item !== undefined) onChange(fillFromCatalog(table, row, item, words));
    }

    const ROW_BUTTONS: {kind: NewRowKind; label: string}[] = [
        {kind: "item", label: "Item"},
        {kind: "discount-percent", label: "Discount %"},
        {kind: "discount-fixed", label: "Discount $"},
        {kind: "subtotal", label: "Subtotal"},
        {kind: "tax", label: "Tax"},
        {kind: "spacer", label: "Spacer"},
    ];
</script>

<section class="flex flex-col gap-2" aria-label="Bill table" data-testid="bill-grid">
    <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table table-sm">
            <thead>
                <tr>
                    <th class="w-8"><span class="sr-only">Move</span></th>
                    {#each table.columns as column, c (c)}
                        <th class={column.role === "quantity" || column.role === "rate" || column.role === "amount" ? "text-right" : ""}>
                            {column.role === "books" ? "Account" : plain(column.header.replace(/^\*(.*)\*$/, "$1"))}
                        </th>
                    {/each}
                    <th><span class="sr-only">Row actions</span></th>
                </tr>
            </thead>
            <tbody bind:this={gridEl}>
                {#each table.rows as cells, r (r)}
                    {@const kind = kindOf(r)}
                    {@const amount = amountOf(r)}
                    {@const adjustment = base(r)}
                    <tr
                        data-row={r}
                        data-kind={kind}
                        data-testid="grid-row"
                        class="{kind === 'total' ? 'font-semibold' : ''} {dragFrom === r ? 'opacity-50' : ''}"
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
                            {@const label = `Row ${r + 1} ${LABEL[column.role].toLowerCase()}`}
                            <td data-col={c} class="align-top {column.role === 'amount' ? 'text-right whitespace-nowrap tabular-nums' : ''}">
                                {#if column.role === "amount" && computedAmount(r)}
                                    <output
                                        aria-label={label}
                                        class={amount.optimistic ? "text-base-content/60 italic" : ""}
                                        title={amount.optimistic ? "Calculating: this app's estimate until the server answers" : undefined}
                                        data-testid="grid-amount"
                                        data-optimistic={amount.optimistic}>{amount.text}</output
                                    >
                                    {#if amount.optimistic}<span class="sr-only">(calculating)</span>{/if}
                                {:else if column.role === "amount"}
                                    <input
                                        class="input w-28 text-right tabular-nums input-xs"
                                        aria-label={label}
                                        value={cells[c]}
                                        disabled={readonly}
                                        oninput={(event) => update(r, "amount", event.currentTarget.value)}
                                        data-testid="grid-amount-input"
                                    />
                                {:else if column.role === "description" && kind !== "total" && catalog.length > 0 && !readonly}
                                    <Combobox
                                        bind:value={() => cells[c], (v) => update(r, "description", v)}
                                        options={catalogByDescription}
                                        {label}
                                        size="xs"
                                        onChoose={(option) =>
                                            chooseCatalog(
                                                r,
                                                catalog.find((i) => i.description === option.value)
                                            )}
                                    />
                                {:else if column.role === "item" && !readonly}
                                    <Combobox
                                        bind:value={() => cells[c], (v) => update(r, "item", v)}
                                        options={catalogByCode}
                                        {label}
                                        size="xs"
                                        onChoose={(option) =>
                                            chooseCatalog(
                                                r,
                                                catalog.find((i) => i.code === option.value)
                                            )}
                                    />
                                {:else if column.role === "books"}
                                    <div class="flex items-start gap-1">
                                        <div class="min-w-48 flex-1">
                                            <AccountInput
                                                bind:value={() => cells[c], (v) => update(r, "books", v)}
                                                accountNames={accounts}
                                                {label}
                                                placeholder="default"
                                                size="xs"
                                                disabled={readonly}
                                            />
                                        </div>
                                        {#if !readonly && kind !== "total" && kind !== "spacer"}
                                            <DeferralPopover
                                                cell={cells[c]}
                                                {accounts}
                                                label="Row {r + 1} deferral"
                                                onApply={(text) => update(r, "books", text)}
                                            />
                                        {/if}
                                    </div>
                                {:else}
                                    <input
                                        class="input w-full input-xs {column.role === 'quantity' || column.role === 'rate' ? 'text-right tabular-nums' : ''}"
                                        aria-label={label}
                                        value={cells[c]}
                                        disabled={readonly}
                                        oninput={(event) => update(r, column.role, event.currentTarget.value)}
                                    />
                                {/if}
                                {#if column.role === "description" && adjustment !== null}
                                    <div class="mt-0.5 text-xs text-base-content/60" data-testid="grid-base">
                                        {plain(table.rows[r][columnOf(table, "rate")] ?? "")}
                                        {adjustment}
                                    </div>
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
                {/each}
            </tbody>
        </table>
    </div>
    <div class="sr-only" aria-live="polite">{announce}</div>

    {#if !readonly}
        <div class="flex flex-wrap items-center gap-1" role="toolbar" aria-label="Add a row">
            <span class="mr-1 text-xs text-base-content/60">Add</span>
            {#each ROW_BUTTONS as button (button.kind)}
                <button type="button" class="btn btn-outline btn-xs" onclick={() => add(button.kind)} data-testid="add-{button.kind}">{button.label}</button>
            {/each}
            {#if loadDeposits !== undefined}
                <button type="button" class="btn btn-outline btn-xs" onclick={openDeposits} data-testid="add-deposit">Apply deposit</button>
            {/if}
        </div>
    {/if}

    {#if depositsOpen}
        <div class="rounded-box border border-base-300 bg-base-200 p-3 text-sm" role="region" aria-label="Open deposits" data-testid="deposits">
            {#if depositsError !== null}
                <p class="text-error">{depositsError}</p>
            {:else if deposits === null}
                <span class="loading loading-sm loading-spinner" aria-label="Loading deposits"></span>
            {:else if deposits.deposits.length === 0}
                <p>{deposits.customer} has no open deposits on {deposits.account}.</p>
            {:else}
                <ul class="flex flex-col gap-1">
                    {#each deposits.deposits as deposit, d (d)}
                        <li class="flex flex-wrap items-center gap-2">
                            <button type="button" class="btn btn-primary btn-xs" onclick={() => applyDeposit(deposit.row)}>Apply</button>
                            <span>{deposit.description}</span>
                            <span class="text-base-content/60">{deposit.date} · {deposit.row.amount.replace(/^-/, "")} left</span>
                        </li>
                    {/each}
                </ul>
            {/if}
            <button type="button" class="btn mt-2 btn-ghost btn-xs" onclick={() => (depositsOpen = false)}>Close</button>
        </div>
    {/if}
</section>
