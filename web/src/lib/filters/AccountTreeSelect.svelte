<script lang="ts">
    // Account-tree multi-select (WP-04): dropdown with search, tri-state
    // checkboxes over a subtree-root selection set. Selection state and its
    // mutators are props so any store can drive it (journal filters, WP-10
    // holdings scope); `accountNames` is a prop for the same reason.
    import {buildAccountTree, type AccountNode} from "$lib/domain/accounts";
    import {filterTree, selectionState} from "./treeSelect";

    let {
        accountNames,
        selected,
        onToggle,
        onClear,
    }: {accountNames: string[]; selected: ReadonlySet<string>; onToggle: (name: string) => void; onClear: () => void} = $props();

    let search = $state("");
    const tree = $derived(buildAccountTree(accountNames));
    const visible = $derived(filterTree(tree, search));
    const selectedCount = $derived(selected.size);
</script>

<details class="dropdown">
    <summary class="btn btn-sm">
        Accounts
        {#if selectedCount > 0}
            <span class="badge badge-primary badge-sm">{selectedCount}</span>
        {/if}
        <svg
            class="h-3 w-3 opacity-60"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            aria-hidden="true"
        >
            <path d="m6 9 6 6 6-6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
    </summary>
    <div class="dropdown-content bg-base-200 rounded-box z-20 mt-1 w-72 max-w-[calc(100vw-2rem)] p-2 shadow-lg">
        <div class="flex items-center gap-2 pb-2">
            <input
                type="text"
                class="input input-sm w-full"
                placeholder="Search accounts…"
                bind:value={search}
                aria-label="Search accounts"
                autocomplete="off"
            />
            {#if selectedCount > 0}
                <button type="button" class="btn btn-ghost btn-xs shrink-0" onclick={() => onClear()}>Clear</button>
            {/if}
        </div>
        <ul class="max-h-64 overflow-y-auto">
            {#if visible.length === 0}
                <li class="text-base-content/60 px-2 py-1 text-sm">No matching accounts</li>
            {:else}
                {@render nodes(visible, 0)}
            {/if}
        </ul>
    </div>
</details>

{#snippet nodes(list: AccountNode[], depth: number)}
    {#each list as node (node.fullName)}
        {@const state = selectionState(selected, node.fullName)}
        <li>
            <label class="hover:bg-base-300 flex cursor-pointer items-center gap-2 rounded px-2 py-1" style="padding-left: {0.5 + depth}rem">
                <input
                    type="checkbox"
                    class="checkbox checkbox-xs"
                    checked={state === "checked"}
                    indeterminate={state === "indeterminate"}
                    onchange={() => onToggle(node.fullName)}
                />
                <span class="truncate text-sm" title={node.fullName}>{node.name}</span>
            </label>
        </li>
        {#if node.children.length > 0}
            {@render nodes(node.children, depth + 1)}
        {/if}
    {/each}
{/snippet}
