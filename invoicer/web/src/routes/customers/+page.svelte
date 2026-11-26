<!-- The customer list (invoicer PLAN 4b, 5b): `GET /api/customers` filtered
     and sorted by the server from the page's URL query (S2: `owes`,
     `overdue`, `kind`, `q`, `sort`, `dir`), so a filtered list can be linked
     and survives a reload. Again whenever a file changes on disk. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {fetchCustomers, routeParam, type CustomerRow} from "#lib/customers.js";
    import {
        clearCustomerFilters,
        countText,
        customerFiltered,
        customerQueryString,
        parseCustomerQuery,
        shownCustomerSort,
        sortCustomersBy,
        type CustomerQuery,
        type CustomerSort,
    } from "#lib/listQuery.js";
    import CustomerFilters from "#lib/components/CustomerFilters.svelte";
    import CustomerList from "#lib/components/CustomerList.svelte";

    const query = $derived(parseCustomerQuery(page.url.searchParams));
    const queryString = $derived(customerQueryString(query));
    const filtered = $derived(customerFiltered(query));

    let customers = $state<CustomerRow[] | null>(null);
    let error = $state<Error | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : customers !== null ? "data" : "loading");

    // Only the latest request's answer is shown.
    let latest = 0;
    function load(qs: string): void {
        const mine = ++latest;
        fetchCustomers(undefined, qs)
            .then((list) => {
                if (mine !== latest) return;
                customers = list;
                error = null;
            })
            .catch((e: unknown) => {
                if (mine !== latest) return;
                if (customers === null) error = e instanceof Error ? e : new Error(String(e));
                else console.warn("customers:", e);
            });
    }

    // The list follows the URL.
    $effect(() => {
        const qs = queryString;
        untrack(() => load(qs));
    });

    function retry(): void {
        error = null;
        customers = null;
        load(queryString);
    }

    /** Go to the list for `next`: a history step, or (`replace`) in place of this one. */
    function setQuery(next: CustomerQuery, options: {replace: boolean} = {replace: false}): void {
        const qs = customerQueryString(next);
        if (qs === queryString) return;
        const base = resolve("/customers");
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
        void goto(qs === "" ? base : `${base}?${qs}`, {replace: options.replace, reset: false});
    }

    const customerHref = (row: CustomerRow): string => resolve("/customers/[...path]", {path: routeParam(row.path)});
    const openCustomer = (row: CustomerRow): Promise<void> => goto(resolve("/customers/[...path]", {path: routeParam(row.path)}));

    /** Enter in the search: the customer, when the search leaves only one. */
    async function jump(text: string): Promise<void> {
        if (text.trim() === "") return;
        try {
            const rows = await fetchCustomers(undefined, customerQueryString({...query, q: text.trim()}));
            if (rows.length === 1) await openCustomer(rows[0]);
        } catch (e) {
            console.warn("customers:", e);
        }
    }

    // Live reload: the list again after a change on disk.
    onFilesChanged(() => load(queryString));

    const shown = $derived(shownCustomerSort(query));
    // The New button shows with any customers, or while filters may be hiding them (H21).
    const showNew = $derived((customers?.length ?? 0) > 0 || filtered);
    const countLine = $derived(
        customers === null
            ? ""
            : filtered
              ? `${countText(customers.length, "customer", "customers")} match`
              : countText(customers.length, "customer", "customers")
    );
</script>

<svelte:head>
    <title>Customers · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-6xl flex-col gap-4">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">Customers</h1>
        {#if showNew}
            <a class="btn btn-primary btn-sm" href={resolve("/customers/new")} data-testid="new-customer">New customer</a>
        {/if}
    </div>
    {#if showNew || customers === null}
        <CustomerFilters {query} onChange={setQuery} onJump={(text) => void jump(text)} onClear={() => setQuery(clearCustomerFilters(query))} />
    {/if}
    <AsyncSection {view} value={customers} {error} testid="customers-error" label="the customers" loadingLabel="Loading customers" onRetry={retry}>
        {#snippet children(list)}
            {#if list.length > 0}
                <p class="-mb-2 text-sm text-base-content/70" role="status" data-testid="customer-count">{countLine}</p>
            {/if}
            <CustomerList
                customers={list}
                href={customerHref}
                newHref={resolve("/customers/new")}
                onOpen={(row) => void openCustomer(row)}
                {shown}
                onSort={(column) => setQuery(sortCustomersBy(query, column as CustomerSort))}
                {filtered}
                onClear={() => setQuery(clearCustomerFilters(query))}
            />
        {/snippet}
    </AsyncSection>
</div>
