<!-- One customer (invoicer PLAN 4b; Phase 4 feedback J1–J10).

     It opens read-only: the contact, how it is invoiced, contracts, what it
     owes, its open invoices and every invoice, and the notes that link it
     (`CustomerView`, `CustomerAccount`). Edit swaps the view for the form;
     Save and Discard are top right, and Cancel returns to the view.
     Leaving with unsaved changes asks: Save, Discard, or stay.

     Saving sends only the fields changed since the note was read, with the
     note's content hash; the server writes them by key, so the rest of the
     note (unknown keys, the body) is never rewritten. If the note changed on
     disk meanwhile the save is refused (409): reload it, or apply the same
     changes over the new version (only the keys changed here are written).
     Live reload: an outside change shows at once when the form is clean;
     with unsaved changes the page says the note changed and keeps them.

     The view is a contact card (contact design, 2026-11-26): the header
     (`CustomerCardHeader`: who, what to do now, what is owed), the body
     (`CustomerView`, with `OpenPos` in it), and the invoices beside it
     (`CustomerAccount`, sticky on a wide window).

     Keyboard (PLAN 5k): `e` edits; `m` emails the default address, `p`
     opens the portal, `n` starts an invoice for this customer (over the
     app's `n`), `y e` / `y p` / `y a` / `y s` copy the default email, phone,
     address or supplier ID, `j`/`k`/`o` walk the invoices; each is also a
     button. While editing, Cmd/Ctrl+S saves and Escape cancels (from a
     field too, once a list in it is closed), asking first when something is
     unsaved. Focus goes back to Edit after a save or Cancel. -->
<script lang="ts">
    import {onMount, tick, untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import type {DataView} from "#ui/ui/dataView.js";
    import {ensureSession, fetchTemplates} from "#lib/api.js";
    import {copyValue} from "#lib/copy.svelte.js";
    import {fetchEditorOptions} from "#lib/editorApi.js";
    import {handoffHref} from "#lib/frameLinks.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {unsavedGuard} from "#lib/unsavedGuard.svelte.js";
    import {
        ApiStatusError,
        changedFields,
        cloneForm,
        defaultOf,
        emptyForm,
        entryOf,
        fetchCustomer,
        fetchCustomers,
        fetchFiles,
        isDirty,
        linkName,
        mailtoHref,
        openBooksFile,
        openCustomerPortal,
        routeParam,
        saveCustomer,
        savedText,
        telHref,
        type CustomerDetail,
        type CustomerFields,
        type CustomerForm,
        type PoForm,
    } from "#lib/customers.js";
    import {pageUrl, type InvoiceEntry, type TemplateEntry} from "#lib/invoices.js";
    import {fieldList, friendlyMessage} from "#lib/labels.js";
    import CustomerAccount from "#lib/components/CustomerAccount.svelte";
    import CustomerCardHeader from "#lib/components/CustomerCardHeader.svelte";
    import CustomerFormView from "#lib/components/CustomerForm.svelte";
    import CustomerView from "#lib/components/CustomerView.svelte";
    import OpenPos from "#lib/components/OpenPos.svelte";
    import SaveDiscardDialog from "#lib/components/SaveDiscardDialog.svelte";

    const key = $derived(page.params.path ?? "");

    const asError = (e: unknown): Error => (e instanceof Error ? e : new Error(String(e)));
    const messageOf = (e: unknown): string => friendlyMessage(asError(e).message);

    let detail = $state<CustomerDetail | null>(null);
    let loadError = $state<Error | null>(null);
    const view = $derived<DataView>(loadError !== null ? "error" : detail !== null ? "data" : "loading");

    /** Read-only (the default) or the form. */
    let editing = $state(false);
    /** The form as the note wrote it when read, and as edited (the company by name). */
    let original = $state<CustomerForm>(emptyForm());
    let form = $state<CustomerForm>(emptyForm());
    const dirty = $derived(detail !== null && editing && isDirty(original, form));

    let templates = $state<TemplateEntry[]>([]);
    let companies = $state<string[]>([]);
    /** The books' accounts, and those that are revenue, for the revenue account's suggestions. */
    let accounts = $state<{all: string[]; revenue: string[]} | null>(null);
    /** The window can open a books file in its viewer. */
    let canOpenFiles = $state(false);
    let saving = $state(false);
    let saveError = $state<string | null>(null);
    /** The last save was refused: the note changed on disk since it was read. */
    let conflict = $state(false);
    /** The note changed on disk while the form had unsaved changes. */
    let stale = $state(false);
    let savedNote = $state<string | null>(null);
    let openError = $state<string | null>(null);

    /** The note's form as the page edits it: the company by name (the server writes the link). */
    function editable(f: CustomerForm): CustomerForm {
        const copy = cloneForm(f);
        copy.company = linkName(copy.company);
        return copy;
    }

    function apply(d: CustomerDetail): void {
        detail = d;
        original = editable(d.form);
        form = editable(d.form);
        loadError = null;
        conflict = false;
        stale = false;
    }

    function load(target: string): void {
        detail = null;
        loadError = null;
        saveError = null;
        savedNote = null;
        editing = false;
        fetchCustomer(target)
            .then((d) => {
                if (target === key) apply(d);
            })
            .catch((e: unknown) => {
                if (target === key) loadError = asError(e);
            });
    }

    /** After a change on disk: everything, unless the form has unsaved changes and the note itself changed. */
    function refresh(target: string): void {
        fetchCustomer(target)
            .then((d) => {
                if (target !== key || detail === null) return;
                const noteChanged = d.hash !== detail.hash;
                if (!dirty) {
                    apply(d);
                } else if (noteChanged) {
                    // Keep the form and the hash it was filled from: a save is
                    // then refused, and the page offers the choice.
                    detail = {...d, hash: detail.hash, form: detail.form};
                    stale = true;
                } else {
                    detail = {...d, form: detail.form};
                }
            })
            .catch((e: unknown) => console.warn("customer:", e));
    }

    /** What the last save sent: "Apply my changes to the new version" sends it again. */
    let lastFields: CustomerFields | null = null;

    /** Write `fields` over `hash`; true when saved (the page is back in the view). */
    async function save(fields: CustomerFields, hash: string): Promise<boolean> {
        const target = key;
        lastFields = fields;
        saving = true;
        saveError = null;
        savedNote = null;
        try {
            const saved = await saveCustomer(target, hash, fields);
            apply(await fetchCustomer(target));
            editing = false;
            savedNote = savedText(saved, fieldList);
            return true;
        } catch (e: unknown) {
            conflict = e instanceof ApiStatusError && e.status === 409;
            saveError = messageOf(e);
            return false;
        } finally {
            saving = false;
        }
    }

    function onSubmit(event: SubmitEvent): void {
        event.preventDefault();
        if (detail === null || !dirty) return;
        void save(changedFields(original, form), detail.hash);
    }

    /** The note changed: write this page's changes over the new version (only the keys changed here). */
    async function applyOverNew(): Promise<void> {
        const fields = editing ? changedFields(original, form) : (lastFields ?? {});
        try {
            const fresh = await fetchCustomer(key);
            await save(fields, fresh.hash);
        } catch (e: unknown) {
            saveError = messageOf(e);
        }
    }

    /** The view's open POs (S5): the whole list, written by key at once. */
    function savePos(pos: PoForm[]): Promise<boolean> {
        if (detail === null) return Promise.resolve(false);
        return save({open_pos: pos}, detail.hash);
    }

    function discard(): void {
        form = cloneForm(original);
        saveError = null;
        conflict = false;
    }

    function startEditing(): void {
        savedNote = null;
        form = cloneForm(original);
        editing = true;
        if (accounts === null) {
            fetchEditorOptions()
                .then((o) => (accounts = {all: o.accounts, revenue: o.revenue_accounts ?? []}))
                .catch((e: unknown) => console.warn("accounts:", e));
        }
        if (companies.length === 0) {
            fetchCustomers()
                .then((rows) => (companies = rows.filter((r) => r.kind === "organization" && r.path !== detail?.path).map((r) => r.title)))
                .catch((e: unknown) => console.warn("customers:", e));
        }
    }

    function stopEditing(): void {
        discard();
        stale = false;
        editing = false;
    }

    /** Cancel: back to the view, asking first when something is unsaved. */
    // Leaving, or Cancel, with unsaved changes asks Save / Discard / stay.
    const guard = unsavedGuard({
        dirty: () => dirty,
        save: async () => (detail !== null && (await save(changedFields(original, form), detail.hash)) ? "saved" : "failed"),
        discard: stopEditing,
        saveError: () => saveError,
        noun: "customer",
    });

    function cancel(): void {
        guard.proceed(stopEditing);
    }

    let formEl = $state<HTMLFormElement | null>(null);

    // Back to Edit after a save or Cancel: the form that had focus is gone.
    let wasEditing = false;
    $effect(() => {
        const now = editing;
        if (wasEditing && !now) void tick().then(() => formEl?.querySelector<HTMLElement>('[data-testid="edit-customer"]')?.focus());
        wasEditing = now;
    });

    /** Click one of the header's actions (a key does what its button does). */
    function act(testid: string): void {
        formEl?.querySelector<HTMLElement>(`[data-testid="${testid}"]`)?.click();
    }

    const viewing = (): boolean => detail !== null && !editing;
    const hasEmail = (): boolean => viewing() && (detail?.form.emails ?? []).some((e) => mailtoHref(e.value) !== null);
    const hasPhone = (): boolean => viewing() && (detail?.form.phones ?? []).some((p) => telHref(p.value) !== null);
    /** The default email, phone or address (billing, then work, then the first), to copy. */
    const defaultValue = (list: "emails" | "phones" | "addresses"): string | null => defaultOf(detail?.form[list] ?? [])?.value ?? null;
    const supplierId = (): string | null => detail?.form.billing_portal?.supplier_id ?? null;
    const copyKey = (keys: string, what: string, value: () => string | null) => ({
        keys,
        label: `Copy the ${what}`,
        group: "Customer" as const,
        enabled: () => viewing() && value() !== null,
        run: () => {
            const text = value();
            if (text !== null) void copyValue(text, what);
        },
    });

    registerKeys({
        id: "customer-page",
        bindings: [
            {keys: "e", label: "Edit the customer", group: "Customer", enabled: () => detail !== null && !editing, run: startEditing},
            {keys: "m", label: "Email the customer", group: "Customer", enabled: hasEmail, run: () => act("action-email")},
            {
                keys: "p",
                label: "Open the customer's portal",
                group: "Customer",
                enabled: () => viewing() && (detail?.portal_url ?? null) !== null,
                run: () => act("action-portal"),
            },
            {keys: "n", label: "New invoice for this customer", group: "Customer", enabled: viewing, run: () => act("action-new-invoice")},
            copyKey("y e", "default email", () => (hasEmail() ? defaultValue("emails") : null)),
            copyKey("y p", "default phone", () => (hasPhone() ? defaultValue("phones") : null)),
            copyKey("y a", "default address", () => defaultValue("addresses")),
            copyKey("y s", "supplier ID", supplierId),
            {
                keys: "Escape",
                label: "Cancel editing",
                group: "Customer",
                fields: true,
                within: () => formEl,
                enabled: () => editing && !saving && guard.pending === null,
                run: cancel,
            },
            {
                keys: "mod+s",
                label: "Save the customer",
                group: "Customer",
                always: true,
                enabled: () => editing,
                run: () => {
                    if (detail !== null && dirty && !saving) void save(changedFields(original, form), detail.hash);
                },
            },
        ],
    });

    $effect(() => {
        const target = key;
        untrack(() => load(target));
    });

    onFilesChanged(() => refresh(key));

    onMount(() => {
        fetchTemplates()
            .then((list) => (templates = list))
            .catch((e: unknown) => console.warn("templates:", e));
        fetchFiles({list: false})
            .then((caps) => (canOpenFiles = caps.open))
            .catch((e: unknown) => console.warn("files:", e));
        // The cookie MBR needs to serve a contract file in a new tab.
        ensureSession().catch((e: unknown) => console.warn("session:", e));
    });

    const invoiceHref = (number: string): string => resolve("/invoices/[number]", {number});
    const openInvoice = (doc: InvoiceEntry): void => void goto(resolve("/invoices/[number]", {number: doc.number ?? doc.path}));
    const customerHref = (path: string): string => resolve("/customers/[...path]", {path: routeParam(path)});
    const noteHref = (path: string): string => resolve("/notes/[...path]", {path: routeParam(path)});
    /** MBR serves a file at its path in the books folder (behind the session cookie). */
    const fileHref = (path: string): string => pageUrl(path).replace(/\/$/, "");

    function openFile(path: string): void {
        openError = null;
        openBooksFile(path).catch((e: unknown) => (openError = messageOf(e)));
    }

    function openPortal(): void {
        if (detail === null) return;
        openError = null;
        openCustomerPortal(detail.path).catch((e: unknown) => (openError = messageOf(e)));
    }

    /** A mailto or tel link as this window follows it: the desktop window's hand-off, else as is. */
    const external = (href: string): string => (canOpenFiles ? handoffHref(href) : href);
    const newInvoiceHref = (title: string): string => `${resolve("/new")}?customer=${encodeURIComponent(title)}`;

    /** A person's company, when it is a customer: its page. */
    const companyHref = $derived(detail !== null && detail.kind === "person" && detail.billing.note !== detail.path ? customerHref(detail.billing.note) : null);
    /** What to call each contract and PO attachment entry in the form. */
    const contractNames = $derived(
        Object.fromEntries(
            [...(detail?.contracts ?? []), ...(detail?.po_usage ?? []).flatMap((po) => (po.attachment == null ? [] : [po.attachment]))].map((c) => [
                entryOf(c),
                c.name,
            ])
        ) as Record<string, string>
    );
    /** The note's folder, so links in its notes resolve as on its page. */
    const linkBase = $derived(detail === null ? "/" : pageUrl(detail.path).replace(/[^/]+\/$/, ""));
</script>

<svelte:head>
    <title>{detail?.title ?? "Customer"} · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-7xl flex-col gap-4">
    <a class="link text-sm link-hover" href={resolve("/customers")}>← Customers</a>
    <AsyncSection
        {view}
        value={detail}
        error={loadError}
        testid="customer-load-error"
        label="this customer"
        loadingLabel="Loading the customer"
        onRetry={() => load(key)}
    >
        {#snippet children(customer)}
            <form class="flex flex-col gap-4" onsubmit={onSubmit} bind:this={formEl} data-testid="customer-page" data-mode={editing ? "edit" : "view"}>
                <CustomerCardHeader
                    {customer}
                    {editing}
                    {companyHref}
                    {external}
                    newInvoiceHref={newInvoiceHref(customer.title)}
                    onOpenPortal={customer.portal_url == null ? undefined : openPortal}
                >
                    {#snippet controls()}
                        {#if editing}
                            {#if dirty}<span class="text-sm text-muted" data-testid="unsaved">Unsaved changes</span>{/if}
                            <button type="button" class="btn btn-ghost btn-sm" disabled={saving} onclick={cancel} data-testid="cancel-edit">Cancel</button>
                            <button type="button" class="btn btn-outline btn-sm" disabled={!dirty || saving} onclick={discard} data-testid="discard-customer"
                                >Discard</button
                            >
                            <button type="submit" class="btn btn-primary btn-sm" disabled={!dirty || saving} data-testid="save-customer">
                                {saving ? "Saving…" : "Save"}
                            </button>
                        {:else}
                            <button
                                type="button"
                                class="btn btn-sm"
                                onclick={startEditing}
                                title="Edit the customer (e)"
                                aria-keyshortcuts="e"
                                data-testid="edit-customer">Edit</button
                            >
                        {/if}
                    {/snippet}
                </CustomerCardHeader>

                {#if stale && !conflict}
                    <div role="status" class="alert alert-soft alert-warning" data-testid="customer-stale">
                        <span>This customer changed on disk while you were editing it.</span>
                        <button type="button" class="btn btn-sm" onclick={() => load(key)}>Reload it (discard my changes)</button>
                    </div>
                {/if}
                {#if saveError !== null}
                    <div role="alert" class="alert alert-soft alert-error" data-testid="customer-save-error">
                        <span>{saveError}</span>
                        {#if conflict}
                            <div class="flex flex-wrap gap-2">
                                <button type="button" class="btn btn-sm" onclick={() => load(key)} data-testid="conflict-reload"
                                    >Reload (discard my changes)</button
                                >
                                <button type="button" class="btn btn-primary btn-sm" onclick={() => void applyOverNew()} data-testid="conflict-apply"
                                    >Apply my changes to the new version</button
                                >
                            </div>
                        {/if}
                    </div>
                {/if}
                {#if savedNote !== null && !dirty}
                    <div role="status" class="text-sm text-success" data-testid="customer-saved">{savedNote}</div>
                {/if}
                {#if openError !== null}
                    <div role="alert" class="alert alert-soft alert-error" data-testid="customer-open-error">{openError}</div>
                {/if}
                {#if customer.problems.length > 0}
                    <ul class="alert flex flex-col items-start alert-soft alert-warning" data-testid="customer-problems">
                        {#each customer.problems as problem, at (at)}<li>{friendlyMessage(problem.message)}</li>{/each}
                    </ul>
                {/if}

                <div class="mt-2 grid gap-8 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)] lg:gap-10">
                    <div class="flex min-w-0 flex-col gap-4">
                        {#if editing}
                            <CustomerFormView
                                bind:form
                                {templates}
                                {companies}
                                {contractNames}
                                {linkBase}
                                accounts={accounts?.all ?? []}
                                revenueAccounts={accounts?.revenue ?? []}
                            />
                        {:else}
                            <CustomerView
                                {customer}
                                {templates}
                                {noteHref}
                                {fileHref}
                                {external}
                                onOpenFile={canOpenFiles ? openFile : undefined}
                                onOpenPortal={customer.portal_url == null ? undefined : openPortal}
                            >
                                {#snippet pos()}
                                    <OpenPos
                                        usage={customer.po_usage}
                                        busy={saving}
                                        onSave={savePos}
                                        {invoiceHref}
                                        {noteHref}
                                        {fileHref}
                                        onOpenFile={canOpenFiles ? openFile : undefined}
                                    />
                                {/snippet}
                            </CustomerView>
                        {/if}
                    </div>
                    <div class="flex min-w-0 flex-col gap-4 lg:sticky lg:top-4 lg:max-h-[calc(100dvh-2rem)] lg:self-start lg:overflow-y-auto">
                        <CustomerAccount {customer} {invoiceHref} {customerHref} {noteHref} onOpen={openInvoice} />
                    </div>
                </div>
            </form>
        {/snippet}
    </AsyncSection>
</div>

{#if guard.pending !== null}
    <SaveDiscardDialog
        message={guard.pending.leaving ? "This customer has unsaved changes. Save them before you leave?" : "This customer has unsaved changes."}
        {saving}
        error={guard.error}
        onSave={() => void guard.save()}
        onDiscard={() => void guard.discard()}
        onCancel={() => guard.cancel()}
    />
{/if}
