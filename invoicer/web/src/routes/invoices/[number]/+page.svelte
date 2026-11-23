<!-- One invoice (invoicer PLAN 3b): the page its template renders, beside the
     Books panel. The saved invoice is MBR's own page in a frame; another
     template (a starter not copied in yet, say) is a `POST /api/preview`,
     which writes nothing. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {ensureSession, fetchInvoice, fetchInvoiceBooks, fetchTemplates, renderPreview} from "#lib/api.js";
    import {kindLabel, pageUrl, templateChoices, type InvoiceBooks, type InvoiceDetail, type TemplateEntry} from "#lib/invoices.js";
    import BooksPanel from "#lib/components/BooksPanel.svelte";
    import InvoiceFrame from "#lib/components/InvoiceFrame.svelte";
    import StatusBadge from "#lib/components/StatusBadge.svelte";

    const number = $derived(page.params.number ?? "");

    const asError = (e: unknown): Error => (e instanceof Error ? e : new Error(String(e)));

    let detail = $state<InvoiceDetail | null>(null);
    let detailError = $state<Error | null>(null);
    let books = $state<InvoiceBooks | null>(null);
    let booksError = $state<Error | null>(null);
    let templates = $state<TemplateEntry[]>([]);
    let sessionReady = $state(false);
    let sessionError = $state<Error | null>(null);

    /** The template the frame shows: the invoice's own (MBR's page), or another (a preview). */
    let chosen = $state<string>("");
    let preview = $state<string | null>(null);
    let previewError = $state<Error | null>(null);

    const detailView = $derived<DataView>(detailError !== null ? "error" : detail !== null ? "data" : "loading");
    const booksView = $derived<DataView>(booksError !== null ? "error" : books !== null ? "data" : "loading");
    const own = $derived(detail?.template ?? null);
    const choices = $derived(templateChoices(templates, own));
    const showingOwn = $derived(chosen === "" || chosen === own);

    function loadDetail(target: string): void {
        detail = null;
        detailError = null;
        fetchInvoice(target)
            .then((d) => {
                if (target === number) detail = d;
            })
            .catch((e: unknown) => (detailError = asError(e)));
    }

    function loadBooks(target: string): void {
        books = null;
        booksError = null;
        fetchInvoiceBooks(target)
            .then((b) => {
                if (target === number) books = b;
            })
            .catch((e: unknown) => (booksError = asError(e)));
    }

    $effect(() => {
        const target = number;
        chosen = "";
        preview = null;
        previewError = null;
        loadDetail(target);
        loadBooks(target);
    });

    onMount(() => {
        fetchTemplates()
            .then((list) => (templates = list))
            .catch((e: unknown) => console.warn("templates:", e));
        ensureSession()
            .then(() => (sessionReady = true))
            .catch((e: unknown) => (sessionError = asError(e)));
    });

    // Another template: render the saved invoice through it (no writes).
    $effect(() => {
        const target = number;
        const template = chosen;
        if (template === "" || template === own) {
            preview = null;
            return;
        }
        preview = null;
        previewError = null;
        renderPreview({number: target, template})
            .then((rendered) => {
                if (target === number && template === chosen) preview = rendered.html;
            })
            .catch((e: unknown) => (previewError = asError(e)));
    });
</script>

<svelte:head>
    <title>{detail?.title ?? `Invoice ${number}`} · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-7xl flex-col gap-4">
    <a class="link text-sm link-hover" href={resolve("/invoices")}>← Invoices</a>
    <AsyncSection
        view={detailView}
        value={detail}
        error={detailError}
        testid="invoice-error"
        label="this invoice"
        loadingLabel="Loading the invoice"
        onRetry={() => loadDetail(number)}
    >
        {#snippet children(doc)}
            <header class="flex flex-wrap items-center gap-x-3 gap-y-1">
                <h1 class="text-2xl font-semibold tracking-tight" data-testid="invoice-heading">
                    {kindLabel(doc.doc_type) ?? "Invoice"}
                    {doc.number ?? ""}
                </h1>
                <StatusBadge {doc} />
                <span class="text-base-content/70">{doc.invoicee.name ?? ""}</span>
                {#if doc.table?.total}
                    <span class="ml-auto text-lg font-semibold tabular-nums" data-testid="invoice-total">{doc.table.total}</span>
                {/if}
            </header>

            <div class="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
                <section class="flex min-w-0 flex-col gap-2" aria-label="The invoice">
                    <div class="flex flex-wrap items-center gap-2">
                        <label class="text-sm text-base-content/70" for="template-choice">Template</label>
                        <select id="template-choice" class="select-bordered select select-sm" bind:value={chosen} data-testid="template-choice">
                            {#if own === null}
                                <option value="">As MBR shows the note</option>
                            {/if}
                            {#each choices as template (template.path)}
                                <option value={template.path === own ? "" : template.path}>
                                    {template.name}{template.path === own ? " (this invoice's)" : template.installed ? "" : " (starter)"}
                                </option>
                            {/each}
                        </select>
                        {#if !showingOwn}
                            <span class="text-xs text-base-content/60">Preview only: the invoice still names {own ?? "no template"}.</span>
                        {/if}
                    </div>
                    {#if !showingOwn && previewError !== null}
                        <div class="alert text-sm alert-error" role="alert" data-testid="preview-error">Couldn't preview it: {previewError.message}</div>
                    {:else if showingOwn && sessionError !== null}
                        <div class="alert text-sm alert-error" role="alert" data-testid="preview-error">Couldn't show the invoice: {sessionError.message}</div>
                    {:else if !showingOwn}
                        {#if preview !== null}
                            <InvoiceFrame html={preview} title="{doc.title}, previewed" />
                        {:else}
                            <div class="flex justify-center py-24" aria-label="Rendering the preview">
                                <span class="loading loading-lg loading-spinner"></span>
                            </div>
                        {/if}
                    {:else if sessionReady}
                        <InvoiceFrame src={pageUrl(doc.path)} title={doc.title} />
                    {/if}
                </section>

                <aside class="min-w-0">
                    <AsyncSection
                        view={booksView}
                        value={books}
                        error={booksError}
                        testid="books-error"
                        label="the books"
                        loadingLabel="Loading the books"
                        onRetry={() => loadBooks(number)}
                    >
                        {#snippet children(panel)}
                            <BooksPanel books={panel} />
                        {/snippet}
                    </AsyncSection>
                </aside>
            </div>
        {/snippet}
    </AsyncSection>
</div>
