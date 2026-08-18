<script lang="ts">
    // Debounced free-text filter input (WP-04). Local text echoes the store so
    // external changes (chip removal, reset, URL restore) refresh the field.
    //
    // Owns the `/` binding. The component that owns the input registers the key
    // — no nonce store, no querySelector from the keyboard layer. Both of those
    // would need the keymap to know which pages have a search box, which is a
    // second list to keep in sync. Registering locally makes "Reports and
    // Imports have no `/`" automatic: no binding, no help row, nothing fires.
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {filters} from "$lib/stores/filters.svelte";

    const DEBOUNCE_MS = 150;

    let field = $state<HTMLInputElement | null>(null);

    registerKeys({
        id: "journal-search",
        priority: PRIORITY.widget,
        bindings: [{keys: "/", label: "Search transactions", group: "Filters", run: focusSearch}],
    });

    function focusSearch(): void {
        field?.focus();
        // Select, so `/` then typing REPLACES the old query rather than appending
        // to it — the same reasoning as `descAutofocus` in TransactionRow.
        field?.select();
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key !== "Escape") return;
        // Hand the keyboard back to the page. Deliberately does NOT clear: the ✕
        // is one click away, and a keystroke that silently destroys a typed query
        // is not something to discover by accident. The two-stage "clear, then
        // blur" version is the obvious improvement; this comment is why it isn't.
        event.preventDefault();
        field?.blur();
    }

    // Writable derived: tracks the store, but user keystrokes override it
    // locally until the debounced setQuery lands.
    let text = $derived(filters.value.query);
    let timer: ReturnType<typeof setTimeout> | null = null;

    function onInput(event: Event): void {
        text = (event.currentTarget as HTMLInputElement).value;
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
            timer = null;
            filters.setQuery(text);
        }, DEBOUNCE_MS);
    }

    function clear(): void {
        if (timer !== null) clearTimeout(timer);
        timer = null;
        text = "";
        filters.setQuery("");
    }
</script>

<label class="input input-sm flex w-full items-center gap-2">
    <svg
        class="h-4 w-4 opacity-60"
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        aria-hidden="true"
    >
        <circle cx="11" cy="11" r="7" />
        <path d="m21 21-4.3-4.3" stroke-linecap="round" />
    </svg>
    <input
        bind:this={field}
        type="text"
        class="grow"
        value={text}
        oninput={onInput}
        onkeydown={onKeydown}
        placeholder="description, amount, account, comment…"
        aria-label="Search transactions"
        enterkeyhint="search"
        autocomplete="off"
    />
    {#if text !== ""}
        <button type="button" class="btn btn-ghost btn-xs btn-circle shrink-0" onclick={clear} aria-label="Clear search">✕</button>
    {/if}
</label>
