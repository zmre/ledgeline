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

    /**
     * How long a pause means "done typing".
     *
     * 150 ms was shorter than the gap between one keystroke and the next for an
     * ordinary typist, so it fired MID-WORD and every character paid for a full
     * re-derivation of the journal view. That is affordable under the default
     * last-90-days filter and not remotely affordable under "All time": one
     * filter change there measured 279 ms at 150k transactions in node, and the
     * app ships in WKWebView against a journal larger than the fixture.
     *
     * 300 ms clears the inter-keystroke interval of even a fast typist while
     * staying inside the ~400 ms where a deferred result still reads as
     * responsive. Nothing the user is LOOKING at waits on it — `text` is local
     * state, so the field itself echoes every keystroke immediately; only the
     * expensive downstream work is deferred.
     */
    const DEBOUNCE_MS = 300;

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

<label class="input flex w-full items-center gap-2 input-sm">
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
        <button type="button" class="btn btn-circle shrink-0 btn-ghost btn-xs" onclick={clear} aria-label="Clear search">✕</button>
    {/if}
</label>
