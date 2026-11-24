<!-- Add a customer, minimally, from the customer combobox (invoicer PLAN 4a):
     a name, person or organization, a billing email and address, terms. It
     writes a customer note in `customers-dir` (tagged `customer`) through the
     server; the customers page (4b) edits everything else. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {createCustomer, type NewCustomer} from "#lib/editorApi.js";

    let {
        name: initial,
        onCreated,
        onCancel,
    }: {
        name: string;
        onCreated: (customer: {path: string; title: string; link: string}) => void;
        onCancel: () => void;
    } = $props();

    // Starts as what was typed in the combobox; the user may change it here.
    let name = $state(untrack(() => initial));
    let kind = $state<NewCustomer["kind"]>("organization");
    let email = $state("");
    let address = $state("");
    let terms = $state<number | null>(null);
    let busy = $state(false);
    let error = $state<string | null>(null);

    const blank = (s: string): string | null => (s.trim() === "" ? null : s.trim());

    async function submit(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        if (name.trim() === "") return;
        busy = true;
        error = null;
        try {
            onCreated(await createCustomer({name: name.trim(), kind, email: blank(email), address: blank(address), terms}));
        } catch (e) {
            error = e instanceof Error ? e.message : String(e);
        } finally {
            busy = false;
        }
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
        }
    }

    function focus(node: HTMLInputElement): void {
        node.focus();
    }
</script>

<div class="modal modal-open" role="presentation" onkeydown={onKeydown}>
    <div class="modal-box" role="dialog" aria-modal="true" aria-label="Add a customer">
        <form class="flex flex-col gap-3" onsubmit={submit} data-testid="add-customer">
            <h2 class="text-lg font-semibold">Add a customer</h2>
            <label class="flex flex-col gap-1 text-sm"
                >Name <input class="input input-sm" bind:value={name} required use:focus data-testid="add-customer-name" /></label
            >
            <fieldset class="flex gap-4 text-sm">
                <legend class="sr-only">Kind</legend>
                <label class="flex items-center gap-2"><input type="radio" class="radio radio-sm" bind:group={kind} value="organization" /> Organization</label>
                <label class="flex items-center gap-2"><input type="radio" class="radio radio-sm" bind:group={kind} value="person" /> Person</label>
            </fieldset>
            <label class="flex flex-col gap-1 text-sm">Billing email <input class="input input-sm" type="email" bind:value={email} /></label>
            <label class="flex flex-col gap-1 text-sm">Billing address <textarea class="textarea textarea-sm" rows="3" bind:value={address}></textarea></label>
            <label class="flex flex-col gap-1 text-sm"
                >Terms (net days) <input class="input w-28 input-sm" type="number" min="0" step="1" bind:value={terms} /></label
            >
            {#if error !== null}<div class="alert text-sm alert-error" role="alert">{error}</div>{/if}
            <div class="modal-action">
                <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy}>Cancel</button>
                <button type="submit" class="btn btn-primary btn-sm" disabled={busy || name.trim() === ""} data-testid="add-customer-save">Add customer</button>
            </div>
        </form>
    </div>
</div>
