<!-- A customer's invoices and links (invoicer PLAN 4b, Phase 4 feedback J1,
     J9, H18; contact design §3.4): who invoices are billed to, then its
     invoices (each linking it from `customer:`), newest first, in two lists
     that never overlap: Open, then Other (paid, drafts, void, …; the newest
     five, then "Show all"); and the other notes that link it, each saying
     what it is. What it owes, overdue and the last payment are in the card's
     header (`CustomerCardHeader`).

     Every link stays in the app: an invoice opens the invoice page, a
     customer its customer page, any other note the in-app note view
     (`noteHref`). A row click opens the invoice; `j`/`k` move a row cursor
     over both lists and `o` or Enter opens the row (`rowKeys`). Below `sm`
     the Date column hides, so the table never scrolls sideways. -->
<script lang="ts">
    import {kindLabel, shortDate, type Backlink, type CustomerDetail} from "#lib/customers.js";
    import type {InvoiceEntry} from "#lib/invoices.js";
    import {CURSOR_ROW, rowKeys} from "#lib/keys/rowKeys.svelte.js";
    import StatusBadge from "./StatusBadge.svelte";

    let {
        customer,
        invoiceHref,
        customerHref,
        noteHref,
        onOpen,
    }: {
        customer: Pick<CustomerDetail, "documents" | "backlinks" | "billing" | "title">;
        /** The app's page for one of its invoices (or another document, by number). */
        invoiceHref: (number: string) => string;
        /** The app's page for a customer note (by its path). */
        customerHref: (path: string) => string;
        /** The app's view of any other note (by its path). */
        noteHref: (path: string) => string;
        onOpen?: (doc: InvoiceEntry) => void;
    } = $props();

    /** Other invoices shown before "Show all". */
    const OTHER_SHOWN = 5;

    const invoicePaths = $derived(new Set(customer.documents.map((d) => d.path)));
    const others = $derived(customer.backlinks.filter((b) => b.path === null || !invoicePaths.has(b.path)));
    const open = $derived(customer.documents.filter((d) => d.open));
    const other = $derived(customer.documents.filter((d) => !d.open));
    let allOther = $state(false);
    const otherShown = $derived(allOther ? other : other.slice(0, OTHER_SHOWN));

    let root = $state<HTMLElement | null>(null);
    const cursor = rowKeys<InvoiceEntry>({
        id: "customer-invoices",
        group: "Customer",
        noun: "invoice",
        items: () => [...open, ...otherShown],
        keyOf: (doc) => doc.path,
        root: () => root,
        open: () => onOpen,
    });
    const cursorPath = $derived(cursor.item?.path ?? null);

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
    <table class="table table-sm" data-testid={testid}>
        <thead>
            <tr class="text-xs font-medium text-muted">
                <th class="pl-0">Number</th>
                <th class="hidden sm:table-cell">Date</th>
                <th>Due</th>
                <th class="text-right">Total</th>
                <th class="pr-0">Status</th>
            </tr>
        </thead>
        <tbody>
            {#each docs as doc (doc.path)}
                <tr
                    class="cursor-pointer hover:bg-base-200 {cursorPath === doc.path ? CURSOR_ROW : ''}"
                    data-cursor={cursorPath === doc.path ? "" : undefined}
                    data-testid={rowTestid}
                    data-number={doc.number}
                    onclick={(e) => rowClick(e, doc)}
                >
                    <td class="pl-0 font-medium whitespace-nowrap">
                        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes `invoiceHref` built with resolve() -->
                        <a class="link link-hover" href={invoiceHref(doc.number ?? doc.path)} data-row-link>{doc.number ?? doc.title}</a>
                    </td>
                    <td class="hidden whitespace-nowrap tabular-nums sm:table-cell" title={doc.date ?? undefined}
                        >{doc.date === null ? "—" : shortDate(doc.date)}</td
                    >
                    <td class="whitespace-nowrap tabular-nums" title={doc.due_date ?? undefined}>
                        {doc.due_date === null ? "—" : shortDate(doc.due_date)}
                        {#if doc.overdue}<span class="ml-1 badge badge-xs badge-error">Overdue</span>{/if}
                    </td>
                    <td class="text-right whitespace-nowrap tabular-nums">{doc.total ?? "—"}</td>
                    <td class="pr-0"><StatusBadge {doc} /></td>
                </tr>
            {/each}
        </tbody>
    </table>
{/snippet}

<section class="flex flex-col gap-3" data-testid="customer-account" bind:this={root}>
    {#if customer.billing.name !== customer.title}
        <p class="text-sm text-muted">Invoices to {customer.title} are billed to {customer.billing.name}.</p>
    {/if}

    {#if customer.documents.length === 0}
        <p class="text-sm text-muted" data-testid="customer-no-invoices">No invoices for this customer yet.</p>
    {:else}
        <h2 id="open-invoices" class="text-sm font-semibold text-base-content">Open</h2>
        {#if open.length > 0}
            {@render invoiceTable(open, "customer-open-invoices", "customer-open-invoice-row")}
        {:else}
            <p class="text-sm text-muted" data-testid="customer-nothing-open">Nothing open.</p>
        {/if}
        {#if other.length > 0}
            <h2 class="mt-3 text-sm font-semibold text-base-content">Other</h2>
            {@render invoiceTable(otherShown, "customer-other-invoices", "customer-other-invoice-row")}
            {#if other.length > OTHER_SHOWN}
                <button
                    type="button"
                    class="btn self-start btn-link px-0 btn-sm"
                    aria-expanded={allOther}
                    onclick={() => (allOther = !allOther)}
                    data-testid="other-show-all">{allOther ? "Show fewer" : `Show all ${other.length}`}</button
                >
            {/if}
        {/if}
    {/if}

    {#if others.length > 0}
        <h2 class="mt-3 text-sm font-semibold text-base-content">Also linked from</h2>
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
