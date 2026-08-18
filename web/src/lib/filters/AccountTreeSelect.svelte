<script lang="ts">
    // Account-tree multi-select (WP-04): dropdown with search, tri-state
    // checkboxes over a subtree-root selection set. Selection state and its
    // mutators are props so any store can drive it (journal filters, WP-10
    // holdings scope); `accountNames` is a prop for the same reason.
    import {untrack} from "svelte";
    import {buildAccountTree, type AccountNode} from "$lib/domain/accounts";
    import {dismissible} from "$lib/keys/dismissible";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {filterTree, selectionState} from "./treeSelect";

    let {
        accountNames,
        selected,
        onToggle,
        onClear,
        searchKey = null,
    }: {
        accountNames: string[];
        selected: ReadonlySet<string>;
        onToggle: (name: string) => void;
        onClear: () => void;
        /**
         * Key that opens this dropdown and focuses its search box, or null for
         * none.
         *
         * A prop rather than a priority race because this component is mounted
         * TWICE and the two mounts want different keys: the journal's FilterBar
         * passes "a" (there `/` is the free-text box), holdings' ScopeBar passes
         * "/" (there is no free-text box on that page). Resolving it by layer
         * priority would make the answer depend on mount order.
         */
        searchKey?: string | null;
    } = $props();

    let search = $state("");
    let open = $state(false);
    let field = $state<HTMLInputElement | null>(null);
    const tree = $derived(buildAccountTree(accountNames));
    const visible = $derived(filterTree(tree, search));
    const selectedCount = $derived(selected.size);

    // Read once, inside a closure: `registerKeys` takes a plain object by
    // contract, and both call sites pass a literal, so this genuinely is static
    // configuration. `untrack` states that rather than suppressing the
    // `state_referenced_locally` warning it would otherwise raise — this repo
    // carries no `svelte-ignore` comments.
    const openKey = untrack(() => searchKey);
    if (openKey !== null) {
        registerKeys({
            id: "account-tree",
            priority: PRIORITY.widget,
            bindings: [{keys: openKey, label: "Filter by account", group: "Filters", run: openAndFocus}],
        });
    }

    function openAndFocus(): void {
        // Open first: `.focus()` on a display:none element is a silent no-op, and
        // daisyUI hides `.dropdown-content` until the <details> is open. jsdom has
        // no layout engine and cannot verify this ordering, so it is covered by a
        // Playwright `toBeFocused` assertion instead.
        open = true;
        field?.focus();
        field?.select();
    }
</script>

<!-- `bind:open` moves the dropdown's state into the DOM where the dismissal
     action can close it. This is ColumnMenu's shape, adopted here because this
     dropdown had neither Escape nor outside-click before. -->
<details class="dropdown" bind:open use:dismissible={{active: open, onDismiss: () => (open = false), outside: true}}>
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
                bind:this={field}
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
