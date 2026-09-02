<script lang="ts">
    // The QuickBooks accounts nothing in the journal maps yet — one row per
    // account, a text field for the Ledgeline account it should become, and a
    // single "Map accounts" submit.
    //
    // This does NOT grow a second way to write an alias. Pressing the button
    // hands the typed rows to `qbJournalStore.saveMappings`, which goes
    // through the EXISTING alias-editing wire (`PUT /api/aliases/{*journalId}`
    // via `aliasStore.save` — the same call the Account Aliases tab makes) and
    // then re-fetches the preview, so a mapped account simply drops off
    // `accounts` on the next render — this component never removes a row
    // itself.
    //
    // A row's typed replacement is validated with the alias editor's OWN
    // rules (`qbJournalModel.mappingProblems`, which is `aliasModel.validateRow`
    // under the QuickBooks account name as a fixed pattern) so a value this
    // component would accept and the engine would refuse can never diverge.
    import AccountInput from "$lib/journal/edit/AccountInput.svelte";
    import {hasMappingsToSave, mappingProblems} from "../qbJournalModel";

    let {
        accounts,
        accountNames,
        draftFor,
        onDraft,
        saving,
        error,
        onSave,
    }: {
        /** Distinct QuickBooks account names in first-seen order — `WireQbPreview.unmappedAccounts`. */
        accounts: readonly string[];
        accountNames: string[];
        draftFor: (account: string) => string;
        onDraft: (account: string, value: string) => void;
        saving: boolean;
        error: string | null;
        onSave: () => void;
    } = $props();

    const drafts = $derived(Object.fromEntries(accounts.map((account) => [account, draftFor(account)])));
    const canSave = $derived(hasMappingsToSave(accounts, drafts));
</script>

<section class="flex flex-col gap-3 rounded-box border border-warning/40 p-3" aria-label="Unmapped QuickBooks accounts" data-testid="qb-unmapped">
    <h2 class="text-sm font-semibold tracking-tight">Map QuickBooks accounts</h2>
    <p class="text-xs text-base-content/60">
        These account names come straight from the export. Type the Ledgeline account each one is, and Ledgeline adds a plain <code>alias</code> for it — the same
        mapping the Account Aliases tab edits. A mapping on a parent account also covers its own sub-accounts.
    </p>

    {#each accounts as account (account)}
        {@const value = draftFor(account)}
        {@const problems = value.trim() === "" ? [] : mappingProblems(account, value)}
        <div class="flex flex-col gap-1 rounded-box border border-base-content/10 p-2">
            <div class="flex flex-wrap items-end gap-2">
                <label class="form-control grow">
                    <span class="label-text text-xs">QuickBooks account</span>
                    <input
                        type="text"
                        class="input-bordered input w-full font-mono input-sm"
                        value={account}
                        disabled
                        readonly
                        data-testid="qb-unmapped-account"
                    />
                </label>
                <label class="form-control grow">
                    <span class="label-text text-xs">Your account</span>
                    <AccountInput
                        bind:value={() => value, (next) => onDraft(account, next)}
                        {accountNames}
                        disabled={saving}
                        placeholder="assets:bank:checking"
                    />
                </label>
            </div>
            {#if problems.length > 0}
                <ul class="list-inside list-disc text-xs text-error" data-testid="qb-mapping-problems">
                    {#each problems as problem (problem)}
                        <li>{problem}</li>
                    {/each}
                </ul>
            {/if}
        </div>
    {/each}

    <div class="flex flex-wrap items-center gap-2">
        <button type="button" class="btn btn-primary btn-sm" disabled={saving || !canSave} onclick={onSave} data-testid="qb-map-accounts">
            {#if saving}<span class="loading loading-xs loading-spinner"></span>{/if}
            Map accounts
        </button>
        {#if error !== null}
            <span class="text-sm text-error" role="alert" data-testid="qb-mapping-error">{error}</span>
        {/if}
    </div>
</section>
