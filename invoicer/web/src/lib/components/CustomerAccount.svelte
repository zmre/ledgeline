<!-- A customer's account (invoicer PLAN 4b, Phase 4 feedback J1, J9, H18):
     what it owes, overdue, its last payment; its invoices (each linking it
     from `customer:`), newest first, with status, total and due date, in two
     lists that never overlap: Open, then Other (paid, drafts, void, …); and
     the other notes that link it, each saying what it is (person,
     organization, contract, …).

     Every link stays in the app: an invoice opens the invoice page, a
     customer its customer page, any other note the in-app note view
     (`noteHref`), never MBR's page in place of the app. A row click opens
     the invoice. -->
<script lang="ts">
    import {balanceText, kindLabel, lastPaymentText, type Backlink, type CustomerDetail} from "#lib/customers.js";
    import type {InvoiceEntry} from "#lib/invoices.js";
    import StatusBadge from "./StatusBadge.svelte";

    let {
        customer,
        invoiceHref,
        customerHref,
        noteHref,
        onOpen,
    }: {
        customer: Pick<CustomerDetail, "account" | "documents" | "backlinks" | "billing" | "title">;
        /** The app's page for one of its invoices (or another document, by number). */
        invoiceHref: (number: string) => string;
        /** The app's page for a customer note (by its path). */
        customerHref: (path: string) => string;
        /** The app's view of any other note (by its path). */
        noteHref: (path: string) => string;
        onOpen?: (doc: InvoiceEntry) => void;
    } = $props();

    const invoicePaths = $derived(new Set(customer.documents.map((d) => d.path)));
    const others = $derived(customer.backlinks.filter((b) => b.path === null || !invoicePaths.has(b.path)));
    const open = $derived(customer.documents.filter((d) => d.open));
    const other = $derived(customer.documents.filter((d) => !d.open));

    /** A backlink's note path: as the index has it, else from MBR's URL (`/notes/kickoff/` → `notes/kickoff`). */
    function notePath(link: Backlink): string {
        if (link.path !== null) return link.path;
        return decodeURIComponent(link.url.replace(/^\/+/, "").replace(/\/+$/, ""));
    }

    function linkHref(link: Backlink): string {
        if (link.number !== null) return invoiceHref(link.number);
        if (link.customer) return customerHref(notePath(link));
        return noteHref(notePath(link));
    }

    function rowClick(event: MouseEvent, doc: InvoiceEntry): void {
        const target = event.target as Element | null;
        if (target?.closest("a, button") ?? false) return;
        if (onOpen !== undefined) onOpen(doc);
        else (event.currentTarget as HTMLElement).querySelector<HTMLAnchorElement>("a[data-row-link]")?.click();
    }
</script>

{#snippet invoiceTable(docs: InvoiceEntry[], testid: string, rowTestid: string)}
    <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table table-sm" data-testid={testid}>
            <thead>
                <tr><th>Number</th><th>Date</th><th>Due</th><th class="text-right">Total</th><th>Status</th></tr>
            </thead>
            <tbody>
                {#each docs as doc (doc.path)}
                    <tr class="cursor-pointer hover:bg-base-200" data-testid={rowTestid} data-number={doc.number} onclick={(e) => rowClick(e, doc)}>
                        <td class="font-medium whitespace-nowrap">
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes `invoiceHref` built with resolve() -->
                            <a class="link link-hover" href={invoiceHref(doc.number ?? doc.path)} data-row-link>{doc.number ?? doc.title}</a>
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
{/snippet}

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

    {#if customer.documents.length === 0}
        <p class="text-sm text-base-content/70" data-testid="customer-no-invoices">No invoices for this customer yet.</p>
    {:else}
        <h2 class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Open</h2>
        {#if open.length > 0}
            {@render invoiceTable(open, "customer-open-invoices", "customer-open-invoice-row")}
        {:else}
            <p class="text-sm text-base-content/70" data-testid="customer-nothing-open">Nothing open.</p>
        {/if}
        {#if other.length > 0}
            <h2 class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Other</h2>
            {@render invoiceTable(other, "customer-other-invoices", "customer-other-invoice-row")}
        {/if}
    {/if}

    {#if others.length > 0}
        <h2 class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Also linked from</h2>
        <ul class="flex flex-col gap-1 text-sm" data-testid="customer-backlinks">
            {#each others as link (link.url)}
                {@const kind = kindLabel(link.kind)}
                <li class="flex flex-wrap items-baseline gap-x-2" data-testid="customer-backlink">
                    <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes hrefs built with resolve() -->
                    <a class="link" href={linkHref(link)}>{link.title ?? notePath(link)}</a>
                    {#if kind !== null}<span class="badge badge-ghost badge-sm" data-testid="backlink-kind">{kind}</span>{/if}
                </li>
            {/each}
        </ul>
    {/if}
</section>
