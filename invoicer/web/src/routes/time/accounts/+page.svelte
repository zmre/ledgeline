<!-- Time → Accounts (plan 46 T-accounts): the time accounts as a tree, each
     with the customer it bills, whether it is billable, the item it bills as,
     and its hours this month and unbilled. Edit an account's settings, add
     one under it, rename it (with a preview of every line it changes), and
     declare the accounts the time files use that the accounts file doesn't
     list. The month is in the URL (`?month=YYYY-MM`). Again whenever a file
     changes on disk. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {fetchCustomers, type CustomerSaved} from "#lib/customers.js";
    import {EditorApiError, fetchEditorOptions} from "#lib/editorApi.js";
    import {localToday} from "#lib/invoices.js";
    import AddCustomerDialog from "#lib/components/AddCustomerDialog.svelte";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import {archiveAccount, declareAccount, renameAccount, setAccountTag, type AccountRow, type RenamePreview} from "#lib/time/api/accounts.js";
    import {refusal} from "#lib/time/api/entries.js";
    import AccountDialog from "#lib/time/accounts/AccountDialog.svelte";
    import AccountTree from "#lib/time/accounts/AccountTree.svelte";
    import AddAccountDialog from "#lib/time/accounts/AddAccountDialog.svelte";
    import RenameDialog from "#lib/time/accounts/RenameDialog.svelte";
    import {AccountsStore} from "#lib/time/accounts/accountsStore.svelte.js";
    import type {TagChange} from "#lib/time/accounts/edit.js";
    import {addMonths, monthName} from "#lib/time/entries/dates.js";

    const thisMonth = localToday().slice(0, 7);
    const month = $derived(/^\d{4}-\d{2}$/.test(page.url.searchParams.get("month") ?? "") ? (page.url.searchParams.get("month") as string) : thisMonth);

    const store = new AccountsStore();
    $effect(() => {
        const m = month;
        untrack(() => void store.load(m));
    });

    let customers = $state<ComboOption[]>([]);
    let items = $state<ComboOption[]>([]);

    function loadPickers(): void {
        fetchCustomers()
            .then((rows) => (customers = rows.map((r) => ({value: r.ledger_customer === "" ? r.title : r.ledger_customer, label: r.title}))))
            .catch((e: unknown) => console.warn("customers:", e));
        fetchEditorOptions()
            .then((options) => (items = options.catalog.map((c) => ({value: c.code, label: c.code, detail: c.description}))))
            .catch((e: unknown) => console.warn("catalog:", e));
    }
    onMount(loadPickers);
    onFilesChanged(() => {
        void store.load();
        loadPickers();
    });

    function setMonth(next: string): void {
        const base = resolve("/time/accounts");
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
        void goto(next === thisMonth ? base : `${base}?month=${next}`, {reset: false});
    }

    const notSetUp = $derived(store.error instanceof EditorApiError && store.error.code === "not-set-up");
    const view = $derived(store.view);

    // ---- Dialogs: one at a time.
    type Dialog =
        | {kind: "edit"; row: AccountRow}
        | {kind: "add"; parent: string | null}
        | {kind: "rename"; from: string}
        | {kind: "customer"; row: AccountRow; name: string};
    let dialog = $state<Dialog | null>(null);
    let notice = $state<string | null>(null);

    /** A write, closing the dialog when it's done; else its refusal in words. */
    async function written(op: () => Promise<unknown>): Promise<string | null> {
        const outcome = await store.write(op);
        if (!outcome.ok) return outcome.message;
        dialog = null;
        return null;
    }

    const saveTags = (account: string, changes: TagChange[]): Promise<string | null> =>
        written(async () => {
            for (const change of changes) {
                if (change.tag === "archived") await archiveAccount(account, change.value !== null);
                else await setAccountTag(account, change.tag, change.value);
            }
        });

    async function preview(from: string, to: string): Promise<RenamePreview | string> {
        try {
            return (await renameAccount(from, to, false)).done;
        } catch (e) {
            return refusal(e).message;
        }
    }

    async function customerCreated(row: AccountRow, saved: CustomerSaved): Promise<void> {
        const name = saved.ledger_customer === "" ? saved.title : saved.ledger_customer;
        dialog = null;
        if (row.customer !== null && name !== row.customer.value) {
            const outcome = await store.write(() => setAccountTag(row.customer?.from ?? row.account, "customer", name));
            notice = outcome.ok ? null : outcome.message;
        } else {
            await store.load();
        }
        loadPickers();
    }

    async function declare(account: string): Promise<void> {
        const outcome = await store.write(() => declareAccount(account));
        notice = outcome.ok ? null : outcome.message;
    }

    registerKeys({
        id: "time-accounts-page",
        priority: PRIORITY.page,
        bindings: [
            {
                keys: "A",
                label: "Add a top-level account",
                group: "Time accounts",
                enabled: () => view !== null && dialog === null,
                run: () => (dialog = {kind: "add", parent: null}),
            },
            {keys: "[", label: "Previous month", group: "Time accounts", run: () => setMonth(addMonths(month, -1))},
            {keys: "]", label: "Next month", group: "Time accounts", run: () => setMonth(addMonths(month, 1))},
        ],
    });
