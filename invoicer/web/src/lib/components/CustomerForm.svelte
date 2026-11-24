<!-- The customer form (invoicer PLAN 4b): MBR's contact fields (name, person
     or organization, a person's company, labelled emails, phones and
     addresses), then the invoicing keys the invoicer reads (terms, default
     template, revenue account, the name in the books, contracts, a vendor
     portal). It edits `form` in place; the page decides what to send
     (only the changed fields: `changedFields`).

     Names, never syntax (Phase 4 feedback J6): the company is typed or
     picked by name (the server writes the link); contracts are listed by
     name and added from the books folder (`ContractPicker`). Fields wrap
     to one column on a narrow window (J2: help text wraps instead of
     running under the next field). -->
<script lang="ts">
    import {contractName, emptyPortal, type CustomerForm, type FileCandidate} from "#lib/customers.js";
    import type {TemplateEntry} from "#lib/invoices.js";
    import ContractPicker from "./ContractPicker.svelte";
    import LabelledList from "./LabelledList.svelte";

    let {
        form = $bindable(),
        templates = [],
        creating = false,
        companies = [],
        contractNames = {},
    }: {
        form: CustomerForm;
        /** The invoice templates, for the default-template picker. */
        templates?: TemplateEntry[];
        /** A new customer: the name is the file's name too. */
        creating?: boolean;
        /** Organization names a person's company may be (suggestions). */
        companies?: string[];
        /** What to call each contract entry (the server's names); else from the entry. */
        contractNames?: Record<string, string>;
    } = $props();

    const uid = $props.id();
    let pickingContract = $state(false);
    /** Names of contracts added here, before the server has named them. */
    const added = $state<Record<string, string>>({});
    const nameOf = (entry: string): string => contractNames[entry] ?? added[entry] ?? contractName(entry);

    function addContract(file: FileCandidate): void {
        added[file.entry] = file.name;
        if (!form.contracts.includes(file.entry)) form.contracts.push(file.entry);
        pickingContract = false;
    }

    const ADDRESS_PLACEHOLDER = "1 Main St\nSpringfield, CO 80000";

    const invoiceTemplates = $derived(templates.filter((t) => t.kinds.includes("invoice")));
    const knownTemplate = $derived(form.invoice_template === "" || invoiceTemplates.some((t) => t.path === form.invoice_template));

    function setPortal(on: boolean): void {
        form.billing_portal = on ? (form.billing_portal ?? emptyPortal()) : null;
    }

    const termsValue = $derived(form.terms === null ? "" : String(form.terms));
    function setTerms(text: string): void {
        const t = text.trim();
        form.terms = t === "" ? null : Number(t);
    }
</script>

