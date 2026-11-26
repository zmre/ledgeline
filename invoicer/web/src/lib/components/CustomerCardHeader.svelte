<!-- The customer card's header (contact design §3.3): who it is (a monogram,
     a circle for a person and a rounded square for an organization; the
     name; "Organization · Billed via Coupa · Net 30"), what can be done now
     (Email, Call, Open the portal, New invoice: each a real link or button
     with its key in its tooltip), and where the money stands (owed, overdue,
     the last payment). The only filled surface on the page.

     While editing it keeps the identity and the page's controls (`controls`)
     and drops the actions and the money line. On a narrow window the name
     goes under the monogram row and the actions make a two-column grid. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import {
        balanceText,
        defaultOf,
        humanizeLabel,
        initials,
        linkName,
        mailtoHref,
        shortDate,
        telHref,
        type CustomerDetail,
        type Labelled,
    } from "#lib/customers.js";
    import Icon from "./Icon.svelte";

    let {
        customer,
        editing = false,
        controls,
        companyHref = null,
        newInvoiceHref,
        onOpenPortal,
        external = (href: string) => href,
    }: {
        customer: CustomerDetail;
        editing?: boolean;
        /** The page's Edit, or Cancel, Discard and Save. */
        controls?: Snippet;
        /** The company's customer page, when a person bills to one that is a customer. */
        companyHref?: string | null;
        /** New invoice for this customer. */
        newInvoiceHref: string;
        /** Open the portal (present when it has a plain https URL). */
        onOpenPortal?: () => void;
        /** A mailto or tel link as this window follows it (the desktop window's hand-off); as is in a browser. */
        external?: (href: string) => string;
    } = $props();

    const form = $derived(customer.form);
    const company = $derived(customer.kind === "person" && form.company !== "" ? linkName(form.company) : null);
    const portal = $derived(form.billing_portal);

    type Target = {item: Labelled; href: string};
    const linked = (items: Labelled[], href: (v: string) => string | null): Target[] =>
        items.flatMap((item) => {
            const h = href(item.value);
            return h === null ? [] : [{item, href: h}];
        });
    const emails = $derived(linked(form.emails, mailtoHref));
    const phones = $derived(linked(form.phones, telHref));
    /** The default of each: billing, then work, then the first. */
    const pick = (targets: Target[]): Target | null => {
        const chosen = defaultOf(targets.map((t) => t.item));
        return targets.find((t) => t.item === chosen) ?? null;
    };
    const email = $derived(pick(emails));
    const phone = $derived(pick(phones));
    /** The first action available takes the primary colour. */
    const primary = $derived(email !== null ? "email" : phone !== null ? "call" : onOpenPortal !== undefined ? "portal" : "new");
    const style = (action: string): string => (action === primary ? "btn-primary" : "btn-ghost bg-base-100");

    const subtitle = $derived(
        [
            customer.kind === "person" ? "Person" : "Organization",
            portal !== null ? `Billed via ${portal.name ?? "a vendor portal"}` : null,
            form.terms !== null ? `Net ${form.terms}` : null,
        ].filter((p) => p !== null)
    );

    const account = $derived(customer.account);
    const lastPayment = $derived(account.last_payment === null ? "None yet" : `${account.last_payment.amount} · ${shortDate(account.last_payment.date)}`);

    /** Between the subtitle's parts. */
    const SEP = " · ";

    const menuLabel = (item: Labelled, value: string): string => (item.label === null ? value : `${humanizeLabel(item.label)} · ${value}`);

    /** Escape closes an open menu and gives its toggle focus back. */
    function menuKey(event: KeyboardEvent): void {
        const menu = (event.currentTarget as HTMLElement).closest("details");
        if (event.key !== "Escape" || menu === null || !menu.open) return;
        event.preventDefault();
        event.stopPropagation();
        menu.open = false;
        menu.querySelector("summary")?.focus();
    }
</script>

