<!-- The scope bar's "Holdings" multi-select (plan 47 D6): which rows the
     value-over-time chart and the stat tiles are narrowed to. Symbols on
     Stocks, accounts on Other — the options are the tab's own report rows, and
     the tab owns the selection (switching tab clears it).

     AccountTreeSelect's shape on purpose (a <details> dropdown, a search box,
     a checkbox per row, Clear, Escape/outside-click via `dismissible`), so the
     two dropdowns side by side behave alike. Flat, not a tree: a holding has
     no parent.

     Keyboard-first: `openKey` opens it with the search focused; typing narrows
     the list; ↓/↑ walk the checkboxes (↓ from the search box enters the list,
     ↑ off the first returns to it); Space ticks; Enter in the search box ticks
     the first match; Escape closes, and `dismissible` hands focus back to
     whatever opened it. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {dismissible} from "#ui/keys/dismissible.js";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import {holdingOptions, type HoldingOption} from "./selection";

    let {
        options,
        selected,
        onToggle,
        onClear,
        noun = "holdings",
        openKey = null,
    }: {
        /** The current tab's rows, in report order. */
        options: readonly HoldingOption[];
        selected: ReadonlySet<string>;
        onToggle: (key: string) => void;
        onClear: () => void;
        /** What a row is, for the search placeholder and the empty line ("holdings", "assets"). */
        noun?: string;
        /** Key that opens the dropdown with the search focused, or null for none. Static configuration. */
        openKey?: string | null;
    } = $props();

    let open = $state(false);
    let search = $state("");
    let field = $state<HTMLInputElement | null>(null);
    let list = $state<HTMLUListElement | null>(null);

    const visible = $derived(holdingOptions(options, selected, search));

    // Read once: `registerKeys` takes a plain object (see AccountTreeSelect).
    const key = untrack(() => openKey);
    if (key !== null) {
        registerKeys({
            id: "holdings-select",
            priority: PRIORITY.widget,
            bindings: [{keys: key, label: "Choose holdings to chart", group: "Holdings", run: openAndFocus}],
        });
    }

    function openAndFocus(): void {
        // Open first: `.focus()` on a display:none element is a silent no-op.
        open = true;
        field?.focus();
        field?.select();
    }

    const boxes = (): HTMLInputElement[] => [...(list?.querySelectorAll<HTMLInputElement>("input[type=checkbox]") ?? [])];

    function onSearchKey(event: KeyboardEvent): void {
        if (event.key === "ArrowDown") {
            event.preventDefault();
            boxes()[0]?.focus();
        } else if (event.key === "Enter") {
            event.preventDefault();
            const first = visible[0];
            if (first !== undefined) onToggle(first.key);
        }
    }

    function onBoxKey(event: KeyboardEvent): void {
        if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
        const all = boxes();
        const at = all.indexOf(event.target as HTMLInputElement);
        if (at < 0) return;
        event.preventDefault();
        const next = at + (event.key === "ArrowDown" ? 1 : -1);
        if (next < 0) field?.focus();
        else all[Math.min(next, all.length - 1)]?.focus();
    }
</script>

<details
    class="ll-hang-from-row dropdown dropdown-start"
    bind:open
    use:dismissible={{active: open, onDismiss: () => (open = false), outside: true}}
    data-testid="holdings-select"
>
    <summary class="btn btn-sm" title="Narrow the chart and the totals to some {noun}; the table keeps every row">
        Holdings
        {#if selected.size > 0}
            <span class="badge badge-sm badge-primary" data-testid="holdings-select-count">{selected.size}</span>
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
    <div class="dropdown-content z-20 mt-1 w-72 max-w-[calc(100vw-2rem)] rounded-box bg-base-200 p-2 shadow-lg">
        <div class="flex items-center gap-2 pb-2">
            <input
                bind:this={field}
                type="text"
                class="input w-full input-sm"
                placeholder="Search {noun}…"
                bind:value={search}
                onkeydown={onSearchKey}
                aria-label="Search {noun}"
                autocomplete="off"
                data-testid="holdings-select-search"
            />
            {#if selected.size > 0}
                <button type="button" class="btn shrink-0 btn-ghost btn-xs" onclick={() => onClear()}>Clear</button>
            {/if}
        </div>
        <ul bind:this={list} class="max-h-64 overflow-y-auto" aria-label="Holdings to chart">
            {#if visible.length === 0}
                <li class="px-2 py-1 text-sm text-muted">No matching {noun}</li>
            {:else}
                {#each visible as option (option.key)}
                    <li>
                        <label class="flex cursor-pointer items-center gap-2 rounded px-2 py-1 hover:bg-base-300" title={option.detail ?? option.label}>
                            <input
                                type="checkbox"
                                class="checkbox checkbox-xs"
                                checked={selected.has(option.key)}
                                onchange={() => onToggle(option.key)}
                                onkeydown={onBoxKey}
                                data-testid="holdings-select-{option.key}"
                            />
                            <span class="min-w-0 truncate text-sm">{option.label}</span>
                            {#if option.detail !== null}
                                <span class="ml-auto min-w-0 truncate text-xs text-muted">{option.detail}</span>
                            {/if}
                        </label>
                    </li>
                {/each}
            {/if}
        </ul>
    </div>
</details>
