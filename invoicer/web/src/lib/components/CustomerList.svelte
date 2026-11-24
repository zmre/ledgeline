<!-- The customer list (invoicer PLAN 4b): `GET /api/customers` by title, with
     what each owes, whether any of it is overdue, and its last payment.

     A click anywhere on a row opens the customer, except on a link or button
     in it, as the invoice list does (Phase 3 feedback F4): the name is the
     row's one link, for the keyboard and screen readers. -->
<script lang="ts">
    import {balanceText, lastPaymentText, type CustomerRow} from "#lib/customers.js";
    import EmptyState from "./EmptyState.svelte";

    let {
        customers,
        href,
        newHref,
        onOpen,
    }: {
        customers: CustomerRow[];
        href: (row: CustomerRow) => string;
        /** Where "New customer" goes. */
        newHref: string;
        onOpen?: (row: CustomerRow) => void;
    } = $props();

    function rowClick(event: MouseEvent, row: CustomerRow): void {
        const target = event.target as Element | null;
        if (target?.closest("a, button, input, select, textarea, label, summary") ?? false) return;
        if ((window.getSelection?.()?.toString() ?? "") !== "") return;
        if (onOpen !== undefined) {
            onOpen(row);
            return;
        }
        (event.currentTarget as HTMLElement).querySelector<HTMLAnchorElement>("a[data-row-link]")?.click();
    }
</script>

{#if customers.length === 0}
    <EmptyState
        title="No customers yet"
        text="A customer is a person or organization you bill. Add one here, or tag a contact note you already have as a customer."
        action="New customer"
        href={newHref}
        testid="customer-list-empty"
        actionTestid="customer-list-new"
    />
{:else}
    <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table table-sm" data-testid="customer-list">
            <thead>
                <tr>
                    <th>Name</th>
                    <th>Billing email</th>
                    <th class="text-right">Invoices</th>
                    <th class="text-right">Owed</th>
                    <th>Last payment</th>
                </tr>
            </thead>
            <tbody>
                {#each customers as row (row.path)}
                    <tr class="cursor-pointer hover:bg-base-200" data-testid="customer-row" data-path={row.path} onclick={(event) => rowClick(event, row)}>
                        <td class="font-medium">
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes `href` built with resolve() -->
                            <a class="link link-hover" href={href(row)} data-row-link>{row.title}</a>
                            <span class="ml-1 text-xs text-base-content/60">{row.kind === "person" ? "Person" : "Organization"}</span>
                            {#if row.billing.name !== row.title}<span class="ml-1 text-xs text-base-content/60">· bills {row.billing.name}</span>{/if}
                        </td>
                        <td class="text-sm">{row.billing.email ?? "—"}</td>
                        <td class="text-right tabular-nums">{row.invoices}</td>
                        <td class="text-right whitespace-nowrap tabular-nums">
                            {#if row.account.balance.length === 0}
                                <span class="text-base-content/60">—</span>
                            {:else}
                                {balanceText(row.account)}
                            {/if}
                            {#if row.account.overdue > 0}
                                <span class="ml-1 badge badge-sm badge-error" data-testid="overdue-badge" title="Due {row.account.oldest_overdue ?? ''}"
                                    >Overdue</span
                                >
                            {/if}
                        </td>
                        <td class="whitespace-nowrap tabular-nums">{lastPaymentText(row.account)}</td>
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>
{/if}
