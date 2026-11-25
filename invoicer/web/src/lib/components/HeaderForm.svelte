<!-- The invoice's header (ARCHITECTURE §3.2, §6.3): customer and template,
     number, dates, terms, reference, Deposit for note, and custom fields. It
     edits the form only; the server writes the frontmatter by key and
     re-snapshots the customer's contact when the customer changes.

     Links are names here (Phase 4 feedback H2): the customer and Deposit for
     note show a note's name and write `[[Name]]`. The template is a list of
     the books' templates and the starters, by name (H4). Deposit for note
     shows on a deposit invoice, or where it is set (H3).

     The due date follows date + terms until it is typed by hand (a due date
     that isn't date + terms is the user's and stays). -->
<script lang="ts">
    import {untrack} from "svelte";
    import Combobox from "#ui/components/Combobox.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import DateInput from "#ui/components/DateInput.svelte";
    import {addDays, type CustomField, type EditorForm, type EditorOptions} from "#lib/editor.js";
    import type {TemplateEntry} from "#lib/invoices.js";
    import {fieldLabel} from "#lib/labels.js";
    import {linkName, toLink} from "#lib/links.js";

    let {
        form,
        options,
        templates,
        status,
        deposit = false,
        onChange,
        onAddCustomer,
        fieldValues,
    }: {
        form: EditorForm;
        options: EditorOptions | null;
        templates: TemplateEntry[];
        status: string;
        /** A deposit invoice (a row books to the deposit account): Deposit for note is offered. */
        deposit?: boolean;
        onChange: (form: EditorForm) => void;
        /** Inline "Add new customer": the name typed. */
        onAddCustomer: (name: string) => void;
        /** A custom field's known values (most used first). */
        fieldValues: (key: string) => Promise<{value: string; count: number}[]>;
    } = $props();

    const draft = $derived(status === "draft");
    const locked = $derived(status === "void");

    const customerOptions = $derived<ComboOption[]>((options?.customers ?? []).map((c) => ({value: c.title, label: c.title, detail: c.kind})));
    const customerTitles = $derived((options?.customers ?? []).map((c) => c.title));
    /** The invoice templates, by name; the invoice's own first when it names one the books don't have. */
    const templateChoices = $derived(templates.filter((t) => t.kinds.length === 0 || t.kinds.includes("invoice")));
    const unknownTemplate = $derived(form.template !== null && !templateChoices.some((t) => t.path === form.template));
    const customer = $derived((options?.customers ?? []).find((c) => c.link === form.customer || c.title === linkName(form.customer)));
    const linkNotes = $derived(options?.link_notes ?? []);
    const depositOptions = $derived<ComboOption[]>(linkNotes.map((n) => ({value: n.title, label: n.title, detail: n.kind ?? undefined})));
    const showDepositFor = $derived(deposit || (form.deposit_for ?? "") !== "");
    const noteTitles = $derived(linkNotes.map((n) => n.title));

    // The link fields' text as typed: the form holds `[[Name]]`, so a space
    // typed at the end isn't trimmed away mid-word. The text follows the form
    // when it changes from outside (a reload, Add new customer).
    let customerText = $state("");
    let depositText = $state("");
    $effect(() => {
        const name = linkName(form.customer);
        if (untrack(() => customerText.trim()) !== name) customerText = name;
    });
    $effect(() => {
        const name = linkName(form.deposit_for);
        if (untrack(() => depositText.trim()) !== name) depositText = name;
    });

    function typeCustomer(v: string): void {
        customerText = v;
        patch({customer: toLink(v, customerTitles, true)});
    }

    function typeDepositFor(v: string): void {
        depositText = v;
        patch({deposit_for: text(toLink(v, noteTitles))});
    }
    // The customer's open POs (managed on its page, PLAN 5b S5; the options
    // are fetched again on any change on disk). One expired by the invoice's
    // date is still offered, last and marked.
    const referenceOptions = $derived<ComboOption[]>(
        (customer?.open_pos ?? [])
            .map((po) => ({po, expired: po.expires !== null && po.expires !== "" && po.expires < form.date}))
            .sort((a, b) => Number(a.expired) - Number(b.expired))
            .map(({po, expired}) => ({
                value: po.reference,
                label: po.reference,
                detail: [po.amount, po.expires ? `${expired ? "expired" : "expires"} ${po.expires}` : null].filter(Boolean).join(" · "),
            }))
    );

    /** `form` with `patch`, the due date following date + terms unless it was set by hand. */
    function patch(next: Partial<EditorForm>): void {
        const merged = {...form, ...next};
        const auto = form.due_date === null || (form.terms !== null && form.due_date === addDays(form.date, form.terms));
        if (auto && ("date" in next || "terms" in next) && !("due_date" in next)) {
            merged.due_date = merged.terms === null ? null : addDays(merged.date, merged.terms);
        }
        onChange(merged);
    }

    const text = (value: string): string | null => (value.trim() === "" ? null : value);

    function chooseCustomer(title: string): void {
        const chosen = (options?.customers ?? []).find((c) => c.title === title);
        const next: Partial<EditorForm> = {customer: chosen?.link ?? toLink(title, customerTitles, true)};
        if (chosen?.terms != null) next.terms = chosen.terms;
        if (form.template === null && chosen?.invoice_template) next.template = chosen.invoice_template;
        patch(next);
    }

    // Custom fields.
    const editable = (value: unknown): boolean => typeof value === "string" || typeof value === "number" || typeof value === "boolean" || value === null;
    const shown = (value: unknown): string => (value === null || value === undefined ? "" : typeof value === "string" ? value : String(value));

    function setField(at: number, field: CustomField): void {
        const fields = form.fields.map((f, i) => (i === at ? field : f));
        onChange({...form, fields});
    }

    function fieldText(at: number, raw: string): void {
        const before = form.fields[at];
        const asNumber = typeof before.value === "number" && raw.trim() !== "" && Number.isFinite(Number(raw)) ? Number(raw) : raw;
        setField(at, {key: before.key, value: asNumber});
    }

    let newKey = $state("");
    let newValue = $state("");
    let known = $state<Record<string, ComboOption[]>>({});
    const keyOptions = $derived<ComboOption[]>((options?.fields ?? []).filter((k) => !form.fields.some((f) => f.key === k)).map((k) => ({value: k, label: k})));
    const validKey = $derived(/^[A-Za-z_][A-Za-z0-9_-]*$/.test(newKey.trim()) && !form.fields.some((f) => f.key === newKey.trim()));

    const valuesFor = (key: string): ComboOption[] => known[key] ?? [];

    // Each field's known values, asked for once per key (`/api/fields`).
    // Bookkeeping only (which keys were asked for), never shown: not reactive on purpose.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const requested = new Set<string>();
    $effect(() => {
        // A half-typed new key isn't asked for: only keys the books already use.
        const typed = newKey.trim();
        const keys = [...form.fields.map((f) => f.key), ...((options?.fields ?? []).includes(typed) ? [typed] : [])];
        for (const key of keys) {
            if (requested.has(key)) continue;
            requested.add(key);
            fieldValues(key)
                .then((values) => (known[key] = values.map((v) => ({value: v.value, label: v.value, detail: `${v.count}×`}))))
                .catch(() => requested.delete(key));
        }
    });

    function addField(): void {
        if (!validKey) return;
        onChange({...form, fields: [...form.fields, {key: newKey.trim(), value: newValue}]});
        newKey = "";
        newValue = "";
    }