<div class="flex flex-col gap-4" data-testid="customer-form">
    <section class="rounded-box border border-base-300 p-4">
        <h2 class="mb-2 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Contact</h2>
        <div class="grid gap-3 sm:grid-cols-2">
            <label class="fieldset min-w-0">
                <span class="fieldset-legend">Name</span>
                <input class="input w-full input-sm" name="title" required bind:value={form.title} data-testid="customer-title" />
                <span class="label text-xs whitespace-normal">
                    {creating
                        ? "Invoices name the customer by it, and its note is named after it."
                        : "Invoices name the customer by it. Renaming keeps the name the books know."}
                </span>
            </label>
            <fieldset class="fieldset min-w-0">
                <legend class="fieldset-legend">Kind</legend>
                <div class="flex flex-wrap gap-x-4 gap-y-2">
                    <label class="flex items-center gap-2">
                        <input type="radio" class="radio radio-sm" name="kind" value="organization" bind:group={form.kind} />
                        Organization
                    </label>
                    <label class="flex items-center gap-2">
                        <input type="radio" class="radio radio-sm" name="kind" value="person" bind:group={form.kind} />
                        Person
                    </label>
                </div>
            </fieldset>
            {#if form.kind === "person"}
                <label class="fieldset min-w-0 sm:col-span-2">
                    <span class="fieldset-legend">Company</span>
                    <input
                        class="input w-full input-sm"
                        name="company"
                        placeholder="The organization's name"
                        list="{uid}-companies"
                        autocomplete="off"
                        bind:value={form.company}
                        data-testid="customer-company"
                    />
                    <datalist id="{uid}-companies">
                        {#each companies as name (name)}<option value={name}></option>{/each}
                    </datalist>
                    <span class="label text-xs whitespace-normal"
                        >A person at a company is billed to the company: its name, and its billing email and address.</span
                    >
                </label>
            {/if}
        </div>
        <LabelledList bind:items={form.emails} legend="Emails" noun="email" name="emails" type="email" placeholder="ap@example.com" />
        <LabelledList bind:items={form.phones} legend="Phones" noun="phone" name="phones" type="tel" placeholder="+1 303 555 0100" />
        <LabelledList bind:items={form.addresses} legend="Addresses" noun="address" name="addresses" multiline placeholder={ADDRESS_PLACEHOLDER} />
        <p class="mt-1 text-xs text-base-content/70">Invoices use the one labelled “billing”, then “work”, then the first.</p>
    </section>

    <section class="rounded-box border border-base-300 p-4">
        <h2 class="mb-2 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Invoicing</h2>
        <div class="grid gap-3 sm:grid-cols-2">
            <label class="fieldset min-w-0">
                <span class="fieldset-legend">Terms (net days)</span>
                <input
                    class="input w-full input-sm"
                    name="terms"
                    type="number"
                    min="0"
                    max="3650"
                    step="1"
                    value={termsValue}
                    oninput={(e) => setTerms(e.currentTarget.value)}
                />
            </label>
            <label class="fieldset min-w-0">
                <span class="fieldset-legend">Invoice template</span>
                <select class="select w-full select-sm" name="invoice_template" bind:value={form.invoice_template}>
                    <option value="">The default</option>
                    {#each invoiceTemplates as t (t.path)}
                        <option value={t.path}>{t.name}{t.starter && !t.installed ? " (starter)" : ""}</option>
                    {/each}
                    {#if !knownTemplate}
                        <option value={form.invoice_template}>{form.invoice_template} (not found)</option>
                    {/if}
                </select>
            </label>
            <label class="fieldset min-w-0">
                <span class="fieldset-legend">Revenue account</span>
                <input class="input w-full input-sm" name="revenue_account" placeholder="revenue:consulting" bind:value={form.revenue_account} />
                <span class="label text-xs whitespace-normal">For invoice rows with no account of their own. Blank: the books' default.</span>
            </label>
            <label class="fieldset min-w-0">
                <span class="fieldset-legend">Name in the books</span>
                <input class="input w-full input-sm" name="ledger_customer" placeholder={form.title || "the name"} bind:value={form.ledger_customer} />
                <span class="label text-xs whitespace-normal">How the books name this customer on its transactions. Blank: the name above.</span>
            </label>
        </div>
        <fieldset class="fieldset" data-testid="contracts">
            <legend class="fieldset-legend">Contracts</legend>
            {#if form.contracts.length === 0}
                <p class="text-sm text-base-content/70">None yet.</p>
            {/if}
            <ul class="flex flex-col gap-1">
                {#each form.contracts as entry, at (`${at}:${entry}`)}
                    <li class="flex items-center gap-2 text-sm" data-testid="contract-row">
                        <span class="min-w-0 flex-1 truncate" title={nameOf(entry)}>{nameOf(entry)}</span>
                        <button
                            type="button"
                            class="btn btn-ghost btn-sm"
                            aria-label="Remove contract {nameOf(entry)}"
                            onclick={() => form.contracts.splice(at, 1)}>✕</button
                        >
                    </li>
                {/each}
            </ul>
            <div>
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (pickingContract = true)} data-testid="add-contract">Add contract…</button>
            </div>
        </fieldset>
        {#if pickingContract}
            <ContractPicker onPick={addContract} onClose={() => (pickingContract = false)} exclude={form.contracts} />
        {/if}
    </section>

    <section class="rounded-box border border-base-300 p-4" data-testid="portal">
        <label class="flex items-center gap-2 text-sm font-medium">
            <input
                type="checkbox"
                class="checkbox checkbox-sm"
                checked={form.billing_portal !== null}
                onchange={(e) => setPortal(e.currentTarget.checked)}
                data-testid="portal-toggle"
            />
            Invoices go through a vendor portal
        </label>
        {#if form.billing_portal !== null}
            <div class="mt-3 grid gap-3 sm:grid-cols-2">
                <label class="fieldset min-w-0">
                    <span class="fieldset-legend">Portal</span>
                    <input class="input w-full input-sm" name="portal_name" placeholder="Coupa" bind:value={form.billing_portal.name} />
                </label>
                <label class="fieldset min-w-0">
                    <span class="fieldset-legend">URL</span>
                    <input class="input w-full input-sm" name="portal_url" type="url" placeholder="https://" bind:value={form.billing_portal.url} />
                </label>
                <label class="fieldset min-w-0">
                    <span class="fieldset-legend">Login (the username only)</span>
                    <input class="input w-full input-sm" name="portal_login" autocomplete="off" bind:value={form.billing_portal.login} />
                    <span class="label text-xs whitespace-normal">Never a password: notes are plain text.</span>
                </label>
                <label class="fieldset min-w-0">
                    <span class="fieldset-legend">Supplier ID</span>
                    <input class="input w-full input-sm" name="portal_supplier_id" bind:value={form.billing_portal.supplier_id} />
                </label>
                <label class="flex items-center gap-2 text-sm sm:col-span-2">
                    <input
                        type="checkbox"
                        class="checkbox checkbox-sm"
                        name="portal_requires_reference"
                        bind:checked={form.billing_portal.requires_reference}
                    />
                    The portal rejects invoices without a reference (PO); sending one without is refused
                </label>
                <label class="fieldset sm:col-span-2">
                    <span class="fieldset-legend">Instructions</span>
                    <textarea class="textarea w-full textarea-sm" name="portal_instructions" rows="3" bind:value={form.billing_portal.instructions}></textarea>
                </label>
            </div>
        {/if}
    </section>
</div>
