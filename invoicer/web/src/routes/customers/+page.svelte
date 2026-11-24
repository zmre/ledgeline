<!-- The customer list (invoicer PLAN 4b): `GET /api/customers`, again
     whenever a file changes on disk (live reload). -->
<script lang="ts">
    import {onMount} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {changes} from "#lib/changes.svelte.js";
    import {fetchCustomers, routeParam, type CustomerRow} from "#lib/customers.js";
    import CustomerList from "#lib/components/CustomerList.svelte";

    let customers = $state<CustomerRow[] | null>(null);
    let error = $state<Error | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : customers !== null ? "data" : "loading");

    function load(): void {
        error = null;
        fetchCustomers()
            .then((list) => (customers = list))
            .catch((e: unknown) => (error = e instanceof Error ? e : new Error(String(e))));
    }

    onMount(load);

    const customerHref = (row: CustomerRow): string => resolve("/customers/[...path]", {path: routeParam(row.path)});

    let seenSeq = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seenSeq) return;
        seenSeq = seq;
        fetchCustomers()
            .then((list) => {
                customers = list;
                error = null;
            })
            .catch((e: unknown) => console.warn("customers:", e));
    });
</script>

<svelte:head>
    <title>Customers · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-6xl flex-col gap-4">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">Customers</h1>
        {#if (customers?.length ?? 0) > 0}
            <a class="btn btn-primary btn-sm" href={resolve("/customers/new")} data-testid="new-customer">New customer</a>
        {/if}
    </div>
    <AsyncSection {view} value={customers} {error} testid="customers-error" label="the customers" loadingLabel="Loading customers" onRetry={load}>
        {#snippet children(list)}
            <CustomerList
                customers={list}
                href={customerHref}
                newHref={resolve("/customers/new")}
                onOpen={(row) => void goto(resolve("/customers/[...path]", {path: routeParam(row.path)}))}
            />
        {/snippet}
    </AsyncSection>
</div>
