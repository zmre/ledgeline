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
     running under the next field).

     Open POs (PLAN 5b, S5): a row each (reference, amount, expiry, and a
     file with its details, attached as a contract is); Close takes one off
     the list, Add PO adds a blank row. Saved with the form.

     The revenue account suggests the books' accounts, revenue first (by
     declared type). Notes: the note's markdown body, in MBR's markdown
     field (`ProseField`). -->
<script lang="ts">
    import AccountInput from "#ui/components/AccountInput.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import {contractName, emptyPortal, isOutsideEntry, type CustomerForm, type FileCandidate} from "#lib/customers.js";
    import type {TemplateEntry} from "#lib/invoices.js";
    import ContractPicker from "./ContractPicker.svelte";
    import LabelledList from "./LabelledList.svelte";
    import ProseField from "./ProseField.svelte";

    let {
        form = $bindable(),
        templates = [],
        creating = false,
        companies = [],
        contractNames = {},
        accounts = [],
        revenueAccounts = [],
        linkBase = "/",
    }: {
        form: CustomerForm;
        /** The invoice templates, for the default-template picker. */
        templates?: TemplateEntry[];
        /** A new customer: the name is the file's name too. */
        creating?: boolean;
        /** Organization names a person's company may be (suggestions). */
        companies?: string[];
        /** What to call each contract or PO attachment entry (the server's names); else from the entry. */
        contractNames?: Record<string, string>;
        /** The books' accounts, for the revenue account's suggestions. */
        accounts?: string[];
        /** Those that are revenue (by declared type): suggested first. */
        revenueAccounts?: string[];
        /** The note's folder URL, so links in its notes resolve as on its page. */
        linkBase?: string;
    } = $props();

    const uid = $props.id();
    /** What the picker is open for: the contracts, or one PO's attachment (its row). */
    let picking = $state<{for: "contract"} | {for: "po"; at: number} | null>(null);
    /** Names of entries added here, before the server has named them. */
    const added = $state<Record<string, string>>({});
    const nameOf = (entry: string): string => contractNames[entry] ?? added[entry] ?? contractName(entry);

    function onPicked(file: FileCandidate): void {
        added[file.entry] = file.name;
        const target = picking;
        picking = null;
        if (target === null) return;
        if (target.for === "contract") {
            if (!form.contracts.includes(file.entry)) form.contracts.push(file.entry);
        } else {
            const po = (form.open_pos ?? [])[target.at];
            if (po !== undefined) po.attachment = file.entry;
        }
    }

    const pickerTitle = $derived.by((): string => {
        if (picking?.for !== "po") return "Add a contract";
        const reference = (form.open_pos ?? [])[picking.at]?.reference.trim() ?? "";
        return reference === "" ? "Attach a file to this PO" : `Attach a file to ${reference}`;
    });

    const ADDRESS_PLACEHOLDER = "1 Main St\nSpringfield, CO 80000";

    const invoiceTemplates = $derived(templates.filter((t) => t.kinds.includes("invoice")));
    const knownTemplate = $derived(form.invoice_template === "" || invoiceTemplates.some((t) => t.path === form.invoice_template));

    function setPortal(on: boolean): void {
        form.billing_portal = on ? (form.billing_portal ?? emptyPortal()) : null;
    }

    function addPo(): void {
        form.open_pos = [...(form.open_pos ?? []), {reference: "", amount: null, expires: null, attachment: null, was: null}];
    }

    function closePo(at: number): void {
        form.open_pos = (form.open_pos ?? []).filter((_, i) => i !== at);
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
            <div class="fieldset min-w-0" data-testid="customer-revenue-account">
                <span class="fieldset-legend" aria-hidden="true">Revenue account</span>
                <AccountInput
                    bind:value={form.revenue_account}
                    accountNames={accounts}
                    preferred={revenueAccounts}
                    label="Revenue account"
                    placeholder="revenue:consulting"
                />
                <span class="label text-xs whitespace-normal">For invoice rows with no account of their own. Blank: the books' default.</span>
            </div>
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
                        {#if isOutsideEntry(entry)}<span class="badge shrink-0 badge-ghost badge-sm">outside the books</span>{/if}
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
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (picking = {for: "contract"})} data-testid="add-contract"
                    >Add contract…</button
                >
            </div>
        </fieldset>
    </section>
    {#if picking !== null}
        <ContractPicker onPick={onPicked} onClose={() => (picking = null)} title={pickerTitle} exclude={picking.for === "contract" ? form.contracts : []} />
    {/if}

    <section class="rounded-box border border-base-300 p-4" data-testid="form-open-pos" aria-labelledby="{uid}-pos">
        <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
            <h2 id="{uid}-pos" class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Open POs</h2>
            <button type="button" class="btn btn-outline btn-xs" onclick={addPo} data-testid="form-po-add">Add PO</button>
        </div>
        {#if (form.open_pos ?? []).length === 0}
            <p class="text-sm text-base-content/60">None. Invoices for this customer suggest its open POs as their reference.</p>
        {:else}
            <p class="text-sm text-base-content/60">A PO closed here is noted under “Closed POs” in the customer's note when you save.</p>
        {/if}
        <ul class="flex flex-col gap-3">
            {#each form.open_pos ?? [] as po, at (at)}
                {@const poName = po.reference.trim() === "" ? "this PO" : po.reference.trim()}
                <li class="grid items-end gap-2 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)_auto]" data-testid="form-po-row">
                    <label class="flex min-w-0 flex-col gap-1 text-sm"
                        >Reference
                        <input class="input w-full input-sm" bind:value={po.reference} placeholder="PO #105" data-testid="form-po-reference" />
                    </label>
                    <label class="flex min-w-0 flex-col gap-1 text-sm"
                        >Amount
                        <input
                            class="input w-full input-sm"
                            bind:value={() => po.amount ?? "", (v) => (po.amount = v)}
                            placeholder="$40,000"
                            data-testid="form-po-amount"
                        />
                    </label>
                    <label class="flex min-w-0 flex-col gap-1 text-sm"
                        >Expires
                        <DateInput
                            class="w-full"
                            value={po.expires ?? ""}
                            aria-label="Expires"
                            onchange={(event) => (po.expires = event.currentTarget.value === "" ? null : event.currentTarget.value)}
                            data-testid="form-po-expires"
                        />
                    </label>
                    <button
                        type="button"
                        class="btn btn-ghost btn-sm"
                        onclick={() => closePo(at)}
                        aria-label="Close {po.reference.trim() === '' ? 'this PO' : po.reference}"
                        data-testid="form-po-close">Close</button
                    >
                    <div class="flex min-w-0 flex-wrap items-center gap-2 text-sm sm:col-span-4" data-testid="form-po-attachment">
                        {#if po.attachment != null && po.attachment.trim() !== ""}
                            {@const entry = po.attachment}
                            <span class="text-base-content/60">Attachment:</span>
                            <span class="min-w-0 truncate" title={nameOf(entry)} data-testid="form-po-attachment-name">{nameOf(entry)}</span>
                            {#if isOutsideEntry(entry)}<span class="badge shrink-0 badge-ghost badge-sm">outside the books</span>{/if}
                            <button
                                type="button"
                                class="btn btn-ghost btn-xs"
                                aria-label="Remove the attachment from {poName}"
                                onclick={() => (po.attachment = null)}
                                data-testid="form-po-detach">✕</button
                            >
                        {:else}
                            <button
                                type="button"
                                class="btn btn-ghost btn-xs"
                                aria-label="Attach a file to {poName}"
                                onclick={() => (picking = {for: "po", at})}
                                data-testid="form-po-attach">Attach…</button
                            >
                        {/if}
                    </div>
                </li>
            {/each}
        </ul>
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

    <section class="rounded-box border border-base-300 p-4" data-testid="form-notes">
        <h2 class="mb-2 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Notes</h2>
        <ProseField
            value={form.body ?? ""}
            label="Notes"
            placeholder="Who to talk to, how they like to be billed, anything worth remembering"
            {linkBase}
            onChange={(value) => (form.body = value)}
            testid="customer-notes"
        />
    </section>
</div>
