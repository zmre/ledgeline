<!-- The new chart of accounts, one row per `account` line, before anything is
     written: rename a line in place or remove it. Deliberately modest — the
     full editor (tags, notes, adding accounts) is Settings → Accounts, which
     works on the new ledger the moment it exists. -->
<script lang="ts">
    import {accountProblems, effectiveTypeName, tagSummary} from "../model";
    import type {TemplateAccount} from "../types";

    let {
        accounts,
        onRename,
        onRemove,
    }: {
        accounts: readonly TemplateAccount[];
        onRename: (index: number, name: string) => void;
        onRemove: (index: number) => void;
    } = $props();

    const problems = $derived(accountProblems(accounts));
</script>

<div class="flex flex-col gap-2" data-testid="new-ledger-accounts">
    <p class="text-sm text-base-content/70">
        {accounts.length} account{accounts.length === 1 ? "" : "s"}. Rename or remove any you won't use. Subaccounts take their parent's type, and you can
        change everything later in Settings → Accounts.
    </p>
    <div class="max-h-[50vh] overflow-y-auto rounded-box border border-base-300">
        <table class="table table-xs">
            <thead class="sticky top-0 bg-base-200">
                <tr>
                    <th>Account</th>
                    <th>Type</th>
                    <th>Tags</th>
                    <th><span class="sr-only">Remove</span></th>
                </tr>
            </thead>
            <tbody>
                {#each accounts as account, index (index)}
                    {@const problem = problems.get(index)}
                    <tr data-testid="new-ledger-account-row">
                        <td class="w-1/2">
                            <input
                                class="input w-full font-mono input-xs {problem === undefined ? '' : 'input-error'}"
                                value={account.name}
                                aria-label="Account name"
                                aria-invalid={problem !== undefined}
                                title={problem}
                                onchange={(event) => onRename(index, event.currentTarget.value)}
                            />
                            {#if problem !== undefined}
                                <div class="mt-0.5 text-xs text-error">{problem}</div>
                            {/if}
                        </td>
                        <td class="text-xs whitespace-nowrap">{effectiveTypeName(accounts, account) ?? "—"}</td>
                        <td class="font-mono text-xs text-base-content/70">{tagSummary(account)}</td>
                        <td class="text-right">
                            <button
                                type="button"
                                class="btn btn-ghost btn-xs"
                                aria-label="Remove {account.name}"
                                title="Remove this account"
                                onclick={() => onRemove(index)}>✕</button
                            >
                        </td>
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>
</div>
