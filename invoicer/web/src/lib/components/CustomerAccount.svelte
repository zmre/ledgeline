<!-- A customer's account (invoicer PLAN 4b): what it owes, overdue, its last
     payment, then its invoices (each linking it from `customer:`, MBR's
     backlinks M1), newest first, with status, total and due date, and the
     other notes that link it. A row click opens the invoice. -->
<script lang="ts">
    import {balanceText, lastPaymentText, type CustomerDetail} from "#lib/customers.js";
    import type {InvoiceEntry} from "#lib/invoices.js";
    import StatusBadge from "./StatusBadge.svelte";

    let {
        customer,
        invoiceHref,
        pageHref,
        onOpen,
    }: {
        customer: Pick<CustomerDetail, "account" | "documents" | "backlinks" | "billing" | "title">;
        invoiceHref: (doc: InvoiceEntry) => string;
        /** MBR's page for a backlinked note (its URL as MBR serves it). */
        pageHref: (url: string) => string;
        onOpen?: (doc: InvoiceEntry) => void;
    } = $props();

    const invoicePaths = $derived(new Set(customer.documents.map((d) => d.path)));
    const others = $derived(customer.backlinks.filter((b) => b.path === null || !invoicePaths.has(b.path)));

    function rowClick(event: MouseEvent, doc: InvoiceEntry): void {
        const target = event.target as Element | null;
        if (target?.closest("a, button") ?? false) return;
        if (onOpen !== undefined) onOpen(doc);
        else (event.currentTarget as HTMLElement).querySelector<HTMLAnchorElement>("a[data-row-link]")?.click();
    }
</script>

<section class="flex flex-col gap-3" data-testid="customer-account">
    <div class="stats stats-vertical border border-base-300 sm:stats-horizontal">
        <div class="stat py-3">
            <div class="stat-title">Owed</div>
            <div class="stat-value text-xl tabular-nums" data-testid="account-balance">{balanceText(customer.account)}</div>
            {#if customer.account.overdue > 0}
                <div class="stat-desc font-medium text-error" data-testid="account-overdue">
                    {customer.account.overdue} overdue{customer.account.oldest_overdue === null ? "" : `, since ${customer.account.oldest_overdue}`}
                </div>
            {/if}
        </div>
        <div class="stat py-3">
            <div class="stat-title">Last payment</div>
            <div class="stat-value text-xl tabular-nums" data-testid="account-last-payment">{lastPaymentText(customer.account)}</div>
        </div>
    </div>
    {#if customer.billing.name !== customer.title}
        <p class="text-sm text-base-content/80">Invoices to {customer.title} are billed to {customer.billing.name}.</p>
    {/if}

    <h2 class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Invoices</h2>
    {#if customer.documents.length === 0}
        <p class="text-sm text-base-content/70" data-testid="customer-no-invoices">No invoices name this customer yet.</p>
    {:else}
        <div class="overflow-x-auto rounded-box border border-base-300">
            <table class="table table-sm" data-testid="customer-invoices">
                <thead>
                    <tr><th>Number</th><th>Date</th><th>Due</th><th class="text-right">Total</th><th>Status</th></tr>
                </thead>
                <tbody>
                    {#each customer.documents as doc (doc.path)}
                        <tr
                            class="cursor-pointer hover:bg-base-200"
                            data-testid="customer-invoice-row"
                            data-number={doc.number}
                            onclick={(e) => rowClick(e, doc)}
                        >
                            <td class="font-medium">
                                <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes `invoiceHref` built with resolve() -->
                                <a class="link link-hover" href={invoiceHref(doc)} data-row-link>{doc.number ?? doc.title}</a>
                            </td>
                            <td class="whitespace-nowrap tabular-nums">{doc.date ?? "—"}</td>
                            <td class="whitespace-nowrap tabular-nums {doc.overdue ? 'font-medium text-error' : ''}">{doc.due_date ?? "—"}</td>
                            <td class="text-right whitespace-nowrap tabular-nums">{doc.total ?? "—"}</td>
                            <td><StatusBadge {doc} /></td>
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
    {/if}

    {#if others.length > 0}
        <h2 class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Also linked from</h2>
        <ul class="list-disc pl-5 text-sm" data-testid="customer-backlinks">
            {#each others as link (link.url)}
                <li>
                    <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- MBR's page, outside the SPA -->
                    <a class="link" href={pageHref(link.url)}>{link.title ?? link.url}</a>
                    {#if link.key !== null}<span class="text-xs text-base-content/60">({link.key})</span>{/if}
                </li>
            {/each}
        </ul>
    {/if}
</section>
