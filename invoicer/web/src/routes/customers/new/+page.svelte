<!-- A new customer (invoicer PLAN 4b): the form, then `POST /api/customers`,
     which writes a person or organization note in `customers-dir`, tagged
     `customer`; then its page. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {fetchTemplates} from "#lib/api.js";
    import {fetchEditorOptions} from "#lib/editorApi.js";
    import {createCustomer, emptyForm, fetchCustomers, newCustomerFields, routeParam} from "#lib/customers.js";
    import {errorText} from "#lib/labels.js";
    import {pageUrl, type TemplateEntry} from "#lib/invoices.js";
    import CustomerForm from "#lib/components/CustomerForm.svelte";

    let form = $state(emptyForm());
    let templates = $state<TemplateEntry[]>([]);
    /** Organizations a new person may work for (suggestions for Company). */
    let companies = $state<string[]>([]);
    /** The books' accounts, and those that are revenue, for the revenue account's suggestions. */
    let accounts = $state<string[]>([]);
    let revenueAccounts = $state<string[]>([]);
    /** Where the note will be, so links in its notes resolve as on its page. */
    let linkBase = $state("/");
    let saving = $state(false);
    let failure = $state<string | null>(null);

    onMount(() => {
        fetchTemplates()
            .then((list) => (templates = list))
            .catch((e: unknown) => console.warn("templates:", e));
        fetchCustomers()
            .then((rows) => (companies = rows.filter((r) => r.kind === "organization").map((r) => r.title)))
            .catch((e: unknown) => console.warn("customers:", e));
        fetchEditorOptions()
            .then((o) => {
                accounts = o.accounts;
                revenueAccounts = o.revenue_accounts ?? [];
                linkBase = pageUrl(`${o.customers_dir.replace(/^\/+|\/+$/g, "")}/new.md`).replace(/[^/]+\/$/, "");
            })
            .catch((e: unknown) => console.warn("accounts:", e));
    });

    async function create(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        saving = true;
        failure = null;
        try {
            const made = await createCustomer(newCustomerFields(form));
            await goto(resolve("/customers/[...path]", {path: routeParam(made.path)}));
        } catch (e: unknown) {
            failure = errorText(e);
        } finally {
            saving = false;
        }
    }
</script>

<svelte:head>
    <title>New customer · Ledgeline Invoicer</title>
</svelte:head>

<form class="mx-auto flex max-w-3xl flex-col gap-4" onsubmit={create} data-testid="new-customer-form">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">New customer</h1>
        <div class="flex gap-2">
            <a class="btn btn-ghost btn-sm" href={resolve("/customers")}>Cancel</a>
            <button type="submit" class="btn btn-primary btn-sm" disabled={saving || form.title.trim() === ""} data-testid="create-customer">
                {saving ? "Creating…" : "Create customer"}
            </button>
        </div>
    </div>
    {#if failure !== null}
        <div role="alert" class="alert alert-soft alert-error" data-testid="customer-error">{failure}</div>
    {/if}
    <CustomerForm bind:form {templates} {companies} {accounts} {revenueAccounts} {linkBase} creating />
</form>
