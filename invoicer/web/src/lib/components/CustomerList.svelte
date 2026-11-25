<!-- The customer list (invoicer PLAN 4b): `GET /api/customers` by title, with
     what each owes, whether any of it is overdue, and its last payment.

     A click anywhere on a row opens the customer, except on a link or button
     in it, as the invoice list does (Phase 3 feedback F4): the name is the
     row's one link, for the keyboard and screen readers.

     PLAN 5b (S2): with `onSort`, Name, Owed, Overdue and Last payment sort
     (SortHeader); rows with something overdue are tinted; with `filtered`, an
     empty list says nothing matches and offers Clear filters. -->
<script lang="ts">
    import {balanceText, lastPaymentText, type CustomerRow} from "#lib/customers.js";
    import type {SortDir} from "#lib/listQuery.js";
    import EmptyState from "./EmptyState.svelte";
    import SortHeader from "./SortHeader.svelte";

    let {
        customers,
        href,
        newHref,
        onOpen,
        shown = {sort: "name", dir: "asc"},
        onSort,
        filtered = false,
        onClear,
    }: {
        customers: CustomerRow[];
        href: (row: CustomerRow) => string;
        /** Where "New customer" goes. */
        newHref: string;
        onOpen?: (row: CustomerRow) => void;
        /** The column the rows are sorted by (by name unless the URL says). */
        shown?: {sort: string; dir: SortDir};
        /** Sort by a column; without it the headers are plain text. */
        onSort?: (column: string) => void;
        filtered?: boolean;
        onClear?: () => void;
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

{#snippet header(column: string, label: string, align?: "right")}
    {#if onSort !== undefined}
        <SortHeader {column} {label} {shown} {onSort} {align} />
    {:else}
        <th class={align === "right" ? "text-right" : ""}>{label}</th>
    {/if}
{/snippet}

{#if customers.length === 0 && filtered}
    <div class="rounded-box border border-base-300 px-4 py-8 text-center" data-testid="customer-list-no-match">
        <p class="text-base-content/70">No customers match these filters.</p>
        {#if onClear !== undefined}
            <button type="button" class="btn mt-3 btn-outline btn-sm" onclick={onClear} data-testid="customer-list-clear">Clear filters</button>
        {/if}
    </div>
{:else if customers.length === 0}
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
                    {@render header("name", "Name")}
                    <th>Billing email</th>
                    <th class="text-right">Invoices</th>
                    {@render header("owed", "Owed", "right")}
                    {@render header("overdue", "Overdue")}
                    {@render header("last-payment", "Last payment")}
                </tr>
            </thead>
            <tbody>
                {#each customers as row (row.path)}
                    <tr
                        class="cursor-pointer {row.account.overdue > 0 ? 'bg-error/8 hover:bg-error/15' : 'hover:bg-base-200'}"
                        data-testid="customer-row"
                        data-path={row.path}
                        onclick={(event) => rowClick(event, row)}
                    >
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
                        </td>
                        <td class="whitespace-nowrap">
                            {#if row.account.overdue > 0}
                                <span
                                    class="badge badge-sm badge-error"
                                    data-testid="overdue-badge"
                                    title={row.account.oldest_overdue === null ? undefined : `The oldest was due ${row.account.oldest_overdue}`}
                                    >{row.account.overdue} overdue</span
                                >
                            {:else}
                                <span class="text-base-content/60">—</span>
                            {/if}
                        </td>
                        <td class="whitespace-nowrap tabular-nums">{lastPaymentText(row.account)}</td>
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>
{/if}