</script>

<svelte:head>
    <title>Time accounts · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-6xl flex-col gap-4">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">Time accounts</h1>
        <div class="flex flex-wrap items-center gap-2">
            <div class="flex items-center gap-1" data-testid="accounts-month">
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => setMonth(addMonths(month, -1))} aria-label="Previous month">←</button>
                <span class="min-w-32 text-center text-sm font-medium" data-testid="accounts-month-label">{monthName(month)}</span>
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => setMonth(addMonths(month, 1))} aria-label="Next month">→</button>
            </div>
            {#if view !== null && view.file_exists}
                <button type="button" class="btn btn-primary btn-sm" onclick={() => (dialog = {kind: "add", parent: null})} data-testid="accounts-add"
                    >Add account</button
                >
            {/if}
        </div>
    </div>

    {#if notice !== null}
        <div class="alert py-2 text-sm alert-error" role="alert" data-testid="accounts-notice">{notice}</div>
    {/if}

    {#if notSetUp}
        <EmptyState
            title="Time tracking isn't set up"
            text="These books have no time files yet. Set up time tracking from Track in the sidebar, then your accounts show here."
            testid="accounts-not-set-up"
        />
    {:else}
        <AsyncSection
            view={store.state}
            value={view}
            error={store.error}
            testid="accounts-error"
            label="the time accounts"
            loadingLabel="Loading accounts"
            onRetry={() => void store.retry()}
        >
            {#snippet children(tree)}
                {#if !tree.file_exists}
                    <p class="alert text-sm alert-warning" data-testid="accounts-no-file">
                        There's no accounts file yet, so accounts can't be added or changed here. Set up time tracking from Track to make one.
                    </p>
                {/if}
                {#if tree.undeclared.length > 0}
                    <section
                        class="flex flex-col gap-2 rounded-box border border-warning/40 p-3"
                        aria-labelledby="undeclared-title"
                        data-testid="accounts-undeclared"
                    >
                        <h2 id="undeclared-title" class="font-medium">Not declared yet</h2>
                        <p class="text-sm text-base-content/70">
                            Time is recorded on these, but the accounts list doesn't have them. Declare one to give it a customer or make it billable.
                        </p>
                        <ul class="flex flex-col gap-1">
                            {#each tree.undeclared as account (account)}
                                <li class="flex items-center gap-2" data-testid="accounts-undeclared-row" data-account={account}>
                                    <span class="font-mono text-sm">{account}</span>
                                    {#if tree.file_exists}
                                        <button
                                            type="button"
                                            class="btn btn-outline btn-xs"
                                            onclick={() => void declare(account)}
                                            data-testid="accounts-declare">Declare</button
                                        >
                                    {/if}
                                </li>
                            {/each}
                        </ul>
                    </section>
                {/if}
                {#if tree.accounts.length === 0}
                    <EmptyState
                        title="No time accounts yet"
                        text="Accounts are what time is recorded against, such as clients:acme or internal:admin."
                        action={tree.file_exists ? "Add account" : undefined}
                        onAction={() => (dialog = {kind: "add", parent: null})}
                        testid="accounts-empty"
                    />
                {:else}
                    <AccountTree
                        rows={tree.accounts}
                        folded={store.folded}
                        onFold={(account) => store.toggleFold(account)}
                        onEdit={(row) => (dialog = {kind: "edit", row})}
                        onAddChild={(row) => (dialog = {kind: "add", parent: row.account})}
                        onRename={(row) => (dialog = {kind: "rename", from: row.account})}
                        onCreateCustomer={(row) => (dialog = {kind: "customer", row, name: row.customer?.value ?? ""})}
                    />
                {/if}
            {/snippet}
        </AsyncSection>
    {/if}
</div>

{#if dialog?.kind === "edit"}
    {@const row = dialog.row}
    <AccountDialog {row} {customers} {items} onSave={(changes) => saveTags(row.account, changes)} onCancel={() => (dialog = null)} />
{:else if dialog?.kind === "add"}
    <AddAccountDialog
        parent={dialog.parent}
        existing={[...(view?.accounts.map((r) => r.account) ?? [])]}
        onAdd={(account) => written(() => declareAccount(account))}
        onCancel={() => (dialog = null)}
    />
{:else if dialog?.kind === "rename"}
    {@const from = dialog.from}
    <RenameDialog
        {from}
        onPreview={(to) => preview(from, to)}
        onApply={(to) => written(() => renameAccount(from, to, true))}
        onCancel={() => (dialog = null)}
    />
{:else if dialog?.kind === "customer"}
    {@const row = dialog.row}
    <AddCustomerDialog name={dialog.name} onCreated={(saved) => void customerCreated(row, saved)} onCancel={() => (dialog = null)} />
{/if}
