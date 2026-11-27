<!-- The bill table as a grid (invoicer PLAN 4a, ARCHITECTURE §3.3, §6.3): one
     row per table row, a field per column by role, the raw markdown cells the
     file holds. Discounts, subtotals and taxes are positional rows (I3), so
     the row-type buttons add rows and reordering moves them; each adjustment
     says what it applies to.

     Row shapes (Phase 4 feedback H6–H8): an item, discount or tax row has a
     field per column; a subtotal is its label (one of the books' total words)
     and its amount; a spacer is an empty separator with only its handle and
     buttons. The final total isn't a row here: it is the footer line, always
     there, its label chosen from the books' total words; the table gets a
     total row when a row is first added, if it had none.

     Amounts are the server's (`POST …/compute`). While an answer is on its
     way the grid shows its own estimate, muted and italic, and replaces it
     when the server answers.

     Keyboard: Tab through the cells; in a description or code cell, ArrowDown
     (or a click) opens the catalog; Alt+ArrowUp / Alt+ArrowDown moves the
     focused row; each row also has Move up, Move down and Delete buttons.
     Alt+Shift+I, D, F, S, T and B add an item, a % discount, a fixed
     discount, a subtotal, a tax and a spacer, from any field on the page,
     and focus the new row (keymap bindings, PLAN 5k Y7). A
     pointer can drag a row by its handle; a line shows where it will land.
     New rows always go at the bottom, above the total (H9). -->
<script lang="ts">
    import {tick} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import Combobox from "#ui/components/Combobox.svelte";
    import {formatKeys} from "#ui/keys/chord.js";
    import {hint} from "#ui/keys/hint.svelte.js";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import {formatMoney, plain, type LocalTable} from "#lib/arithmetic.js";
    import {
        allTotalWords,
        baseText,
        columnOf,
        dropIndex,
        ensureColumn,
        ensureTotal,
        fillFromCatalog,
        finalTotal,
        insertCells,
        insertRow,
        moveRow,
        removeRow,
        setCell,
        setTotalLabel,
        totalLabel,
        type CatalogItem,
        type GridForm,
        type NewRowKind,
        type OpenDeposits,
        type Role,
        type RowView,
        type TableView,
        type TotalWords,
        type Words,
    } from "#lib/editor.js";
    import {moveRowKeys} from "#lib/keys/gridKeys.js";
    import {errorText} from "#lib/labels.js";
    import DeferralPopover from "./DeferralPopover.svelte";

    let {
        table,
        view = null,
        local,
        pending = false,
        accounts = [],
        catalog = [],
        words,
        labels = {total: [words.total], subtotal: [words.subtotal]},
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
        /** The words total and subtotal rows may be labelled with. */
        labels?: TotalWords;
        commodity?: string;
        readonly?: boolean;
        /** The customer's open deposits, for Apply deposit. */
        loadDeposits?: () => Promise<OpenDeposits>;
        onChange: (table: GridForm) => void;
    } = $props();

    let dragFrom = $state<number | null>(null);
    /** Where a dragged row would land: before this row index. */
    let dropBefore = $state<number | null>(null);
    let gridEl = $state<HTMLTableSectionElement | null>(null);
    let announce = $state("");

    let deposits = $state<OpenDeposits | null>(null);
    let depositsOpen = $state(false);
    let depositsError = $state<string | null>(null);

    const extra = $derived(allTotalWords(labels));
    // The server's rows describe `table` only when they line up with it.
    const serverRows = $derived<RowView[] | null>(!pending && view !== null && view.rows.length === table.rows.length ? view.rows : null);
    /** The final total's row (the footer), or -1. */
    const totalRow = $derived(finalTotal(table, words, extra));
    const visible = $derived(table.rows.map((_, r) => r).filter((r) => r !== totalRow));
    const amountColumn = $derived(columnOf(table, "amount"));
    const footLabel = $derived(totalRow >= 0 ? totalLabel(table, totalRow, words, extra) : words.total);

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

    /** Each column's width: the description takes what's left; the rest fit their content and never crush (the grid scrolls instead). */
    const WIDTH: Record<Role, string> = {
        description: "w-full min-w-56",
        quantity: "w-20 min-w-20",
        rate: "w-28 min-w-28",
        amount: "w-28 min-w-28",
        date: "w-36 min-w-36",
        unit: "w-20 min-w-20",
        item: "w-28 min-w-28",
        books: "w-52 min-w-52",
        other: "w-32 min-w-32",
    };

    const catalogByDescription = $derived<ComboOption[]>(
        catalog.map((c) => ({value: c.description, label: c.description, detail: `${c.code}${c.rate ? ` · ${c.rate}` : ""}`}))
    );
    const catalogByCode = $derived<ComboOption[]>(catalog.map((c) => ({value: c.code, label: c.code, detail: c.description})));

    // Structure (which rows are totals, spacers) is the local reading: it is always in step with `table`.
    const kindOf = (row: number): string => local.rows[row]?.kind ?? "item";

    /** The row's amount: the server's, else the display-only one (flagged). */
    function amountOf(row: number): {text: string; optimistic: boolean} {
        if (serverRows !== null) return {text: serverRows[row]?.amount ?? "", optimistic: false};
        const cents = local.rows[row]?.cents ?? null;
        return {text: cents === null ? "" : formatMoney(cents, commodity), optimistic: true};
    }

    /** The amount due: the final total's, else the table's sum. */
    const total = $derived.by((): {text: string; optimistic: boolean} => {
        if (serverRows !== null && view !== null) return {text: view.total, optimistic: false};
        return {text: formatMoney(local.total, commodity), optimistic: true};
    });

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
                ? totalLabel(table, r.appliesTo, words, extra) || "the total"
                : plain(table.rows[r.appliesTo][columnOf(table, "description")] ?? "") || `row ${r.appliesTo + 1}`;
        return `of ${name}${target.cents !== null ? `, ${formatMoney(target.cents, commodity)}` : ""}`;
    }

    const update = (row: number, role: Role, text: string): void => onChange(setCell(table, row, role, text));

    /** The label choices for a total row: the books' words, and its own label if it is another. */
    function labelChoices(current: string, finalRow: boolean): string[] {
        const list = finalRow ? [...labels.total, ...labels.subtotal] : [...labels.subtotal, ...labels.total];
        const unique = [...new Set(list)];
        return current === "" || unique.some((w) => w.toLowerCase() === current.toLowerCase()) ? unique : [current, ...unique];
    }

    function relabel(row: number, label: string): void {
        onChange(setTotalLabel(table, row, label, words, extra));
    }

    function relabelTotal(label: string): void {
        const totalled = ensureTotal(table, words, extra);
        onChange(setTotalLabel(totalled, finalTotal(totalled, words, extra), label, words, extra));
    }

    async function focusRow(row: number, column: number): Promise<void> {
        await tick();
        const target = gridEl?.querySelector<HTMLElement>(
            `tr[data-row="${row}"] [data-col="${column}"] input, tr[data-row="${row}"] [data-col="${column}"] select, tr[data-row="${row}"] [data-col="${column}"] textarea`
        );
        (target ?? gridEl?.querySelector<HTMLElement>(`tr[data-row="${row}"] select, tr[data-row="${row}"] button[data-move="up"]`))?.focus();
    }

    function move(from: number, to: number, column = 0): void {
        if (to < 0 || to >= table.rows.length || (totalRow >= 0 && to >= totalRow)) return;
        onChange(moveRow(table, from, to));
        announce = `Row ${from + 1} moved to ${to + 1}`;
        void focusRow(to, column);
    }

    function onDragOver(event: DragEvent, row: number): void {
        if (dragFrom === null) return;
        event.preventDefault();
        const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
        const lower = event.clientY > box.top + box.height / 2;
        let before = lower ? row + 1 : row;
        // Never below the final total.
        if (totalRow >= 0 && before > totalRow) before = totalRow;
        dropBefore = before;
    }

    function onDrop(event: DragEvent): void {
        event.preventDefault();
        if (dragFrom !== null && dropBefore !== null) {
            const to = dropIndex(dragFrom, dropBefore);
            if (to !== dragFrom) move(dragFrom, to);
        }
        dragFrom = null;
        dropBefore = null;
    }

    /** The drop line on row `row`: above it, or below it (the last place before the total). */
    function dropMark(row: number): "before" | "after" | undefined {
        if (dragFrom === null || dropBefore === null) return undefined;
        const at = dropIndex(dragFrom, dropBefore);
        if (at === dragFrom) return undefined;
        if (dropBefore === row) return "before";
        const lastShown = visible.at(-1);
        return dropBefore === row + 1 && row === lastShown ? "after" : undefined;
    }

    function add(kind: NewRowKind): void {
        const {table: next, at} = insertRow(table, kind, words, commodity, extra);
        onChange(next);
        void focusRow(at, Math.max(0, columnOf(next, "description")));
    }

    function addColumn(role: "unit" | "item"): void {
        onChange(ensureColumn(table, role, words));
    }

    async function openDeposits(): Promise<void> {
        if (loadDeposits === undefined) return;
        depositsOpen = true;
        depositsError = null;
        deposits = null;
        try {
            deposits = await loadDeposits();
        } catch (error) {
            depositsError = errorText(error);
        }
    }

    function applyDeposit(row: {description: string; amount: string; books: string}): void {
        const totalled = ensureTotal(table, words, extra);
        onChange(insertCells(totalled, finalTotal(totalled, words, extra), {description: row.description, amount: row.amount, books: row.books}));
        depositsOpen = false;
    }

    function chooseCatalog(row: number, item: CatalogItem | undefined): void {
        if (item !== undefined) onChange(fillFromCatalog(table, row, item, words));
    }

    /** The row-type buttons, and their keys (Alt+Shift+letter: Option+letter alone types ß and accents on a Mac). */
    const ROW_BUTTONS: {kind: NewRowKind; label: string; keys: string; help: string}[] = [
        {kind: "item", label: "Item", keys: "alt+I", help: "Add an item row"},
        {kind: "discount-percent", label: "Discount %", keys: "alt+D", help: "Add a % discount row"},
        {kind: "discount-fixed", label: "Discount $", keys: "alt+F", help: "Add a fixed discount row"},
        {kind: "subtotal", label: "Subtotal", keys: "alt+S", help: "Add a subtotal row"},
        {kind: "tax", label: "Tax", keys: "alt+T", help: "Add a tax row"},
        {kind: "spacer", label: "Spacer", keys: "alt+B", help: "Add a spacer (blank) row"},
    ];

    registerKeys({
        id: "bill-grid",
        priority: PRIORITY.widget,
        bindings: [
            ...ROW_BUTTONS.map((b) => ({
                keys: b.keys,
                label: b.help,
                group: "Bill table",
                fields: true,
                enabled: () => !readonly,
                run: () => add(b.kind),
            })),
            ...moveRowKeys({group: "Bill table", grid: () => gridEl, readonly: () => readonly, move}),
        ],
    });

    const numeric = (role: Role): boolean => role === "quantity" || role === "rate" || role === "amount";
    /** The cells before the amount column, merged for a subtotal's label (all of them when there's no amount column). */
    const leading = $derived(amountColumn < 0 ? table.columns.length : amountColumn);