{#snippet split(action: "email" | "call", targets: Target[], chosen: Target, label: string, icon: "mail" | "phone", keys: string | null)}
    <div class="join min-w-0">
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a mailto or tel link, not an app route -->
        <a
            class="btn join-item min-w-0 flex-1 btn-sm {style(action)}"
            href={external(chosen.href)}
            rel="external"
            title={`${label} ${chosen.item.value}${keys === null ? "" : ` (${keys})`}`}
            aria-keyshortcuts={keys ?? undefined}
            data-testid="action-{action}"><Icon name={icon} />{label}</a
        >
        {#if targets.length > 1}
            <details class="dropdown dropdown-end join-item border-0">
                <summary
                    class="btn join-item btn-sm {style(action)} px-2"
                    aria-label="Choose {action === 'email' ? 'an email' : 'a number'}"
                    onkeydown={menuKey}
                    data-testid="action-{action}-menu"
                >
                    <Icon name="chevron-down" />
                </summary>
                <ul class="menu dropdown-content z-20 w-max max-w-[80vw] rounded-box bg-base-100 p-1 text-sm shadow-lg">
                    {#each targets as target, at (at)}
                        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a mailto or tel link, not an app route -->
                        <li><a href={external(target.href)} rel="external" onkeydown={menuKey}>{menuLabel(target.item, target.item.value)}</a></li>
                    {/each}
                </ul>
            </details>
        {/if}
    </div>
{/snippet}

<header class="rounded-box bg-base-200 p-4 sm:p-6" data-testid="customer-card-header">
    <div class="grid grid-cols-[auto_minmax(0,1fr)] items-start gap-x-4 gap-y-3 sm:grid-cols-[auto_minmax(0,1fr)_auto]">
        <div
            class="grid size-12 place-items-center bg-neutral text-xl font-semibold text-neutral-content sm:size-16 {customer.kind === 'person'
                ? 'rounded-full'
                : 'rounded-box'}"
            aria-hidden="true"
            data-testid="customer-monogram"
        >
            {initials(customer.title, customer.kind)}
        </div>
        <div class="flex flex-wrap items-center justify-end gap-2 justify-self-end sm:col-start-3 sm:row-start-1">
            {@render controls?.()}
        </div>
        <div class="col-span-2 flex min-w-0 flex-col gap-1 sm:col-span-1 sm:col-start-2 sm:row-start-1 sm:self-center">
            <h1 class="text-2xl font-semibold tracking-tight break-words text-base-content sm:text-3xl" data-testid="customer-heading">{customer.title}</h1>
            <p class="text-sm text-muted" data-testid="customer-subtitle">
                {subtitle[0]}{#if company !== null}{SEP}<span data-testid="view-company">
                        {#if companyHref !== null}
                            <!-- eslint-disable svelte/no-navigation-without-resolve -- the page passes an href built with resolve() -->
                            <a class="link" href={companyHref}>{company}</a>
                            <!-- eslint-enable svelte/no-navigation-without-resolve -->
                        {:else}{company}{/if}
                    </span>{/if}{#each subtitle.slice(1) as part (part)}{SEP}{part}{/each}
            </p>
        </div>
    </div>

    {#if !editing}
        <div class="mt-4 grid grid-cols-2 gap-2 sm:flex sm:flex-wrap" role="group" aria-label="Actions" data-testid="view-actions">
            {#if email !== null}{@render split("email", emails, email, "Email", "mail", "m")}{/if}
            {#if phone !== null}{@render split("call", phones, phone, "Call", "phone", null)}{/if}
            {#if onOpenPortal !== undefined}
                <button
                    type="button"
                    class="btn min-w-0 btn-sm {style('portal')}"
                    onclick={onOpenPortal}
                    title="Open the {portal?.name ?? 'vendor'} portal in the browser (p)"
                    aria-keyshortcuts="p"
                    data-testid="action-portal"><Icon name="external" /><span class="truncate">Open {portal?.name ?? "portal"}</span></button
                >
            {/if}
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the page passes an href built with resolve() -->
            <a
                class="btn min-w-0 btn-sm {style('new')}"
                href={newInvoiceHref}
                title="New invoice for {customer.title} (n)"
                aria-keyshortcuts="n"
                data-testid="action-new-invoice"><Icon name="plus" />New invoice</a
            >
            <!-- eslint-enable svelte/no-navigation-without-resolve -->
        </div>

        {#if customer.documents.length > 0}
            <dl
                class="mt-5 flex flex-col gap-1 border-t border-base-300 pt-4 sm:flex-row sm:flex-wrap sm:items-baseline sm:gap-x-6"
                data-testid="account-summary"
            >
                <div class="flex items-baseline gap-2">
                    <dt class="text-xs font-medium text-muted">Owed</dt>
                    <dd class="text-lg font-semibold tabular-nums" data-testid="account-balance">{balanceText(account)}</dd>
                </div>
                {#if account.overdue > 0}
                    <div class="flex items-baseline">
                        <dt class="sr-only">Overdue</dt>
                        <dd>
                            <a class="badge badge-error" href="#open-invoices" data-testid="account-overdue"
                                >{account.overdue} overdue{account.oldest_overdue === null ? "" : ` · since ${shortDate(account.oldest_overdue)}`}</a
                            >
                        </dd>
                    </div>
                {/if}
                <div class="flex items-baseline gap-2">
                    <dt class="text-xs font-medium text-muted">Last payment</dt>
                    <dd class="text-lg font-semibold tabular-nums" data-testid="account-last-payment">{lastPayment}</dd>
                </div>
            </dl>
        {/if}
    {/if}
</header>
