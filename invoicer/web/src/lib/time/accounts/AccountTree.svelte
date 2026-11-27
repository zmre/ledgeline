<!-- The time accounts as a tree (T-accounts): each row indented by depth,
     a branch folding with its button or ←/→. Columns: account · customer
     (resolved; "from clients:acme" when inherited) · billable · item ·
     hours this month · unbilled. A customer with no Invoicer note is flagged
     with a way to create it; the server's warnings show under the account.
     Item and Billable drop out below `md` (container queries, plan 39).

     Keyboard: `j`/`k` move the cursor, → unfolds (or steps into the
     branch), ← folds (or steps out to the parent), Enter edits, `a` adds an
     account under the cursored one, `r` renames it. -->
<script lang="ts">
    import type {SvelteSet} from "svelte/reactivity";
    import {CURSOR_ROW, revealCursored, rowKeys} from "#lib/keys/rowKeys.svelte.js";
    import type {AccountRow} from "../api/accounts.js";
    import {inheritedText} from "./edit.js";
    import {indentRem, isArchived, parentOf, tagText, visibleRows, withChildren} from "./tree.js";

    let {
        rows,
        folded,
        onFold,
        onEdit,
        onAddChild,
        onRename,
        onCreateCustomer,
    }: {
        rows: AccountRow[];
        folded: SvelteSet<string>;
        onFold: (account: string) => void;
        onEdit: (row: AccountRow) => void;
        onAddChild: (row: AccountRow) => void;
        onRename: (row: AccountRow) => void;
        /** The customer `row` names has no note: make one. */
        onCreateCustomer: (row: AccountRow) => void;
    } = $props();

    const shown = $derived(visibleRows(rows, folded));
    const branches = $derived(withChildren(rows));

    let tableEl = $state<HTMLTableElement | null>(null);
    const cursored = (): AccountRow | null => cursor.item;

    function right(): void {
        const row = cursored();
        if (row === null || !branches.has(row.account)) return;
        if (folded.has(row.account)) onFold(row.account);
        else {
            cursor.move(1);
            revealCursored(() => tableEl);
        }
    }

    function left(): void {
        const row = cursored();
        if (row === null) return;
        if (branches.has(row.account) && !folded.has(row.account)) {
            onFold(row.account);
            return;
        }
        // Out to the nearest ancestor that has a row.
        for (let p = parentOf(row.account); p !== null; p = parentOf(p)) {
            const at = shown.findIndex((r) => r.account === p);
            if (at >= 0) {
                cursor.to(at);
                revealCursored(() => tableEl);
                return;
            }
        }
    }

    const run = (action: () => (row: AccountRow) => void) => (): void => {
        const row = cursored();
        if (row !== null) action()(row);
    };
    const some = (): boolean => cursor.item !== null;

    const cursor = rowKeys<AccountRow>({
        id: "time-accounts",
        group: "Time accounts",
        noun: "account",
        items: () => shown,
        keyOf: (r) => r.account,
        root: () => tableEl,
        open: () => onEdit,
        extra: [
            {keys: "ArrowRight", label: "Unfold the account, or step into it", group: "Time accounts", enabled: some, run: right},
            {keys: "ArrowLeft", label: "Fold the account, or step out to its parent", group: "Time accounts", enabled: some, run: left},
            {keys: "a", label: "Add an account under this one", group: "Time accounts", enabled: some, run: run(() => onAddChild)},
            {keys: "r", label: "Rename the account", group: "Time accounts", enabled: some, run: run(() => onRename)},
        ],
    });

    const leaf = (account: string): string => account.slice(account.lastIndexOf(":") + 1);
</script>

<div class="overflow-x-auto rounded-box border border-base-300">
    <table class="table table-sm" bind:this={tableEl} data-testid="account-tree">
        <thead>
            <tr>
                <th>Account</th>
                <th>Customer</th>
                <th class="hidden md:table-cell">Billable</th>
                <th class="hidden md:table-cell">Item</th>
                <th class="text-right">This month</th>
                <th class="text-right">Unbilled</th>
                <th><span class="sr-only">Actions</span></th>
            </tr>
        </thead>
        <tbody>
            {#each shown as row, at (row.account)}
                {@const branch = branches.has(row.account)}
                {@const archived = isArchived(row)}
                <tr
                    class={["hover:bg-base-200", cursor.index === at && CURSOR_ROW, archived && "text-base-content/50"]}
                    data-cursor={cursor.index === at ? "" : undefined}
                    data-testid="account-row"
                    data-account={row.account}
                    aria-level={row.depth + 1}
                >
                    <td>
                        <div class="flex items-start gap-1" style="padding-left: {indentRem(row.depth)}rem">
                            {#if branch}
                                <button
                                    type="button"
                                    class="btn btn-square btn-ghost btn-xs"
                                    aria-expanded={!folded.has(row.account)}
                                    aria-label="{folded.has(row.account) ? 'Unfold' : 'Fold'} {row.account}"
                                    onclick={() => onFold(row.account)}
                                    data-testid="account-fold">{folded.has(row.account) ? "▸" : "▾"}</button
                                >
                            {:else}
                                <span class="inline-block w-6" aria-hidden="true"></span>
                            {/if}
                            <div class="flex min-w-0 flex-col">
                                <span class="font-medium" title={row.account}
                                    >{leaf(row.account)}
                                    {#if !row.declared}<span class="badge badge-outline badge-xs badge-warning">undeclared</span>{/if}
                                    {#if archived}<span class="badge badge-ghost badge-xs">archived</span>{/if}
                                </span>
                                {#each row.warnings as warning (warning)}
                                    <span class="text-xs text-warning" data-testid="account-warning">{warning}</span>
                                {/each}
                            </div>
                        </div>
                    </td>
                    <td data-testid="account-customer">
                        {#if row.customer !== null}
                            <span>{row.customer.value}</span>
                            {#if row.customer.inherited}<span class="block text-xs text-base-content/60">{inheritedText(row.customer)}</span>{/if}
                            {#if !row.customer_found && !row.customer.inherited}
                                <button
                                    type="button"
                                    class="btn mt-1 block btn-outline btn-warning btn-xs"
                                    onclick={() => onCreateCustomer(row)}
                                    data-testid="account-create-customer">Create customer</button
                                >
                            {/if}
                        {:else}
                            <span class="text-base-content/50">—</span>
                        {/if}
                    </td>
                    <td class="hidden md:table-cell" data-testid="account-billable">
                        {tagText(row.billable)}
                        {#if row.billable?.inherited}<span class="block text-xs text-base-content/60">{inheritedText(row.billable)}</span>{/if}
                    </td>
                    <td class="hidden md:table-cell" data-testid="account-item">
                        {tagText(row.item)}
                        {#if row.item?.inherited}<span class="block text-xs text-base-content/60">{inheritedText(row.item)}</span>{/if}
                    </td>
                    <td class="text-right tabular-nums" data-testid="account-month">{row.month.seconds === 0 ? "" : row.month.text}</td>
                    <td class="text-right tabular-nums" data-testid="account-unbilled">{row.unbilled.seconds === 0 ? "" : row.unbilled.text}</td>
                    <td class="whitespace-nowrap">
                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => onEdit(row)} data-testid="account-edit">Edit</button>
                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => onAddChild(row)} data-testid="account-add-child">Add under</button>
                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => onRename(row)} data-testid="account-rename">Rename</button>
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
</div>