</script>

{#snippet handle(r: number)}
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
                    if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
                }}
                ondragend={() => {
                    dragFrom = null;
                    dropBefore = null;
                }}>⠿</span
            >
        {/if}
    </td>
{/snippet}

{#snippet actions(r: number)}
    <td class="w-24 align-top whitespace-nowrap">
        {#if !readonly}
            <button
                type="button"
                class="btn btn-square btn-ghost btn-xs"
                aria-label="Move row {r + 1} up"
                data-move="up"
                disabled={r === visible[0]}
                onclick={() => move(r, r - 1)}>↑</button
            >
            <button
                type="button"
                class="btn btn-square btn-ghost btn-xs"
                aria-label="Move row {r + 1} down"
                data-move="down"
                disabled={r === visible.at(-1) || (totalRow >= 0 && r + 1 >= totalRow)}
                onclick={() => move(r, r + 1)}>↓</button
            >
            <button type="button" class="btn btn-square btn-ghost btn-xs" aria-label="Delete row {r + 1}" onclick={() => onChange(removeRow(table, r))}
                >✕</button
            >
        {/if}
    </td>
{/snippet}

{#snippet amountCell(r: number, label: string)}
    {@const amount = amountOf(r)}
    <output
        aria-label={label}
        class="block text-right {amount.optimistic ? 'text-base-content/60 italic' : ''}"
        data-testid="grid-amount"
        data-optimistic={amount.optimistic}>{amount.text}</output
    >
    {#if amount.optimistic}<span class="sr-only">(updating)</span>{/if}
{/snippet}

<section class="flex min-w-0 flex-col gap-2" aria-label="Bill table" data-testid="bill-grid">
    <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table table-sm">
            <thead>
                <tr>
                    <th class="w-8"><span class="sr-only">Move</span></th>
                    {#each table.columns as column, c (c)}
                        <th class="{WIDTH[column.role]} {numeric(column.role) ? 'text-right' : ''}" data-role={column.role}>
                            {column.role === "books" ? "Account" : plain(column.header.replace(/^\*(.*)\*$/, "$1"))}
                        </th>
                    {/each}
                    <th class="w-24"><span class="sr-only">Row actions</span></th>
                </tr>
            </thead>
            <tbody bind:this={gridEl}>
                {#each visible as r (r)}
                    {@const cells = table.rows[r]}
                    {@const kind = kindOf(r)}
                    {@const adjustment = base(r)}
                    <tr
                        data-row={r}
                        data-kind={kind === "total" ? "subtotal" : kind}
                        data-drop={dropMark(r)}
                        data-testid="grid-row"
                        class="{kind === 'total' ? 'bg-base-200/60 font-semibold' : ''} {dragFrom === r ? 'opacity-50' : ''}"
                        ondragover={(event) => onDragOver(event, r)}
                        ondrop={onDrop}
                    >
                        {@render handle(r)}
                        {#if kind === "spacer"}
                            <td colspan={table.columns.length} class="align-middle" data-col="0">
                                <div class="flex items-center gap-2 text-xs text-base-content/40" data-testid="grid-spacer">
                                    <span class="flex-1 border-t border-dashed border-base-300"></span>
                                    <span>Spacer</span>
                                    <span class="flex-1 border-t border-dashed border-base-300"></span>
                                </div>
                            </td>
                        {:else if kind === "total"}
                            {@const label = totalLabel(table, r, words, extra)}
                            <td colspan={leading} class="text-right align-top" data-col="0">
                                {#if readonly}
                                    <span>{label}</span>
                                {:else}
                                    <select
                                        class="select w-auto select-ghost text-right font-semibold select-xs"
                                        aria-label="Row {r + 1} subtotal label"
                                        value={label}
                                        onchange={(event) => relabel(r, event.currentTarget.value)}
                                        data-testid="grid-subtotal-label"
                                    >
                                        {#each labelChoices(label, false) as word (word)}<option value={word}>{word}</option>{/each}
                                    </select>
                                {/if}
                            </td>
                            {#if amountColumn >= 0}
                                <td class="text-right align-top whitespace-nowrap tabular-nums" data-col={amountColumn}
                                    >{@render amountCell(r, `Row ${r + 1} amount`)}</td
                                >
                                {#if table.columns.length > amountColumn + 1}<td colspan={table.columns.length - amountColumn - 1}></td>{/if}
                            {/if}
                        {:else}
                            {#each table.columns as column, c (c)}
                                {@const label = `Row ${r + 1} ${LABEL[column.role].toLowerCase()}`}
                                <td data-col={c} class="align-top {column.role === 'amount' ? 'text-right whitespace-nowrap tabular-nums' : ''}">
                                    {#if column.role === "amount" && computedAmount(r)}
                                        {@render amountCell(r, label)}
                                    {:else if column.role === "amount"}
                                        <input
                                            class="input w-full min-w-24 text-right tabular-nums input-xs"
                                            aria-label={label}
                                            value={cells[c]}
                                            disabled={readonly}
                                            oninput={(event) => update(r, "amount", event.currentTarget.value)}
                                            data-testid="grid-amount-input"
                                        />
                                    {:else if column.role === "description" && catalog.length > 0 && !readonly}
                                        <Combobox
                                            bind:value={() => cells[c], (v) => update(r, "description", v)}
                                            options={catalogByDescription}
                                            {label}
                                            size="xs"
                                            chevron={false}
                                            passAltArrows
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
                                            chevron={false}
                                            passAltArrows
                                            onChoose={(option) =>
                                                chooseCatalog(
                                                    r,
                                                    catalog.find((i) => i.code === option.value)
                                                )}
                                        />
                                    {:else if column.role === "books"}
                                        <div class="flex items-start gap-1">
                                            <div class="min-w-0 flex-1" title={cells[c] === "" ? "The default account" : cells[c]}>
                                                <AccountInput
                                                    passAltArrows
                                                    bind:value={() => cells[c], (v) => update(r, "books", v)}
                                                    accountNames={accounts}
                                                    {label}
                                                    placeholder="default"
                                                    size="xs"
                                                    disabled={readonly}
                                                />
                                            </div>
                                            {#if !readonly}
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
                                            class="input w-full input-xs {numeric(column.role) ? 'text-right tabular-nums' : ''} {column.role === 'item'
                                                ? 'font-mono'
                                                : ''}"
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
                        {/if}
                        {@render actions(r)}
                    </tr>
                {:else}
                    <tr>
                        <td colspan={table.columns.length + 2} class="text-base-content/60" data-testid="grid-empty">No items yet: add one below.</td>
                    </tr>
                {/each}
            </tbody>
            <tfoot>
                <tr class="border-t-2 border-base-300 text-sm text-base-content" data-testid="grid-total-row">
                    <td></td>
                    <td colspan={leading} class="text-right">
                        {#if readonly}
                            <span class="font-semibold">{footLabel}</span>
                        {:else}
                            <select
                                class="select w-auto select-ghost text-right font-semibold select-sm"
                                aria-label="Total label"
                                title="The total's label"
                                value={footLabel}
                                onchange={(event) => relabelTotal(event.currentTarget.value)}
                                data-testid="grid-total-label"
                            >
                                {#each labelChoices(footLabel, true) as word (word)}<option value={word}>{word}</option>{/each}
                            </select>
                        {/if}
                    </td>
                    <td class="text-right font-semibold whitespace-nowrap tabular-nums">
                        <output
                            aria-label="Total"
                            class={total.optimistic ? "text-base-content/60 italic" : ""}
                            data-testid="grid-total"
                            data-optimistic={total.optimistic}>{total.text}</output
                        >
                    </td>
                    {#if table.columns.length > leading + 1}<td colspan={table.columns.length - leading - 1}></td>{/if}
                    <td></td>
                </tr>
            </tfoot>
        </table>
    </div>
    <div class="sr-only" aria-live="polite">{announce}</div>

    {#if !readonly}
        <div class="flex flex-wrap items-center gap-1" role="toolbar" aria-label="Add a row">
            <span class="mr-1 text-xs text-base-content/60">Add</span>
            {#each ROW_BUTTONS as button (button.kind)}
                <button
                    type="button"
                    class="btn btn-outline btn-xs"
                    title={hint(button.help, formatKeys(button.keys)[0].text)}
                    onclick={() => add(button.kind)}
                    data-testid="add-{button.kind}">{button.label}</button
                >
            {/each}
            {#if loadDeposits !== undefined}
                <button type="button" class="btn btn-outline btn-xs" onclick={openDeposits} data-testid="add-deposit">Apply deposit</button>
            {/if}
            {#if columnOf(table, "unit") < 0 || columnOf(table, "item") < 0}
                <span class="mr-1 ml-2 text-xs text-base-content/60">Column</span>
                {#if columnOf(table, "unit") < 0}
                    <button type="button" class="btn btn-ghost btn-xs" onclick={() => addColumn("unit")} data-testid="add-unit-column">+ {words.unit}</button>
                {/if}
                {#if columnOf(table, "item") < 0}
                    <button type="button" class="btn btn-ghost btn-xs" onclick={() => addColumn("item")} data-testid="add-code-column">+ {words.item}</button>
                {/if}
            {/if}
        </div>
    {/if}

    {#if depositsOpen}
        <div class="rounded-box border border-base-300 bg-base-200 p-3 text-sm" role="region" aria-label="Open deposits" data-testid="deposits">
            {#if depositsError !== null}
                <p class="text-error-text">{depositsError}</p>
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
