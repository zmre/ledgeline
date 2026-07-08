<script lang="ts">
    // Debounced free-text filter input (WP-04). Local text echoes the store so
    // external changes (chip removal, reset, URL restore) refresh the field.
    import {filters} from "$lib/stores/filters.svelte";

    const DEBOUNCE_MS = 150;

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
        type="text"
        class="grow"
        value={text}
        oninput={onInput}
        placeholder="description, amount, account, comment…"
        aria-label="Search transactions"
        enterkeyhint="search"
        autocomplete="off"
    />
    {#if text !== ""}
        <button type="button" class="btn btn-ghost btn-xs btn-circle shrink-0" onclick={clear} aria-label="Clear search">✕</button>
    {/if}
</label>
