<!-- The invoice's header (ARCHITECTURE §3.2, §6.3): customer and template,
     number, dates, terms, reference, deposit-for, and custom fields. It edits
     the form only; the server writes the frontmatter by key and re-snapshots
     the customer's contact when the customer changes.

     The due date follows date + terms until it is typed by hand (a due date
     that isn't date + terms is the user's and stays). -->
<script lang="ts">
    import Combobox from "#ui/components/Combobox.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import DateInput from "#ui/components/DateInput.svelte";
    import {addDays, type CustomField, type EditorForm, type EditorOptions} from "#lib/editor.js";
    import type {TemplateEntry} from "#lib/invoices.js";

    let {
        form,
        options,
        templates,
        status,
        onChange,
        onAddCustomer,
        fieldValues,
    }: {
        form: EditorForm;
        options: EditorOptions | null;
        templates: TemplateEntry[];
        status: string;
        onChange: (form: EditorForm) => void;
        /** Inline "Add new customer": the name typed. */
        onAddCustomer: (name: string) => void;
        /** A custom field's known values (most used first). */
        fieldValues: (key: string) => Promise<{value: string; count: number}[]>;
    } = $props();

    const draft = $derived(status === "draft");
    const locked = $derived(status === "void");

    const customerOptions = $derived<ComboOption[]>((options?.customers ?? []).map((c) => ({value: c.link, label: c.title, detail: c.kind})));
    const templateOptions = $derived<ComboOption[]>(
        templates.map((t) => ({value: t.path, label: t.name, detail: t.starter && !t.installed ? "starter, copied in on save" : t.path}))
    );
    const customer = $derived((options?.customers ?? []).find((c) => c.link === form.customer || c.title === form.customer.replace(/^\[\[|\]\]$/g, "")));
    const referenceOptions = $derived<ComboOption[]>(
        (customer?.open_pos ?? []).map((po) => ({
            value: po.reference,
            label: po.reference,
            detail: [po.amount, po.expires ? `expires ${po.expires}` : null].filter(Boolean).join(" · "),
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

    function chooseCustomer(link: string): void {
        const chosen = (options?.customers ?? []).find((c) => c.link === link);
        const next: Partial<EditorForm> = {customer: link};
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

<section class="grid gap-3 rounded-box border border-base-300 p-3 sm:grid-cols-2 lg:grid-cols-3" aria-label="Invoice details" data-testid="header-form">
    <label class="flex flex-col gap-1 text-sm sm:col-span-2"
        >Customer
        <Combobox
            bind:value={() => form.customer, (v) => patch({customer: v})}
            options={customerOptions}
            label="Customer"
            placeholder="[[Customer]]"
            disabled={locked}
            onChoose={(option) => chooseCustomer(option.value)}
            addNewLabel={(name) => `Add new customer “${name}”…`}
            onAddNew={onAddCustomer}
            testid="header-customer"
        />
    </label>
    <label class="flex flex-col gap-1 text-sm"
        >Template
        <Combobox
            bind:value={() => form.template ?? "", (v) => patch({template: text(v)})}
            options={templateOptions}
            label="Template"
            placeholder="/invoices/tpl/…"
            disabled={locked}
            testid="header-template"
        />
    </label>
    <label class="flex flex-col gap-1 text-sm"
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
    <label class="flex flex-col gap-1 text-sm"
        >Date
        <DateInput value={form.date} disabled={locked} aria-label="Date" oninput={(event) => patch({date: event.currentTarget.value})} />
    </label>
    <label class="flex flex-col gap-1 text-sm"
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
    <label class="flex flex-col gap-1 text-sm"
        >Due date
        <DateInput value={form.due_date} disabled={locked} aria-label="Due date" oninput={(event) => patch({due_date: text(event.currentTarget.value)})} />
    </label>
    <label class="flex flex-col gap-1 text-sm"
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
    <label class="flex flex-col gap-1 text-sm"
        >Deposit for
        <input
            class="input input-sm"
            value={form.deposit_for ?? ""}
            disabled={locked}
            placeholder="on a deposit invoice: what it is a deposit on"
            oninput={(event) => patch({deposit_for: text(event.currentTarget.value)})}
        />
    </label>

    <fieldset class="flex flex-col gap-2 sm:col-span-2 lg:col-span-3" data-testid="custom-fields">
        <legend class="text-sm font-medium">Custom fields</legend>
        {#each form.fields as field, at (field.key)}
            <div class="flex items-center gap-2" data-testid="custom-field">
                <span class="w-40 truncate font-mono text-xs" title={field.key}>{field.key}</span>
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
                    <code class="flex-1 truncate text-xs text-base-content/70" title="A list or map: edit it in the file">{JSON.stringify(field.value)}</code>
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
</section>
