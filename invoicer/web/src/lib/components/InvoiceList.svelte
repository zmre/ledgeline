<!-- The invoice list, read-only (invoicer PLAN 3b): `GET /api/invoices` as
     the server filters and sorts it (PLAN 5b). With `onSort`, the headers sort
     (SortHeader: a button each, `aria-sort` on the `<th>`); overdue rows are
     tinted, not only their badge. With `filtered`, an empty list says nothing
     matches and offers Clear filters, instead of the first-invoice card.

     A click anywhere on a row opens its invoice (Patrick, 2026-11-23), except
     on a link or button inside it, which do their own thing. The row itself
     isn't made a link (a row can't wrap the number's link without nesting
     interactive elements): the number is the row's one link, so the keyboard
     reaches each invoice with Tab and opens it with Enter, and a screen reader
     reads one link per row. The row click is the pointer's shortcut to it. -->
<script lang="ts">
    import {kindLabel, type InvoiceEntry} from "#lib/invoices.js";
    import type {SortDir} from "#lib/listQuery.js";
    import {rowClick} from "#lib/rows.js";
    import EmptyState from "./EmptyState.svelte";
    import SortHeader from "./SortHeader.svelte";
    import StatusBadge from "./StatusBadge.svelte";

    let {
        invoices,
        href,
        onOpen,
        newHref = "new",
        shown = {sort: "date", dir: "desc"},
        onSort,
        filtered = false,
        onClear,
    }: {
        invoices: InvoiceEntry[];
        href: (doc: InvoiceEntry) => string;
        onOpen?: (doc: InvoiceEntry) => void;
        newHref?: string;
        /** The column the rows are sorted by (the server's order is Date, newest first). */
        shown?: {sort: string; dir: SortDir};
        /** Sort by a column; without it the headers are plain text. */
        onSort?: (column: string) => void;
        /** A filter narrows the list: empty means "nothing matches", not "no invoices yet". */
        filtered?: boolean;
        onClear?: () => void;
    } = $props();

    const COLUMNS: {column: string; label: string; align?: "right"}[] = [
        {column: "number", label: "Number"},
        {column: "customer", label: "Customer"},
        {column: "date", label: "Date"},
        {column: "due", label: "Due"},
        {column: "total", label: "Total", align: "right"},
        {column: "status", label: "Status"},
    ];
</script>

{#if invoices.length === 0 && filtered}
    <div class="rounded-box border border-base-300 px-4 py-8 text-center" data-testid="invoice-list-no-match">
        <p class="text-base-content/70">No invoices match these filters.</p>
        {#if onClear !== undefined}
            <button type="button" class="btn mt-3 btn-outline btn-sm" onclick={onClear} data-testid="invoice-list-clear">Clear filters</button>
        {/if}
    </div>
{:else if invoices.length === 0}
    <EmptyState
        title="No invoices yet"
        text="Invoices you write here are drafts until you send them; sending books them in your journal."
        action="New invoice"
        href={newHref}
        testid="invoice-list-empty"
        actionTestid="invoice-list-new"
    />
{:else}
    <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table table-sm" data-testid="invoice-list">
            <thead>
                <tr>
                    {#each COLUMNS as c (c.column)}
                        {#if onSort !== undefined}
                            <SortHeader column={c.column} label={c.label} {shown} {onSort} align={c.align} />
                        {:else}
                            <th class={c.align === "right" ? "text-right" : ""}>{c.label}</th>
                        {/if}
                    {/each}
                </tr>
            </thead>
            <tbody>
                {#each invoices as doc (doc.path)}
                    {@const kind = kindLabel(doc.doc_type)}
                    <!-- The pointer's shortcut; the number's link is the keyboard's way in (see the top). -->
                    <tr
                        class="cursor-pointer {doc.overdue ? 'bg-error/8 hover:bg-error/15' : 'hover:bg-base-200'}"
                        data-testid="invoice-row"
                        data-number={doc.number}
                        data-overdue={doc.overdue ? "true" : undefined}
                        onclick={(event) => rowClick(event, onOpen === undefined ? undefined : () => onOpen(doc))}
                    >
                        <td class="font-medium whitespace-nowrap">
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes `href` built with resolve() -->
                            <a class="link link-hover" href={href(doc)} data-row-link>{doc.number ?? doc.title}</a>
                            {#if kind !== null}<span class="ml-1 text-xs text-base-content/60">{kind}</span>{/if}
                        </td>
                        <td>{doc.invoicee.name ?? "—"}</td>
                        <td class="whitespace-nowrap tabular-nums">{doc.date ?? "—"}</td>
                        <td class="whitespace-nowrap tabular-nums {doc.overdue ? 'font-medium text-error' : ''}">{doc.due_date ?? "—"}</td>
                        <td class="text-right whitespace-nowrap tabular-nums">{doc.total ?? "—"}</td>
                        <td>
                            <StatusBadge {doc} />
                            {#if doc.problems.errors > 0}
                                <span class="ml-1 badge badge-outline badge-xs badge-error" title="ledgeline-invoicer check lists them"
                                    >{doc.problems.errors} {doc.problems.errors === 1 ? "error" : "errors"}</span
                                >
                            {/if}
                        </td>
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>
{/if}
