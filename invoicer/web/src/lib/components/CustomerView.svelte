<!-- A customer, read-only (Phase 4 feedback J1): the contact (kind,
     company, labelled emails, phones and addresses), how it is invoiced
     (terms, the template by name, the revenue account, the name in the
     books, a vendor portal), its contracts by name, open POs and default
     snippets. The page's Edit button swaps in the form.

     Names, never syntax (J6): a company or snippet shows its name, not
     `[[…]]`; a contract its title or file name. A contract note opens in
     the app's note view; a file in its viewer (the window) or a new tab. -->
<script lang="ts">
    import {contractName, linkName, type Contract, type CustomerDetail, type Labelled} from "#lib/customers.js";
    import type {TemplateEntry} from "#lib/invoices.js";

    let {
        customer,
        templates = [],
        companyHref = null,
        noteHref,
        fileHref,
        onOpenFile,
    }: {
        customer: CustomerDetail;
        templates?: TemplateEntry[];
        /** The company's customer page, when a person bills to one that is a customer. */
        companyHref?: string | null;
        /** The app's view of a note (by its path). */
        noteHref: (path: string) => string;
        /** Where a file in the books opens in a browser tab (MBR serves it). */
        fileHref: (path: string) => string;
        /** Open a file in its viewer (the window); absent: a link to `fileHref`. */
        onOpenFile?: (path: string) => void;
    } = $props();

    const form = $derived(customer.form);
    const templateName = $derived.by((): string => {
        const path = form.invoice_template;
        if (path === "") return "The default";
        const known = templates.find((t) => t.path === path);
        return known?.name ?? contractName(path).replace(/\.html?$/i, "");
    });
    const company = $derived(form.company === "" ? null : linkName(form.company));
    const portal = $derived(form.billing_portal);

    function contractKey(c: Contract, at: number): string {
        return `${at}:${c.kind === "note" ? c.link : c.path}`;
    }
</script>

{#snippet labelled(heading: string, items: Labelled[], testid: string, multiline = false)}
    {#if items.length > 0}
        <div class="flex flex-col gap-1" data-testid={testid}>
            <dt class="text-xs font-medium text-base-content/60">{heading}</dt>
            {#each items as item, at (at)}
                <dd class="flex gap-2 text-sm">
                    <span class="w-16 shrink-0 text-base-content/60">{item.label ?? ""}</span>
                    <span class="min-w-0 break-words {multiline ? 'whitespace-pre-line' : ''}">{item.value}</span>
                </dd>
            {/each}
        </div>
    {/if}
{/snippet}

{#snippet field(heading: string, value: string, testid: string)}
    <div class="flex flex-col gap-0.5" data-testid={testid}>
        <dt class="text-xs font-medium text-base-content/60">{heading}</dt>
        <dd class="text-sm break-words">{value}</dd>
    </div>
{/snippet}

<div class="flex flex-col gap-4" data-testid="customer-view">
    <section class="rounded-box border border-base-300 p-4">
        <h2 class="mb-3 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Contact</h2>
        <dl class="grid gap-4 sm:grid-cols-2">
            {#if company !== null}
                <div class="flex flex-col gap-0.5 sm:col-span-2" data-testid="view-company">
                    <dt class="text-xs font-medium text-base-content/60">Company</dt>
                    <dd class="text-sm">
                        {#if companyHref !== null}
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes an href built with resolve() -->
                            <a class="link" href={companyHref}>{company}</a>
                        {:else}
                            {company}
                        {/if}
                        <span class="text-base-content/60">(invoices are billed to it)</span>
                    </dd>
                </div>
            {/if}
            {@render labelled("Emails", form.emails, "view-emails")}
            {@render labelled("Phones", form.phones, "view-phones")}
            {@render labelled("Addresses", form.addresses, "view-addresses", true)}
        </dl>
        {#if form.emails.length === 0 && form.phones.length === 0 && form.addresses.length === 0}
            <p class="text-sm text-base-content/70">No email, phone or address yet.</p>
        {/if}
    </section>

    <section class="rounded-box border border-base-300 p-4">
        <h2 class="mb-3 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Invoicing</h2>
        <dl class="grid gap-4 sm:grid-cols-2">
            {@render field("Terms", form.terms === null ? "The books' default" : `Net ${form.terms} days`, "view-terms")}
            {@render field("Invoice template", templateName, "view-template")}
            {@render field("Revenue account", form.revenue_account === "" ? "The books' default" : form.revenue_account, "view-revenue-account")}
            {@render field("Name in the books", form.ledger_customer === "" ? customer.title : form.ledger_customer, "view-ledger-customer")}
        </dl>
        {#if portal !== null}
            <div class="mt-4 border-t border-base-300 pt-3" data-testid="view-portal">
                <h3 class="mb-2 text-sm font-medium">Vendor portal{portal.name === null ? "" : `: ${portal.name}`}</h3>
                <dl class="grid gap-3 sm:grid-cols-2">
                    {#if portal.url !== null}{@render field("URL", portal.url, "view-portal-url")}{/if}
                    {#if portal.login !== null}{@render field("Login", portal.login, "view-portal-login")}{/if}
                    {#if portal.supplier_id !== null}{@render field("Supplier ID", portal.supplier_id, "view-portal-supplier")}{/if}
                    {#if portal.requires_reference}{@render field("Reference", "Required: invoices without a PO are refused", "view-portal-reference")}{/if}
                </dl>
                {#if portal.instructions !== null}
                    <p class="mt-2 text-sm whitespace-pre-line">{portal.instructions}</p>
                {/if}
            </div>
        {/if}
    </section>

    <section class="rounded-box border border-base-300 p-4" data-testid="view-contracts">
        <h2 class="mb-2 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Contracts</h2>
        {#if customer.contracts.length === 0}
            <p class="text-sm text-base-content/70">None yet. Edit the customer to add one.</p>
        {:else}
            <ul class="flex flex-col gap-1 text-sm">
                {#each customer.contracts as contract, at (contractKey(contract, at))}
                    <li class="flex flex-wrap items-baseline gap-2" data-testid="view-contract">
                        {#if contract.found === null}
                            <span>{contract.name}</span>
                            <span class="badge badge-sm badge-warning" title="No such note or file in the books folder">missing</span>
                        {:else if contract.kind === "note"}
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes an href built with resolve() -->
                            <a class="link" href={noteHref(contract.found)}>{contract.name}</a>
                        {:else if onOpenFile !== undefined}
                            {@const found = contract.found}
                            <button type="button" class="link text-left" onclick={() => onOpenFile(found)}>{contract.name}</button>
                        {:else}
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a file MBR serves, in its own tab -->
                            <a class="link" href={fileHref(contract.found)} target="_blank" rel="noopener">{contract.name}</a>
                        {/if}
                    </li>
                {/each}
            </ul>
        {/if}
    </section>

    {#if customer.snippets.length > 0}
        <section class="rounded-box border border-base-300 p-4 text-sm" data-testid="view-snippets">
            <h2 class="mb-2 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Default snippets</h2>
            <p>{customer.snippets.map(linkName).join(", ")}</p>
            <p class="mt-1 text-xs text-base-content/60">Each new invoice for this customer starts with them.</p>
        </section>
    {/if}
</div>