</script>

<section class="@container rounded-box border border-base-300 p-3" aria-label="Invoice details" data-testid="header-form">
    <div class="grid gap-3 @lg:grid-cols-2 @3xl:grid-cols-3">
        <label class="flex min-w-0 flex-col gap-1 text-sm @lg:col-span-2"
            >Customer
            <Combobox
                bind:value={() => customerText, typeCustomer}
                options={customerOptions}
                label="Customer"
                placeholder="Who is it for?"
                disabled={locked}
                onChoose={(option) => chooseCustomer(option.value)}
                addNewLabel={(name) => `Add new customer “${name}”…`}
                onAddNew={onAddCustomer}
                testid="header-customer"
            />
        </label>
        <label class="flex min-w-0 flex-col gap-1 text-sm"
            >Template
            <select
                class="select w-full select-sm"
                value={form.template ?? ""}
                disabled={locked}
                onchange={(event) => patch({template: text(event.currentTarget.value)})}
                data-testid="header-template"
            >
                <option value="">The books' default</option>
                {#if unknownTemplate}<option value={form.template}>{form.template} (not found)</option>{/if}
                {#each templateChoices as entry (entry.path)}
                    <option value={entry.path}>{entry.name}{entry.starter && !entry.installed ? " (starter)" : ""}</option>
                {/each}
            </select>
        </label>
        <label class="flex min-w-0 flex-col gap-1 text-sm"
            >Number
            <input
                class="input input-sm"
                value={form.number}
                disabled={!draft}
                title={draft ? undefined : "A sent invoice keeps its number: the books find it by number"}
                oninput={(event) => patch({number: event.currentTarget.value})}
                data-testid="header-number"
            />
        </label>
        <label class="flex min-w-0 flex-col gap-1 text-sm"
            >Date
            <DateInput value={form.date} disabled={locked} aria-label="Date" oninput={(event) => patch({date: event.currentTarget.value})} />
        </label>
        <label class="flex min-w-0 flex-col gap-1 text-sm"
            >Terms (net days)
            <input
                class="input input-sm"
                type="number"
                min="0"
                step="1"
                value={form.terms ?? ""}
                disabled={locked}
                oninput={(event) => {
                    const raw = event.currentTarget.value;
                    patch({terms: raw === "" ? null : Math.max(0, Math.trunc(Number(raw)))});
                }}
                data-testid="header-terms"
            />
        </label>
        <label class="flex min-w-0 flex-col gap-1 text-sm"
            >Due date
            <DateInput value={form.due_date} disabled={locked} aria-label="Due date" oninput={(event) => patch({due_date: text(event.currentTarget.value)})} />
        </label>
        <label class="flex min-w-0 flex-col gap-1 text-sm"
            >Reference
            <Combobox
                bind:value={() => form.reference ?? "", (v) => patch({reference: text(v)})}
                options={referenceOptions}
                label="Reference"
                placeholder={referenceOptions.length > 0 ? "an open PO, or any text" : "PO #…"}
                disabled={locked}
                testid="header-reference"
            />
        </label>
        {#if showDepositFor}
            <label class="flex min-w-0 flex-col gap-1 text-sm"
                >{fieldLabel("deposit_for")}
                <Combobox
                    bind:value={() => depositText, typeDepositFor}
                    options={depositOptions}
                    label={fieldLabel("deposit_for")}
                    placeholder="An estimate, or a few words"
                    disabled={locked}
                    testid="header-deposit-for"
                />
                <span class="text-xs text-base-content/60">What this deposit is toward: pick an estimate or note to link it, or type a description.</span>
            </label>
        {/if}

        <fieldset class="flex min-w-0 flex-col gap-2 @lg:col-span-2 @3xl:col-span-3" data-testid="custom-fields">
            <legend class="text-sm font-medium">Custom fields</legend>
            {#each form.fields as field, at (field.key)}
                <div class="flex items-center gap-2" data-testid="custom-field">
                    <span class="w-40 shrink-0 truncate text-xs" title={field.key}>{fieldLabel(field.key)}</span>
                    {#if typeof field.value === "boolean"}
                        <input
                            type="checkbox"
                            class="checkbox checkbox-sm"
                            aria-label={field.key}
                            checked={field.value}
                            disabled={locked}
                            onchange={(event) => setField(at, {key: field.key, value: event.currentTarget.checked})}
                        />
                    {:else if editable(field.value)}
                        <div class="flex-1">
                            <Combobox
                                bind:value={() => shown(field.value), (v) => fieldText(at, v)}
                                options={valuesFor(field.key)}
                                label={field.key}
                                size="xs"
                                disabled={locked}
                            />
                        </div>
                    {:else}
                        <code class="flex-1 truncate text-xs text-base-content/70" title="A list or map: edit it in the file"
                            >{JSON.stringify(field.value)}</code
                        >
                    {/if}
                    {#if !locked}
                        <button
                            type="button"
                            class="btn btn-square btn-ghost btn-xs"
                            aria-label="Remove {field.key}"
                            onclick={() => onChange({...form, fields: form.fields.filter((_, i) => i !== at)})}>✕</button
                        >
                    {/if}
                </div>
            {/each}
            {#if !locked}
                <div class="flex flex-wrap items-center gap-2">
                    <div class="w-40"><Combobox bind:value={newKey} options={keyOptions} label="New field name" placeholder="field_name" size="xs" /></div>
                    <div class="flex-1">
                        <Combobox
                            bind:value={newValue}
                            options={valuesFor(newKey.trim())}
                            label="New field value"
                            placeholder="value"
                            size="xs"
                            onCommit={() => undefined}
                        />
                    </div>
                    <button type="button" class="btn btn-outline btn-xs" disabled={!validKey} onclick={addField} data-testid="add-field">Add field</button>
                </div>
            {/if}
        </fieldset>
    </div>
</section>
