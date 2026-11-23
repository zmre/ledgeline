<!-- One invoice (invoicer PLAN 3b, 3c): the page its template renders, beside the
     Send panel (its PDF, the Send flow) and the Books panel. The saved invoice is MBR's own page in a frame; another
     template (a starter not copied in yet, say) is the preview page
     (`/.ledgeline-invoicer/preview`), which writes nothing. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {
        copyPdf,
        ensureSession,
        exportPdf,
        fetchDelivery,
        fetchInvoice,
        fetchInvoiceBooks,
        fetchTemplates,
        openPortal,
        sendInvoice,
        sharePdf,
    } from "#lib/api.js";
    import type {Anchor, Delivery, Destination, Exported} from "#lib/delivery.js";
    import {kindLabel, pageUrl, previewUrl, templateChoices, type InvoiceBooks, type InvoiceDetail, type TemplateEntry} from "#lib/invoices.js";
    import BooksPanel from "#lib/components/BooksPanel.svelte";
    import InvoiceFrame from "#lib/components/InvoiceFrame.svelte";
    import SendPanel from "#lib/components/SendPanel.svelte";
    import {changes} from "#lib/changes.svelte.js";
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

    const detailView = $derived<DataView>(detailError !== null ? "error" : detail !== null ? "data" : "loading");
    const booksView = $derived<DataView>(booksError !== null ? "error" : books !== null ? "data" : "loading");
    const own = $derived(detail?.template ?? null);
    const choices = $derived(templateChoices(templates, own));
    const showingOwn = $derived(chosen === "" || chosen === own);

    /** Load the invoice; `quiet` keeps what is shown until the new one arrives (a refresh). */
    function loadDetail(target: string, quiet = false): void {
        if (!quiet) {
            detail = null;
            detailError = null;
        }
        fetchInvoice(target)
            .then((d) => {
                if (target !== number) return;
                detail = d;
                detailError = null;
            })
            .catch((e: unknown) => {
                if (target === number) detailError = asError(e);
            });
    }

    function loadBooks(target: string, quiet = false): void {
        if (!quiet) {
            books = null;
            booksError = null;
        }
        fetchInvoiceBooks(target)
            .then((b) => {
                if (target !== number) return;
                books = b;
                booksError = null;
            })
            .catch((e: unknown) => {
                if (target === number) booksError = asError(e);
            });
    }

    // The Send panel (PLAN 3c).
    let delivery = $state<Delivery | null>(null);
    let exported = $state<Exported | null>(null);
    let sending = $state(false);
    let sendError = $state<string | null>(null);
    /** Bumped after a write, so MBR's page in the frame shows the file again. */
    let frameKey = $state(0);

    function loadDelivery(target: string): void {
        fetchDelivery(target)
            .then((d) => {
                if (target === number) delivery = d;
            })
            .catch((e: unknown) => (sendError = asError(e).message));
    }

    /** Run one Send-panel action: busy while it runs, its error shown. */
    async function act(action: () => Promise<void>): Promise<void> {
        sending = true;
        sendError = null;
        try {
            await action();
        } catch (e: unknown) {
            sendError = asError(e).message;
        } finally {
            sending = false;
        }
    }

    /** Everything again, keeping what is shown until it arrives; the frame reloads its page. */
    function refreshAll(target: string): void {
        loadDetail(target, true);
        loadBooks(target, true);
        loadDelivery(target);
        frameKey += 1;
    }

    // Live reload: a file changed on disk (this invoice, its template, a
    // wikilinked note, the journal), so show it all again.
    let seenSeq = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seenSeq) return;
        seenSeq = seq;
        untrack(() => refreshAll(number));
    });

    const onExport = (options: {temp: boolean}): Promise<void> =>
        act(async () => {
            const target = number;
            exported = await exportPdf(target, options);
            loadDelivery(target);
        });

    const onSendAndExport = (via: string): Promise<void> =>
        act(async () => {
            const target = number;
            await sendInvoice(target, via);
            refreshAll(target);
            exported = await exportPdf(target);
            refreshAll(target);
        });

    const onShare = (destination: Destination, file: string, anchor: Anchor): Promise<void> => act(() => sharePdf(destination, file, anchor));
    const onCopyFile = (destination: Destination, file: string): Promise<void> => act(() => copyPdf(destination, file));
    const onOpenPortal = (): Promise<void> => act(() => openPortal(number));
    const onCopyText = (label: string, text: string): Promise<void> =>
        act(async () => {
            try {
                await navigator.clipboard.writeText(text);
            } catch {
                throw new Error(`Couldn't copy the ${label.toLowerCase()}; select it and copy by hand.`);
            }
        });

    $effect(() => {
        const target = number;
        delivery = null;
        exported = null;
        sendError = null;
        loadDelivery(target);
    });

    $effect(() => {
        const target = number;
        chosen = "";
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
                <span class="text-base-content/70" data-testid="invoice-customer">{doc.invoicee.name ?? ""}</span>
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
                    {#if sessionError !== null}
                        <div class="alert text-sm alert-error" role="alert" data-testid="preview-error">Couldn't show the invoice: {sessionError.message}</div>
                    {:else if sessionReady}
                        <!-- Re-made after a write or a change on disk (frameKey), so it shows the file again. -->
                        {#key frameKey}
                            {#if showingOwn}
                                <InvoiceFrame src={pageUrl(doc.path)} title={doc.title} />
                            {:else}
                                <InvoiceFrame src={previewUrl(number, chosen)} title="{doc.title}, previewed" mode="preview" />
                            {/if}
                        {/key}
                    {/if}
                </section>

                <aside class="flex min-w-0 flex-col gap-6">
                    {#if delivery !== null}
                        <SendPanel
                            {delivery}
                            {exported}
                            busy={sending}
                            error={sendError}
                            {onExport}
                            {onSendAndExport}
                            {onShare}
                            {onCopyFile}
                            {onOpenPortal}
                            {onCopyText}
                        />
                    {:else if sendError !== null}
                        <div class="alert text-sm alert-error" role="alert" data-testid="send-error">{sendError}</div>
                    {/if}
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
