<!-- The invoice list (invoicer PLAN 3b, 5b): `GET /api/invoices` filtered and
     sorted by the server, from the page's URL query (S1: `status`,
     `customer`, `from`, `to`, `aged`, `paid_from`, `paid_to`, `q`, `sort`,
     `dir`). The URL is the
     state: a link opens the same list, a reload keeps it, Back and Forward
     step through it. Again whenever a file changes on disk (live reload). -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {fetchInvoices} from "#lib/api.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {fetchCustomers} from "#lib/customers.js";
    import type {InvoiceEntry} from "#lib/invoices.js";
    import {
        clearInvoiceFilters,
        countText,
        invoiceFiltered,
        invoiceQueryString,
        parseInvoiceQuery,
        quickJump,
        shownInvoiceSort,
        sortInvoicesBy,
        type InvoiceQuery,
        type InvoiceSort,
    } from "#lib/listQuery.js";
    import InvoiceFilters from "#lib/components/InvoiceFilters.svelte";
    import InvoiceList from "#lib/components/InvoiceList.svelte";

    const query = $derived(parseInvoiceQuery(page.url.searchParams));
    const queryString = $derived(invoiceQueryString(query));
    const filtered = $derived(invoiceFiltered(query));

    let invoices = $state<InvoiceEntry[] | null>(null);
    let error = $state<Error | null>(null);
    let customerNames = $state<string[]>([]);
    let jumpNote = $state<string | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : invoices !== null ? "data" : "loading");

    // Only the latest request's answer is shown: a slow one for an older
    // query never overwrites a newer list.
    let latest = 0;
    function load(qs: string): void {
        const mine = ++latest;
        fetchInvoices(undefined, qs)
            .then((list) => {
                if (mine !== latest) return;
                invoices = list;
                error = null;
            })
            .catch((e: unknown) => {
                if (mine !== latest) return;
                // Keep a list already on screen (live reload); fail only the first load.
                if (invoices === null) error = e instanceof Error ? e : new Error(String(e));
                else console.warn("invoices:", e);
            });
    }

    function loadCustomers(): void {
        fetchCustomers()
            .then((rows) => (customerNames = rows.map((r) => r.title)))
            .catch((e: unknown) => console.warn("customers:", e));
    }

    // The list follows the URL.
    $effect(() => {
        const qs = queryString;
        untrack(() => load(qs));
    });
    onMount(loadCustomers);

    function retry(): void {
        error = null;
        invoices = null;
        load(queryString);
    }

    /** Go to the list for `next`: a history step, or (`replace`) in place of this one. */
    function setQuery(next: InvoiceQuery, options: {replace: boolean} = {replace: false}): void {
        jumpNote = null;
        const qs = invoiceQueryString(next);
        if (qs === queryString) return;
        const base = resolve("/invoices");
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
        void goto(qs === "" ? base : `${base}?${qs}`, {replace: options.replace, reset: false});
    }

    const openInvoice = (doc: InvoiceEntry): Promise<void> => goto(resolve("/invoices/[number]", {number: doc.number ?? doc.path}));
    const invoiceHref = (doc: InvoiceEntry): string => resolve("/invoices/[number]", {number: doc.number ?? doc.path});

    /** Quick jump (S3): the invoice numbered `text`, or the only one the search finds. */
    async function jump(text: string): Promise<void> {
        jumpNote = null;
        if (text.trim() === "") return;
        try {
            const found = quickJump(await fetchInvoices(undefined, invoiceQueryString({...query, q: text.trim()})), text);
            if (found !== null) await openInvoice(found);
            else jumpNote = `More than one invoice, or none, matches “${text.trim()}”; pick one below.`;
        } catch (e) {
            console.warn("quick jump:", e);
        }
    }

    // Live reload: the list again after a change on disk, the old one on
    // screen meanwhile; the customer names too.
    onFilesChanged(() => {
        load(queryString);
        loadCustomers();
    });

    const shown = $derived(shownInvoiceSort(query));
    const countLine = $derived(
        invoices === null ? "" : filtered ? `${countText(invoices.length, "invoice", "invoices")} match` : countText(invoices.length, "invoice", "invoices")
    );
    // The page's New button shows with any invoices, or while filters may be hiding them (H21).
    const showNew = $derived((invoices?.length ?? 0) > 0 || filtered);
</script>

<svelte:head>
    <title>Invoices · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-6xl flex-col gap-4">
    <!-- The page's New button is top right, primary, as on every list (H21); the empty state has its own. -->
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">Invoices</h1>
        {#if showNew}
            <a class="btn btn-primary btn-sm" href={resolve("/new")} data-testid="invoices-new">New invoice</a>
        {/if}
    </div>
    {#if showNew || invoices === null}
        <InvoiceFilters
            {query}
            customers={customerNames}
            onChange={setQuery}
            onJump={(text) => void jump(text)}
            onClear={() => setQuery(clearInvoiceFilters(query))}
        />
    {/if}
    {#if jumpNote !== null}
        <p class="text-sm text-base-content/70" data-testid="invoice-jump-note">{jumpNote}</p>
    {/if}
    <AsyncSection {view} value={invoices} {error} testid="invoices-error" label="the invoices" loadingLabel="Loading invoices" onRetry={retry}>
        {#snippet children(list)}
            {#if list.length > 0}
                <p class="-mb-2 text-sm text-base-content/70" role="status" data-testid="invoice-count">{countLine}</p>
            {/if}
            <InvoiceList
                invoices={list}
                href={invoiceHref}
                newHref={resolve("/new")}
                onOpen={(doc) => void openInvoice(doc)}
                {shown}
                onSort={(column) => setQuery(sortInvoicesBy(query, column as InvoiceSort))}
                {filtered}
                onClear={() => setQuery(clearInvoiceFilters(query))}
            />
        {/snippet}
    </AsyncSection>
</div>
