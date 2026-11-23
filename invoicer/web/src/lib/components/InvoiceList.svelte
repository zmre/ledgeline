<!-- The invoice list, read-only (invoicer PLAN 3b): `GET /api/invoices` as
     the server sorts it. Filters and sorting are Phase 5. -->
<script lang="ts">
    import {kindLabel, type InvoiceEntry} from "#lib/invoices.js";
    import StatusBadge from "./StatusBadge.svelte";

    let {invoices, href}: {invoices: InvoiceEntry[]; href: (doc: InvoiceEntry) => string} = $props();
</script>

{#if invoices.length === 0}
    <div class="rounded-box border border-dashed border-base-300 p-8 text-center text-base-content/70" data-testid="invoice-list-empty">
        No invoices yet. <code>ledgeline-invoicer new --customer …</code> makes one.
    </div>
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
                    <tr class="hover:bg-base-200" data-testid="invoice-row" data-number={doc.number}>
                        <td class="font-medium whitespace-nowrap">
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes `href` built with resolve() -->
                            <a class="link link-hover" href={href(doc)}>{doc.number ?? doc.title}</a>
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
