<!-- Add an account (T-accounts *Add child*), declared in the accounts file;
     under `parent` the name starts as `parent:`. It inherits its parent's
     customer, billable and item until it says otherwise. -->
<script lang="ts">
    import {untrack} from "svelte";
    import DialogShell from "../DialogShell.svelte";

    let {
        parent = null,
        existing,
        onAdd,
        onCancel,
    }: {
        parent?: string | null;
        /** Accounts already there: a name among them is refused before asking. */
        existing: readonly string[];
        /** Declare `account`; null when done, else why not. */
        onAdd: (account: string) => Promise<string | null>;
        onCancel: () => void;
    } = $props();

    let name = $state(untrack(() => (parent === null ? "" : `${parent}:`)));
    let busy = $state(false);
    let error = $state<string | null>(null);

    const wanted = $derived(name.trim());
    const problem = $derived(
        wanted === "" || wanted.endsWith(":")
            ? "Write the account's name."
            : /\s{2}|\t|;/.test(wanted)
              ? "An account name can't hold two spaces in a row, a tab or “;”."
              : existing.includes(wanted)
                ? `${wanted} is already there.`
                : null
    );

    async function add(): Promise<void> {
        busy = true;
        error = await onAdd(wanted);
        busy = false;
    }
</script>

<DialogShell
    title={parent === null ? "Add an account" : `Add an account under ${parent}`}
    submitLabel="Add"
    {busy}
    error={error ?? (wanted !== "" && !wanted.endsWith(":") && problem !== null ? problem : null)}
    submitDisabled={problem !== null}
    onSubmit={() => void add()}
    {onCancel}
    testid="account-add-dialog"
>
    <label class="flex flex-col gap-1 text-sm"
        >Account
        <input class="input w-full input-sm" bind:value={name} autocomplete="off" spellcheck="false" data-initial data-testid="account-add-name" />
    </label>
</DialogShell>
