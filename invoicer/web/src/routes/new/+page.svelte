<!-- New invoice (invoicer PLAN 4a; File → New Invoice loads /app/new, A9): a
     customer, a template and a date, then the CLI's `new` writes the draft
     (the customer's and your company's contact snapshot, terms and due date,
     default snippets, a bill table with its total row) and the editor opens
     on it. Drafts never touch the books (I2).

     The customer field has focus on arrival (`n` brings you here, PLAN 5k):
     type, Enter to choose, Tab on, Enter on Create draft.

     New deposit invoice (`d`, the New menu; `?deposit=1`, ARCHITECTURE §4.4)
     is the same draft with one row booked to the deposit account, so the
     editor opens on its deposit fields (Deposit for note) to fill in.

     Anything that went wrong once the draft was written (its starter
     template not copied in) goes to the editor with it (page state), which
     shows it: the draft exists either way. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import Combobox from "#ui/components/Combobox.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import DateInput from "#ui/components/DateInput.svelte";
    import {fetchTemplates} from "#lib/api.js";
    import type {EditorOptions} from "#lib/editor.js";
    import {createInvoice, fetchEditorOptions} from "#lib/editorApi.js";
    import {localToday, nextNumberReason, type TemplateEntry} from "#lib/invoices.js";
    import {errorText} from "#lib/labels.js";
    import {linkName, toLink} from "#lib/links.js";
    import AddCustomerDialog from "#lib/components/AddCustomerDialog.svelte";

    let options = $state<EditorOptions | null>(null);
    let templates = $state<TemplateEntry[]>([]);
    let customer = $state("");
    let template = $state("");
    let templateTouched = $state(false);
    let date = $state(localToday());
    let busy = $state(false);
    let error = $state<string | null>(null);
    let adding = $state<string | null>(null);

    const deposit = $derived(page.url.searchParams.get("deposit") === "1");
    const heading = $derived(deposit ? "New deposit invoice" : "New invoice");

    function loadOptions(): void {
        fetchEditorOptions()
            .then((o) => (options = o))
            .catch((e: unknown) => (error = errorText(e)));
    }

    let formEl = $state<HTMLFormElement | null>(null);

    onMount(() => {
        formEl?.querySelector<HTMLInputElement>('[data-testid="new-customer"]')?.focus();
        loadOptions();
        fetchTemplates()
            .then((t) => (templates = t.filter((entry) => entry.kinds.includes("invoice"))))
            .catch((e: unknown) => console.warn("templates:", e));
    });

    // The field shows names; the file gets `[[Name]]` (Phase 4 feedback H2).
    const customerOptions = $derived<ComboOption[]>((options?.customers ?? []).map((c) => ({value: c.title, label: c.title, detail: c.kind})));
    const chosen = $derived((options?.customers ?? []).find((c) => c.title.toLowerCase() === customer.trim().toLowerCase()));

    // The customer's own template is the default, until one is picked by hand.
    // A customer without one goes back to the default (not the last customer's).
    $effect(() => {
        const own = chosen?.invoice_template ?? null;
        if (!templateTouched) template = own ?? "";
    });

    async function submit(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        if (customer.trim() === "") return;
        busy = true;
        error = null;
        try {
            const link = chosen?.link ?? toLink(customer, [], true);
            const made = await createInvoice({customer: link, template: template === "" ? null : template, date, deposit});
            await goto(resolve("/invoices/[number]/edit", {number: made.number}), {state: {createWarnings: made.warnings}});
        } catch (e) {
            error = errorText(e);
        } finally {
            busy = false;
        }
    }
</script>

<svelte:head>
    <title>{heading} · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-xl flex-col gap-4">
    <h1 class="text-2xl font-semibold tracking-tight" data-testid="new-heading">{heading}</h1>
    <form class="flex flex-col gap-3 rounded-box border border-base-300 p-4" onsubmit={submit} bind:this={formEl} data-testid="new-invoice">
        <label class="flex flex-col gap-1 text-sm"
            >Customer
            <Combobox
                bind:value={customer}
                options={customerOptions}
                label="Customer"
                placeholder="Who is it for?"
                addNewLabel={(name) => `Add new customer “${name}”…`}
                onAddNew={(name) => (adding = name)}
                testid="new-customer"
            />
        </label>
        <label class="flex flex-col gap-1 text-sm"
            >Template
            <select class="select select-sm" bind:value={template} onchange={() => (templateTouched = true)} data-testid="new-template">
                <option value="">The books' default</option>
                {#each templates as entry (entry.path)}
                    <option value={entry.path}>{entry.name}{entry.starter && !entry.installed ? " (starter)" : ""}</option>
                {/each}
            </select>
        </label>
        <label class="flex flex-col gap-1 text-sm">Date <DateInput bind:value={date} aria-label="Date" required /></label>
        {#if options !== null}
            {@const reason = nextNumberReason(options.next_number_after, options.next_number_source)}
            <!-- Where the number comes from (H19): one booked in the journal with no file is never reused. -->
            <p class="text-xs text-base-content/60" data-testid="new-number">
                It will be number <span class="font-medium text-base-content">{options.next_number}</span>{#if reason !== null}<span
                        data-testid="new-number-reason">{` (${reason})`}</span
                    >{/if}, a draft: nothing is booked until it is sent.
            </p>
        {/if}
        {#if error !== null}<div class="alert text-sm alert-error" role="alert" data-testid="new-error">{error}</div>{/if}
        <div class="flex justify-end gap-2">
            <a class="btn btn-ghost btn-sm" href={resolve("/invoices")}>Cancel</a>
            <button type="submit" class="btn btn-primary btn-sm" disabled={busy || customer.trim() === "" || date === ""} data-testid="new-create"
                >Create draft</button
            >
        </div>
    </form>
</div>

{#if adding !== null}
    <AddCustomerDialog
        name={adding}
        onCancel={() => (adding = null)}
        onCreated={(made) => {
            adding = null;
            customer = linkName(made.link);
            loadOptions();
        }}
    />
{/if}
