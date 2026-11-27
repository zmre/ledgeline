<!-- One invoice (invoicer PLAN 3b, 3c): the Send panel (its PDF, the Send flow)
     above the page its template renders, and the Books panel under it. The saved invoice is MBR's own page in a frame; another
     template (a starter not copied in yet, say) is the preview page
     (`/.ledgeline-invoicer/preview`), which writes nothing, until "Use this
     template" makes it the invoice's own. When the bill table's written
     amounts disagree with its rows, a notice says so, with Recalculate
     (Phase 3 feedback F9, F10).

     Cash books: Apply payment… (`a`) in the Books panel re-splits an imported
     deposit onto the invoice (I17; ApplyPaymentDialog), then says what it did.

     A sent invoice whose copy of your company differs from the company note
     now offers Update from company (CompanyNotice, PLAN D18); a draft uses the
     company's current details anyway.

     Keyboard (PLAN 5k): `e` edits it; Shift+P exports its PDF (SendPanel); `a`
     applies a payment (cash books, a sent invoice not paid in full); `u`
     updates a sent invoice from your company when its details have changed. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import type {DataView} from "#ui/ui/dataView.js";
    import {ensureSession, fetchInvoice, fetchInvoiceBooks, fetchTemplates, chooseTemplate, recalculate, updateFromCompany} from "#lib/api.js";
    import {kindLabel, mismatches, pageUrl, previewUrl, type InvoiceBooks, type InvoiceDetail, type TemplateEntry} from "#lib/invoices.js";
    import BooksPanel from "#lib/components/BooksPanel.svelte";
    import CompanyNotice from "#lib/components/CompanyNotice.svelte";
    import InvoiceFrame from "#lib/components/InvoiceFrame.svelte";
    import SendSection from "#lib/components/SendSection.svelte";
    import TemplatePicker from "#lib/components/TemplatePicker.svelte";
    import TotalsNotice from "#lib/components/TotalsNotice.svelte";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import StatusBadge from "#lib/components/StatusBadge.svelte";
    import ApplyPaymentDialog from "#lib/components/ApplyPaymentDialog.svelte";
    import {errorText} from "#lib/labels.js";
    import {canApplyPayment, paymentOutcome, type PaymentResult} from "#lib/payments.js";

    const number = $derived(page.params.number ?? "");

    const asError = (e: unknown): Error => (e instanceof Error ? e : new Error(String(e)));

    registerKeys({
        id: "invoice-page",
        bindings: [
            {
                keys: "e",
                label: "Edit the invoice",
                group: "Invoice",
                enabled: () => detail !== null,
                run: () => void goto(resolve("/invoices/[number]/edit", {number})),
            },
            {
                keys: "a",
                label: "Apply a payment…",
                group: "Invoice",
                enabled: () => canApplyPayment(books) && !applying,
                run: () => openApplyPayment(),
            },
            {
                keys: "u",
                label: "Update from your company…",
                group: "Invoice",
                enabled: () => detail?.status === "sent" && detail.company_changed && editing === null,
                run: () => (confirmingCompany = true),
            },
        ],
    });

    let detail = $state<InvoiceDetail | null>(null);
    let detailError = $state<Error | null>(null);
    let books = $state<InvoiceBooks | null>(null);
    let booksError = $state<Error | null>(null);
    let templates = $state<TemplateEntry[]>([]);
    let sessionReady = $state(false);
    let sessionError = $state<Error | null>(null);

    // Apply payment (cash books, I17): the dialog, and what the last apply did.
    let applying = $state(false);
    let applied = $state<PaymentResult | null>(null);

    function openApplyPayment(): void {
        applied = null;
        applying = true;
    }

    function onPaymentApplied(result: PaymentResult): void {
        applying = false;
        applied = result;
        refreshAll(number);
    }

    /** The template the frame shows: the invoice's own (MBR's page), or another (a preview). */
    let chosen = $state<string>("");

    const detailView = $derived<DataView>(detailError !== null ? "error" : detail !== null ? "data" : "loading");
    const booksView = $derived<DataView>(booksError !== null ? "error" : books !== null ? "data" : "loading");
    const own = $derived(detail?.template ?? null);
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

    // The Send panel (PLAN 3c), with its own wiring (SendSection), re-read on every refresh.
    let sendReload = $state(0);
    /** Bumped after a write, so MBR's page in the frame shows the file again. */
    let frameKey = $state(0);

    /** Everything again, keeping what is shown until it arrives; the frame reloads its page. */
    function refreshAll(target: string): void {
        loadDetail(target, true);
        loadBooks(target, true);
        sendReload += 1;
        frameKey += 1;
    }

    // Live reload: a file changed on disk (this invoice, its template, a
    // wikilinked note, the journal), so show it all again.
    onFilesChanged(() => {
        refreshAll(number);
        loadTemplates();
    });

    // The invoice's own writes: its template, its amounts (F9, F10), its copy of your company (D18).
    let editing = $state<"template" | "recalc" | "company" | null>(null);
    let templateError = $state<string | null>(null);
    let recalcError = $state<string | null>(null);
    let companyError = $state<string | null>(null);
    let confirmingCompany = $state(false);
    const mismatched = $derived(mismatches(detail?.table ?? null));

    function loadTemplates(): void {
        fetchTemplates()
            .then((list) => (templates = list))
            .catch((e: unknown) => console.warn("templates:", e));
    }

    async function onUseTemplate(template: string): Promise<void> {
        const target = number;
        editing = "template";
        templateError = null;
        try {
            await chooseTemplate(target, template);
            chosen = "";
            loadTemplates();
            refreshAll(target);
        } catch (e: unknown) {
            templateError = asError(e).message;
        } finally {
            editing = null;
        }
    }

    async function onRecalculate(rebook: boolean): Promise<void> {
        const target = number;
        editing = "recalc";
        recalcError = null;
        try {
            await recalculate(target, {rebook});
            refreshAll(target);
        } catch (e: unknown) {
            recalcError = asError(e).message;
        } finally {
            editing = null;
        }
    }

    async function onUpdateFromCompany(): Promise<void> {
        const target = number;
        editing = "company";
        companyError = null;
        try {
            await updateFromCompany(target);
            refreshAll(target);
        } catch (e: unknown) {
            companyError = asError(e).message;
        } finally {
            editing = null;
        }
    }

    $effect(() => {
        const target = number;
        chosen = "";
        templateError = null;
        recalcError = null;
        companyError = null;
        confirmingCompany = false;
        applying = false;
        applied = null;
        loadDetail(target);
        loadBooks(target);
    });

    onMount(() => {
        loadTemplates();
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
                <a class="btn btn-outline btn-xs" href={resolve("/invoices/[number]/edit", {number})} data-testid="invoice-edit">Edit</a>
                <span class="text-base-content/70" data-testid="invoice-customer">{doc.invoicee.name ?? ""}</span>
                {#if doc.table?.total}
                    <span class="ml-auto text-lg font-semibold tabular-nums" data-testid="invoice-total">{doc.table.total}</span>
                {/if}
            </header>

            <!-- Top to bottom, each full width (Patrick, 2026-11-23): sending, the
                 invoice as wide as the page allows, then the books with room for their tables. -->
            <SendSection {number} reload={sendReload} onChanged={() => refreshAll(number)} />

            <section class="flex min-w-0 flex-col gap-2" aria-label="The invoice">
                <CompanyNotice
                    number={doc.number ?? number}
                    status={doc.status}
                    changed={doc.company_changed}
                    busy={editing === "company"}
                    error={companyError}
                    bind:confirming={confirmingCompany}
                    onUpdate={onUpdateFromCompany}
                />
                <TotalsNotice
                    number={doc.number ?? number}
                    status={doc.status}
                    items={mismatched}
                    busy={editing === "recalc"}
                    error={recalcError}
                    {onRecalculate}
                />
                <TemplatePicker {templates} {own} bind:chosen busy={editing === "template"} error={templateError} onUse={onUseTemplate} />
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
                    {#if applied !== null}
                        <div class="alert flex-col items-start gap-1 text-sm alert-success" role="status" data-testid="books-payment-applied">
                            <span>{applied.date} {applied.description}: {paymentOutcome(applied)}</span>
                            {#each applied.warnings as warning, i (i)}<span class="text-warning-content" data-testid="books-payment-warning"
                                    >{errorText(warning)}</span
                                >{/each}
                        </div>
                    {/if}
                    <BooksPanel books={panel} onApplyPayment={openApplyPayment} />
                {/snippet}
            </AsyncSection>
        {/snippet}
    </AsyncSection>
</div>

{#if applying}
    <ApplyPaymentDialog {number} onApplied={onPaymentApplied} onCancel={() => (applying = false)} />
{/if}
