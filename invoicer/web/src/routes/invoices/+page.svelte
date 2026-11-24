<!-- The invoice list (invoicer PLAN 3b): read-only, from `GET /api/invoices`,
     again whenever a file changes on disk (live reload). -->
<script lang="ts">
    import {onMount} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {fetchInvoices} from "#lib/api.js";
    import {changes} from "#lib/changes.svelte.js";
    import type {InvoiceEntry} from "#lib/invoices.js";
    import InvoiceList from "#lib/components/InvoiceList.svelte";

    let invoices = $state<InvoiceEntry[] | null>(null);
    let error = $state<Error | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : invoices !== null ? "data" : "loading");

    function load(): void {
        error = null;
        fetchInvoices()
            .then((list) => (invoices = list))
            .catch((e: unknown) => (error = e instanceof Error ? e : new Error(String(e))));
    }

    onMount(load);

    const invoiceHref = (doc: InvoiceEntry): string => resolve("/invoices/[number]", {number: doc.number ?? doc.path});

    // Live reload: the list again after a change on disk, keeping the old one
    // on screen meanwhile.
    let seenSeq = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seenSeq) return;
        seenSeq = seq;
        fetchInvoices()
            .then((list) => {
                invoices = list;
                error = null;
            })
            .catch((e: unknown) => console.warn("invoices:", e));
    });
</script>

<svelte:head>
    <title>Invoices · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-6xl flex-col gap-4">
    <!-- The page's New button is top right, primary, as on every list (H21); the empty state has its own. -->
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">Invoices</h1>
        {#if (invoices?.length ?? 0) > 0}
            <a class="btn btn-primary btn-sm" href={resolve("/new")} data-testid="invoices-new">New invoice</a>
        {/if}
    </div>
    <AsyncSection {view} value={invoices} {error} testid="invoices-error" label="the invoices" loadingLabel="Loading invoices" onRetry={load}>
        {#snippet children(list)}
            <InvoiceList
                invoices={list}
                href={invoiceHref}
                newHref={resolve("/new")}
                onOpen={(doc) => void goto(resolve("/invoices/[number]", {number: doc.number ?? doc.path}))}
            />
        {/snippet}
    </AsyncSection>
</div>
