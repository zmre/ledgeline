<!-- The invoice list, read-only (invoicer PLAN 3b): `GET /api/invoices` as
     the server sorts it. Filters and sorting are Phase 5.

     A click anywhere on a row opens its invoice (Patrick, 2026-11-23), except
     on a link or button inside it, which do their own thing. The row itself
     isn't made a link (a row can't wrap the number's link without nesting
     interactive elements): the number is the row's one link, so the keyboard
     reaches each invoice with Tab and opens it with Enter, and a screen reader
     reads one link per row. The row click is the pointer's shortcut to it. -->
<script lang="ts">
    import {kindLabel, type InvoiceEntry} from "#lib/invoices.js";
    import EmptyState from "./EmptyState.svelte";
    import StatusBadge from "./StatusBadge.svelte";

    let {
        invoices,
        href,
        onOpen,
        newHref = "new",
    }: {invoices: InvoiceEntry[]; href: (doc: InvoiceEntry) => string; onOpen?: (doc: InvoiceEntry) => void; newHref?: string} = $props();

    /** Open `doc` from a click on its row, unless the click was on a control, or selected text. */
    function rowClick(event: MouseEvent, doc: InvoiceEntry): void {
        const target = event.target as Element | null;
        if (target?.closest("a, button, input, select, textarea, label, summary") ?? false) return;
        if ((window.getSelection?.()?.toString() ?? "") !== "") return;
        if (onOpen !== undefined) {
            onOpen(doc);
            return;
        }
        // Without a handler: follow the number's own link (same tab rules).
        (event.currentTarget as HTMLElement).querySelector<HTMLAnchorElement>("a[data-row-link]")?.click();
    }
</script>

{#if invoices.length === 0}
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
                    <th>Number</th>
                    <th>Customer</th>
                    <th>Date</th>
                    <th>Due</th>
                    <th class="text-right">Total</th>
                    <th>Status</th>
                </tr>
            </thead>
            <tbody>
                {#each invoices as doc (doc.path)}
                    {@const kind = kindLabel(doc.doc_type)}
                    <!-- The pointer's shortcut; the number's link is the keyboard's way in (see the top). -->
                    <tr class="cursor-pointer hover:bg-base-200" data-testid="invoice-row" data-number={doc.number} onclick={(event) => rowClick(event, doc)}>
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
