<!-- A customer, read-only: the body of the contact card (contact design
     §3.4), under `CustomerCardHeader`. Groups, not boxes: Contact (emails,
     phones and addresses side by side when there is room, each value its
     own action, mailto or tel, with a copy button), Invoicing, the vendor
     portal, the open POs (`pos`, the page's), contracts, default snippets
     and the notes. Optional groups hide when empty, and one line at the end
     says what Edit can add.

     Names, never syntax (J6): a snippet or contract shows its name, never
     `[[…]]` or a path (`AttachmentLink`). Notes: the note's markdown body,
     rendered read-only by MBR's markdown field (`ProseField`), clamped when
     long; the "Closed POs" section the app writes is folded away. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import InfoTip from "#ui/components/InfoTip.svelte";
    import {copyStatus} from "#lib/copy.svelte.js";
    import {
        contractName,
        defaultOf,
        entryOf,
        humanizeLabel,
        linkName,
        mailtoHref,
        notesParts,
        portalHost,
        telHref,
        type Contract,
        type CustomerDetail,
        type Labelled,
    } from "#lib/customers.js";
    import {pageUrl, type TemplateEntry} from "#lib/invoices.js";
    import AttachmentLink from "./AttachmentLink.svelte";
    import CopyButton from "./CopyButton.svelte";
    import Icon from "./Icon.svelte";
    import ProseField from "./ProseField.svelte";

    let {
        customer,
        templates = [],
        noteHref,
        fileHref,
        onOpenFile,
        onOpenPortal,
        external = (href: string) => href,
        pos,
    }: {
        customer: CustomerDetail;
        templates?: TemplateEntry[];
        /** The app's view of a note (by its path). */
        noteHref: (path: string) => string;
        /** Where a file in the books opens in a browser tab (MBR serves it). */
        fileHref: (path: string) => string;
        /** Open a file in its viewer (the window): a books path or an outside file's entry; absent: a link to `fileHref`. */
        onOpenFile?: (path: string) => void;
        /** Open the portal (present when it has a plain https URL). */
        onOpenPortal?: () => void;
        /** A mailto or tel link as this window follows it (the desktop window's hand-off); as is in a browser. */
        external?: (href: string) => string;
        /** The open POs group (the page's `OpenPos`), between the portal and the contracts. */
        pos?: Snippet;
    } = $props();

    const uid = $props.id();
    const form = $derived(customer.form);
    const templateName = $derived.by((): string | null => {
        const path = form.invoice_template;
        if (path === "") return null;
        const known = templates.find((t) => t.path === path);
        return known?.name ?? contractName(path).replace(/\.html?$/i, "");
    });
    const portal = $derived(form.billing_portal);
    const ledgerName = $derived(form.ledger_customer !== "" && form.ledger_customer !== customer.title ? form.ledger_customer : null);
    const hasContact = $derived(form.emails.length + form.phones.length + form.addresses.length > 0);

    const parts = $derived(notesParts(form.body ?? ""));
    /** The note's folder, so links in its notes resolve as on its page. */
    const linkBase = $derived(pageUrl(customer.path).replace(/[^/]+\/$/, ""));

    /** Notes taller than about twelve lines are clamped until "Show all notes". */
    let notesBox = $state<HTMLDivElement | null>(null);
    let overflowing = $state(false);
    let expanded = $state(false);
    $effect(() => {
        const box = notesBox;
        if (box === null || typeof ResizeObserver === "undefined") return;
        const measure = (): void => {
            overflowing = box.scrollHeight > box.clientHeight + 1;
        };
        const observer = new ResizeObserver(measure);
        observer.observe(box);
        for (const child of Array.from(box.children)) observer.observe(child);
        measure();
        return () => observer.disconnect();
    });

    /** The optional groups that are empty, for the closing line. */
    const missing = $derived(
        [
            customer.contracts.length === 0 ? "contracts" : null,
            portal === null ? "a vendor portal" : null,
            customer.snippets.length === 0 ? "default snippets" : null,
            parts.notes === "" ? "notes" : null,
        ].filter((m) => m !== null)
    );
    const missingText = $derived(missing.length <= 1 ? (missing[0] ?? "") : `${missing.slice(0, -1).join(", ")} or ${missing[missing.length - 1]}`);

    /** The chord that copies a group's default value, for its copy button's tooltip. */
    const CHORDS = {email: "y e", phone: "y p", address: "y a"} as const;

    function contractKey(c: Contract, at: number): string {
        return `${at}:${entryOf(c)}`;
    }

    /** What kind of thing a contract is, in a word: "Note", "PDF". */
    function typeOf(c: Contract): string | null {
        if (c.kind === "note") return "Note";
        const ext = /\.([a-z0-9]{1,5})$/i.exec(c.name);
        return ext === null ? null : ext[1].toUpperCase();
    }
</script>

{#snippet contactGroup(heading: string, noun: "email" | "phone" | "address", items: Labelled[], testid: string)}
    {#if items.length > 0}
        {@const chosen = defaultOf(items)}
        <div class="min-w-0" data-testid={testid}>
            <h3 class="mb-1 text-xs font-medium text-muted">{heading}</h3>
            <dl class="flex flex-col gap-3">
                {#each items as item, at (at)}
                    {@const label = item.label === null ? null : humanizeLabel(item.label)}
                    {@const what = label === null ? noun : `${label.toLowerCase()} ${noun}`}
                    {@const href = noun === "email" ? mailtoHref(item.value) : noun === "phone" ? telHref(item.value) : null}
                    <div class="group flex min-w-0 flex-col gap-0.5">
                        {#if label !== null}<dt class="text-xs text-muted">{label}</dt>{:else}<dt class="sr-only">{heading}</dt>{/if}
                        <dd class="flex min-w-0 items-start gap-1 text-sm text-base-content">
                            {#if noun === "address"}
                                <address class="min-w-0 break-words whitespace-pre-line not-italic">{item.value}</address>
                            {:else if href !== null}
                                <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a mailto or tel link, not an app route -->
                                <a class="min-w-0 link break-all link-hover" href={external(href)} rel="external">{item.value}</a>
                            {:else}
                                <span class="min-w-0 break-words">{item.value}</span>
                            {/if}
                            <CopyButton value={item.value} {what} keys={item === chosen ? CHORDS[noun] : undefined} />
                        </dd>
                    </div>
                {/each}
            </dl>
        </div>
    {/if}
{/snippet}

{#snippet field(heading: string, testid: string, value: string | null, fallback: string, mono = false)}
    <div class="flex min-w-0 flex-col gap-0.5" data-testid={testid}>
        <dt class="text-xs font-medium text-muted">{heading}</dt>
        {#if value === null}
            <dd class="text-sm text-muted">{fallback}</dd>
        {:else}
            <dd class="text-sm break-words text-base-content {mono ? 'font-mono text-[0.8125rem]' : ''}">{value}</dd>
        {/if}
    </div>
{/snippet}

<div class="flex flex-col gap-8" data-testid="customer-view">
    <div class="sr-only" role="status" data-testid="copy-status">{copyStatus.message}</div>

    <section aria-labelledby="{uid}-contact">
        <h2 id="{uid}-contact" class="mb-3 text-sm font-semibold text-base-content">Contact</h2>
        {#if hasContact}
            <div class="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-x-8 gap-y-5">
                {@render contactGroup("Email", "email", form.emails, "view-emails")}
                {@render contactGroup("Phone", "phone", form.phones, "view-phones")}
                {@render contactGroup("Address", "address", form.addresses, "view-addresses")}
            </div>
        {:else}
            <p class="text-sm text-muted" title="Press e to edit" data-testid="view-no-contact">
                No contact details yet. Edit to add an email, phone or address.
            </p>
        {/if}
    </section>

    <section aria-labelledby="{uid}-invoicing">
        <h2 id="{uid}-invoicing" class="mb-3 text-sm font-semibold text-base-content">Invoicing</h2>
        <dl class="grid gap-x-8 gap-y-4 sm:grid-cols-2">
            {@render field("Terms", "view-terms", form.terms === null ? null : `Net ${form.terms}`, "Books default")}
            {@render field("Invoice template", "view-template", templateName, "Default")}
            {@render field("Revenue account", "view-revenue-account", form.revenue_account === "" ? null : form.revenue_account, "Books default", true)}
            {#if ledgerName !== null}{@render field("Name in the books", "view-ledger-customer", ledgerName, "")}{/if}
        </dl>
    </section>

    {#if portal !== null}
        <section aria-labelledby="{uid}-portal" data-testid="view-portal">
            <h2 id="{uid}-portal" class="mb-3 text-sm font-semibold text-base-content">{portal.name === null ? "Vendor portal" : `${portal.name} portal`}</h2>
            <dl class="grid gap-x-8 gap-y-4 sm:grid-cols-2">
                {#if portal.url !== null}
                    <div class="flex min-w-0 flex-col gap-0.5" data-testid="view-portal-url">
                        <dt class="text-xs font-medium text-muted">URL</dt>
                        <dd class="text-sm">
                            {#if onOpenPortal !== undefined}
                                <button type="button" class="link text-left break-all link-hover" title={portal.url} onclick={onOpenPortal}
                                    >{portalHost(portal.url)} ↗</button
                                >
                            {:else}
                                <span class="break-all" title={portal.url}>{portalHost(portal.url)}</span>
                            {/if}
                        </dd>
                    </div>
                {/if}
                {#if portal.login !== null}
                    <div class="group flex min-w-0 flex-col gap-0.5" data-testid="view-portal-login">
                        <dt class="text-xs font-medium text-muted">Login</dt>
                        <dd class="flex min-w-0 items-start gap-1 text-sm">
                            <span class="min-w-0 break-all">{portal.login}</span><CopyButton value={portal.login} what="portal login" />
                        </dd>
                    </div>
                {/if}
                {#if portal.supplier_id !== null}
                    <div class="group flex min-w-0 flex-col gap-0.5" data-testid="view-portal-supplier">
                        <dt class="text-xs font-medium text-muted">Supplier ID</dt>
                        <dd class="flex min-w-0 items-start gap-1 font-mono text-[0.8125rem]">
                            <span class="min-w-0 break-all">{portal.supplier_id}</span><CopyButton value={portal.supplier_id} what="supplier ID" keys="y s" />
                        </dd>
                    </div>
                {/if}
                {#if portal.requires_reference}
                    <div class="flex min-w-0 flex-col gap-0.5" data-testid="view-portal-reference">
                        <dt class="text-xs font-medium text-muted">Reference</dt>
                        <dd class="flex flex-wrap items-baseline gap-2 text-sm">
                            <span class="badge badge-soft badge-sm badge-warning">PO required</span>
                            <span class="text-muted">Invoices without a PO reference are refused.</span>
                        </dd>
                    </div>
                {/if}
            </dl>
            {#if portal.instructions !== null}
                <div class="customer-prose mt-4 rounded-box bg-base-200 p-3 text-sm" data-testid="view-portal-instructions">
                    <ProseField value={portal.instructions} label="Portal instructions" readonly {linkBase} onChange={() => {}} />
                </div>
            {/if}
        </section>
    {/if}

    {@render pos?.()}

    {#if customer.contracts.length > 0}
        <section aria-labelledby="{uid}-contracts" data-testid="view-contracts">
            <h2 id="{uid}-contracts" class="mb-2 text-sm font-semibold text-base-content">Contracts</h2>
            <ul class="divide-y divide-base-300 text-sm">
                {#each customer.contracts as contract, at (contractKey(contract, at))}
                    {@const type = typeOf(contract)}
                    <li class="flex min-w-0 items-center gap-3 py-2" data-testid="view-contract">
                        <Icon name={contract.kind === "note" ? "note" : "file"} class="size-4 text-muted" />
                        <span class="min-w-0 flex-1">
                            <AttachmentLink attachment={contract} {noteHref} {fileHref} {onOpenFile} testid="contract" outsideTestid="view-contract-outside" />
                        </span>
                        {#if type !== null}<span class="shrink-0 text-xs text-muted">{type}</span>{/if}
                    </li>
                {/each}
            </ul>
        </section>
    {/if}

    {#if customer.snippets.length > 0}
        <section aria-labelledby="{uid}-snippets" data-testid="view-snippets">
            <h2 id="{uid}-snippets" class="mb-2 flex items-center gap-1 text-sm font-semibold text-base-content">
                Default snippets
                <InfoTip label="default snippets" text="Each new invoice for this customer starts with them." />
            </h2>
            <ul class="flex flex-wrap gap-1.5">
                {#each customer.snippets as snippet, at (at)}<li class="badge badge-outline">{linkName(snippet)}</li>{/each}
            </ul>
        </section>
    {/if}

    {#if parts.notes !== "" || parts.closed !== ""}
        <section aria-labelledby="{uid}-notes" data-testid="view-notes">
            <h2 id="{uid}-notes" class="mb-2 text-sm font-semibold text-base-content">Notes</h2>
            {#if parts.notes !== ""}
                <div class="relative max-w-[70ch] text-sm leading-relaxed">
                    <div bind:this={notesBox} class="customer-prose {expanded ? '' : 'max-h-[18rem] overflow-hidden'}" data-testid="view-notes-box">
                        <ProseField value={parts.notes} label="Notes" readonly {linkBase} onChange={() => {}} testid="view-notes-text" />
                    </div>
                    {#if overflowing && !expanded}
                        <div class="pointer-events-none absolute inset-x-0 bottom-0 h-12 bg-gradient-to-t from-base-100"></div>
                    {/if}
                </div>
                {#if overflowing || expanded}
                    <button
                        type="button"
                        class="btn btn-link px-0 btn-sm"
                        aria-expanded={expanded}
                        onclick={() => (expanded = !expanded)}
                        data-testid="notes-show-all">{expanded ? "Show less" : "Show all notes"}</button
                    >
                {/if}
            {/if}
            {#if parts.closed !== ""}
                <details class="mt-3 max-w-[70ch] text-sm" data-testid="view-closed-pos">
                    <summary class="cursor-pointer text-muted">Closed POs ({parts.closedCount})</summary>
                    <div class="customer-prose mt-2">
                        <ProseField value={parts.closed} label="Closed POs" readonly {linkBase} onChange={() => {}} />
                    </div>
                </details>
            {/if}
        </section>
    {/if}

    {#if missing.length > 0}
        <p class="text-xs text-muted" data-testid="view-missing-extras">Add {missingText} with Edit.</p>
    {/if}
</div>
