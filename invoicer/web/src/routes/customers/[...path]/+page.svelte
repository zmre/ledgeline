<!-- One customer (invoicer PLAN 4b): the form (MBR's contact fields and the
     invoicing keys), MBR's contact card for the saved note, and its account:
     what it owes, its last payment, its invoices (MBR backlinks, M1).

     Saving sends only the fields changed since the note was read, with the
     note's content hash; the server writes them by key, so the rest of the
     note (unknown keys, the body) is never rewritten. If the note changed on
     disk meanwhile the save is refused (409): reload it, or apply the same
     changes over the new version (only the keys changed here are written).
     Live reload: an outside change shows at once when the form is clean;
     with unsaved changes the page says the note changed and keeps them. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {beforeNavigate, goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {ensureSession, fetchTemplates} from "#lib/api.js";
    import {changes} from "#lib/changes.svelte.js";
    import {
        ApiStatusError,
        changedFields,
        cloneForm,
        emptyForm,
        fetchCustomer,
        isDirty,
        saveCustomer,
        type CustomerDetail,
        type CustomerFields,
        type CustomerForm,
    } from "#lib/customers.js";
    import {pageUrl, type InvoiceEntry, type TemplateEntry} from "#lib/invoices.js";
    import CustomerAccount from "#lib/components/CustomerAccount.svelte";
    import CustomerFormView from "#lib/components/CustomerForm.svelte";
    import NoteFrame from "#lib/components/NoteFrame.svelte";

    const key = $derived(page.params.path ?? "");

    const asError = (e: unknown): Error => (e instanceof Error ? e : new Error(String(e)));

    let detail = $state<CustomerDetail | null>(null);
    let loadError = $state<Error | null>(null);
    const view = $derived<DataView>(loadError !== null ? "error" : detail !== null ? "data" : "loading");

    /** The form as the note wrote it when read, and as edited. */
    let original = $state<CustomerForm>(emptyForm());
    let form = $state<CustomerForm>(emptyForm());
    const dirty = $derived(detail !== null && isDirty(original, form));

    let templates = $state<TemplateEntry[]>([]);
    let sessionReady = $state(false);
    let saving = $state(false);
    let saveError = $state<string | null>(null);
    /** The last save was refused: the note changed on disk since it was read. */
    let conflict = $state(false);
    /** The note changed on disk while the form had unsaved changes. */
    let stale = $state(false);
    let savedNote = $state<string | null>(null);
    /** Bumped after the note changes, so MBR's card shows it again. */
    let frameKey = $state(0);

    function apply(d: CustomerDetail): void {
        detail = d;
        original = cloneForm(d.form);
        form = cloneForm(d.form);
        loadError = null;
        conflict = false;
        stale = false;
    }

    function load(target: string): void {
        detail = null;
        loadError = null;
        saveError = null;
        savedNote = null;
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
                if (noteChanged) frameKey += 1;
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

    async function save(fields: CustomerFields, hash: string): Promise<void> {
        const target = key;
        saving = true;
        saveError = null;
        savedNote = null;
        try {
            const saved = await saveCustomer(target, hash, fields);
            apply(await fetchCustomer(target));
            frameKey += 1;
            savedNote = saved.written ? `Saved ${saved.changed.join(", ")}.` : "Nothing to save.";
        } catch (e: unknown) {
            conflict = e instanceof ApiStatusError && e.status === 409;
            saveError = asError(e).message;
        } finally {
            saving = false;
        }
    }

    function onSubmit(event: SubmitEvent): void {
        event.preventDefault();
        if (detail === null) return;
        void save(changedFields(original, form), detail.hash);
    }

    /** The note changed: write this page's changes over the new version (only the keys changed here). */
    async function applyOverNew(): Promise<void> {
        const fields = changedFields(original, form);
        try {
            const fresh = await fetchCustomer(key);
            await save(fields, fresh.hash);
        } catch (e: unknown) {
            saveError = asError(e).message;
        }
    }

    function discard(): void {
        form = cloneForm(original);
        saveError = null;
        conflict = false;
    }

    beforeNavigate(({cancel, to}) => {
        if (dirty && to !== null && !confirm("This customer has unsaved changes. Leave without saving?")) cancel();
    });

    $effect(() => {
        const target = key;
        untrack(() => load(target));
    });

    let seenSeq = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seenSeq) return;
        seenSeq = seq;
        untrack(() => refresh(key));
    });

    onMount(() => {
        fetchTemplates()
            .then((list) => (templates = list))
            .catch((e: unknown) => console.warn("templates:", e));
        ensureSession()
            .then(() => (sessionReady = true))
            .catch((e: unknown) => console.warn("session:", e));
    });

    const invoiceHref = (doc: InvoiceEntry): string => resolve("/invoices/[number]", {number: doc.number ?? doc.path});
    const openInvoice = (doc: InvoiceEntry): void => void goto(resolve("/invoices/[number]", {number: doc.number ?? doc.path}));
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
            <form class="flex flex-col gap-4" onsubmit={onSubmit} data-testid="customer-page">
                <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
                    <h1 class="text-2xl font-semibold tracking-tight" data-testid="customer-heading">{customer.title}</h1>
                    <span class="badge badge-outline badge-sm">{customer.kind === "person" ? "Person" : "Organization"}</span>
                    {#if customer.account.overdue > 0}<span class="badge badge-sm badge-error">Overdue</span>{/if}
                    <span class="text-xs text-base-content/60" title="The note in the books folder">{customer.path}</span>
                    <div class="ml-auto flex items-center gap-2">
                        {#if dirty}<span class="text-sm text-base-content/70" data-testid="unsaved">Unsaved changes</span>{/if}
                        <button type="button" class="btn btn-ghost btn-sm" disabled={!dirty || saving} onclick={discard}>Discard</button>
                        <button type="submit" class="btn btn-primary btn-sm" disabled={!dirty || saving} data-testid="save-customer">
                            {saving ? "Saving…" : "Save"}
                        </button>
                    </div>
                </header>

                {#if stale && !conflict}
                    <div role="status" class="alert alert-soft alert-warning" data-testid="customer-stale">
                        <span>This customer's note changed on disk while you were editing it.</span>
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
                {#if customer.problems.length > 0}
                    <ul class="alert flex flex-col items-start alert-soft alert-warning" data-testid="customer-problems">
                        {#each customer.problems as problem, at (at)}<li>{problem.message}</li>{/each}
                    </ul>
                {/if}

                <div class="grid gap-4 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
                    <div class="flex flex-col gap-4">
                        <CustomerFormView bind:form {templates} />
                        {#if customer.open_pos.length > 0}
                            <section class="rounded-box border border-base-300 p-4 text-sm" data-testid="customer-open-pos">
                                <h2 class="mb-2 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Open POs</h2>
                                <ul>
                                    {#each customer.open_pos as po (po.reference)}
                                        <li>
                                            {po.reference}{po.amount === null ? "" : `, ${po.amount}`}{po.expires === null ? "" : `, expires ${po.expires}`}
                                        </li>
                                    {/each}
                                </ul>
                                <p class="mt-1 text-xs text-base-content/60">Edit <code>open_pos:</code> in the note itself.</p>
                            </section>
                        {/if}
                    </div>
                    <div class="flex flex-col gap-4">
                        <CustomerAccount {customer} {invoiceHref} pageHref={(url) => url} onOpen={openInvoice} />
                        <section class="flex flex-col gap-2">
                            <h2 class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Contact card</h2>
                            {#if sessionReady}
                                {#key frameKey}
                                    <NoteFrame src={pageUrl(customer.path)} title="{customer.title}: contact card" />
                                {/key}
                            {/if}
                            <p class="text-xs text-base-content/60">MBR's page for the saved note; it shows a save at once.</p>
                        </section>
                    </div>
                </div>
            </form>
        {/snippet}
    </AsyncSection>
</div>
