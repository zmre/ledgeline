<!-- A new customer (invoicer PLAN 4b): the form, then `POST /api/customers`,
     which writes a person or organization note in `customers-dir`, tagged
     `customer`; then its page. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {fetchTemplates} from "#lib/api.js";
    import {createCustomer, emptyForm, newCustomerFields, routeParam} from "#lib/customers.js";
    import type {TemplateEntry} from "#lib/invoices.js";
    import CustomerForm from "#lib/components/CustomerForm.svelte";

    let form = $state(emptyForm());
    let templates = $state<TemplateEntry[]>([]);
    let saving = $state(false);
    let failure = $state<string | null>(null);

    onMount(() => {
        fetchTemplates()
            .then((list) => (templates = list))
            .catch((e: unknown) => console.warn("templates:", e));
    });

    async function create(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        saving = true;
        failure = null;
        try {
            const made = await createCustomer(newCustomerFields(form));
            await goto(resolve("/customers/[...path]", {path: routeParam(made.path)}));
        } catch (e: unknown) {
            failure = e instanceof Error ? e.message : String(e);
        } finally {
            saving = false;
        }
    }
</script>

<svelte:head>
    <title>New customer · Ledgeline Invoicer</title>
</svelte:head>

<form class="mx-auto flex max-w-3xl flex-col gap-4" onsubmit={create} data-testid="new-customer-form">
    <div class="flex items-center justify-between gap-2">
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
    <CustomerForm bind:form {templates} creating />
</form>
