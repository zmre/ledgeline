<!-- One account's settings (T-accounts): the customer it bills (a picker
     over the Invoicer's customers, or a new one), whether its time is
     billable, the catalog item it bills as, and whether it is archived
     (hidden from pickers; its history still reports). Each field shows what
     the account inherits when it sets nothing itself; leaving a field empty
     keeps inheriting. Save writes only what changed. -->
<script lang="ts">
    import {untrack} from "svelte";
    import Combobox from "#ui/components/Combobox.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import AddCustomerDialog from "#lib/components/AddCustomerDialog.svelte";
    import type {AccountRow} from "../api/accounts.js";
    import DialogShell from "../DialogShell.svelte";
    import {formOf, inheritedText, tagChanges, type AccountForm, type TagChange} from "./edit.js";

    let {
        row,
        customers,
        items,
        onSave,
        onCancel,
    }: {
        row: AccountRow;
        /** The Invoicer's customers: `value` is what the tag holds. */
        customers: ComboOption[];
        /** The catalog's items by code. */
        items: ComboOption[];
        /** Write `changes`; null when done, else why not. */
        onSave: (changes: TagChange[]) => Promise<string | null>;
        onCancel: () => void;
    } = $props();

    const start = untrack(() => formOf(row));
    let form = $state<AccountForm>({...start});
    let busy = $state(false);
    let error = $state<string | null>(null);
    let creating = $state<string | null>(null);

    const changes = $derived(tagChanges(row, form));
    const inheritedBillable = $derived(row.billable !== null && row.billable.inherited ? row.billable : null);
    const archivedAbove = $derived(row.archived !== null && row.archived.inherited ? row.archived.from : null);

    async function save(): Promise<void> {
        if (changes.length === 0) {
            onCancel();
            return;
        }
        busy = true;
        error = await onSave(changes);
        busy = false;
    }

    const placeholder = (inherited: string, value: string | undefined): string => (inherited === "" ? "None" : `${value} (${inherited})`);
</script>

<DialogShell
    title={row.account}
    submitLabel="Save"
    cancelLabel="Discard"
    {busy}
    {error}
    submitDisabled={changes.length === 0}
    onSubmit={() => void save()}
    {onCancel}
    testid="account-dialog"
    initial="#account-dialog-customer"
>
    <div class="grid gap-3 sm:grid-cols-2">
        <div class="flex flex-col gap-1 text-sm">
            <span aria-hidden="true">Customer</span>
            <Combobox
                bind:value={form.customer}
                options={customers}
                label="Customer"
                placeholder={placeholder(inheritedText(row.customer), row.customer?.value)}
                addNewLabel={(text) => `New customer “${text}”…`}
                onAddNew={(text) => (creating = text)}
                testid="account-dialog-customer"
                id="account-dialog-customer"
            />
            <span class="text-xs text-base-content/60"
                >Empty: {row.customer !== null && row.customer.inherited ? `use ${row.customer.from}'s` : "no customer"}.</span
            >
        </div>
        <label class="flex flex-col gap-1 text-sm"
            >Billable
            <select class="select w-full select-sm" bind:value={form.billable} data-testid="account-dialog-billable">
                <option value="">{inheritedBillable === null ? "Not set (not billable)" : `As ${inheritedBillable.from} (${inheritedBillable.value})`}</option>
                <option value="yes">Yes</option>
                <option value="no">No</option>
            </select>
        </label>
        <div class="flex flex-col gap-1 text-sm">
            <span aria-hidden="true">Default item</span>
            <Combobox
                bind:value={form.item}
                options={items}
                label="Default item"
                placeholder={placeholder(inheritedText(row.item), row.item?.value)}
                testid="account-dialog-item"
            />
            <span class="text-xs text-base-content/60">The catalog row its time is billed as.</span>
        </div>
        <label class="flex items-center gap-2 self-center text-sm">
            <input type="checkbox" class="checkbox checkbox-sm" bind:checked={form.archived} data-testid="account-dialog-archived" />
            Archived (hidden from pickers)
        </label>
        {#if archivedAbove !== null}
            <p class="text-xs text-base-content/60 sm:col-span-2">Archived through {archivedAbove}.</p>
        {/if}
    </div>
</DialogShell>

{#if creating !== null}
    <AddCustomerDialog
        name={creating}
        onCreated={(saved) => {
            form.customer = saved.ledger_customer === "" ? saved.title : saved.ledger_customer;
            creating = null;
        }}
        onCancel={() => (creating = null)}
    />
{/if}
