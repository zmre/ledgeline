<!-- The customer list's filter bar (invoicer PLAN 5b, S2): a search, "Owes
     money" and "Overdue" toggles, and person or organization. Like the
     invoice list's (InvoiceFilters): it shows the URL's state and asks for a
     new one; the search is debounced and replaces the history step, the rest
     add one. Enter in the search opens the customer when only one is left.
     Keyboard (PLAN 5k): `/` focuses the search; Escape in it clears it, a
     second Escape leaves it for the list's `j`/`k`. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {searchKeys} from "#lib/keys/pageSearch.svelte.js";
    import {SearchBox} from "#lib/searchBox.svelte.js";
    import {customerFiltered, type CustomerKind, type CustomerQuery} from "#lib/listQuery.js";

    let {
        query,
        onChange,
        onJump,
        onClear,
        debounceMs = 250,
    }: {
        query: CustomerQuery;
        onChange: (next: CustomerQuery, options: {replace: boolean}) => void;
        onJump: (text: string) => void;
        onClear: () => void;
        debounceMs?: number;
    } = $props();

    // The search box: sent after a pause, replacing the history step; a `q`
    // from Back or a link replaces its text (SearchBox).
    const search = new SearchBox(
        untrack(() => query.q),
        (q) => onChange({...query, q}, {replace: true}),
        untrack(() => debounceMs)
    );
    $effect(() => {
        const q = query.q;
        untrack(() => search.sync(q));
    });
    $effect(() => () => search.dispose());

    function searchKey(event: KeyboardEvent): void {
        if (event.key !== "Enter") return;
        event.preventDefault();
        search.flush();
        onJump(search.text);
    }

    const set = (patch: Partial<CustomerQuery>): void => onChange({...query, ...patch}, {replace: false});
    const filtered = $derived(customerFiltered(query));
</script>

<div role="search" aria-label="Filter customers" class="flex flex-wrap items-end gap-x-3 gap-y-2" data-testid="customer-filters">
    <label class="flex min-w-0 grow basis-full flex-col gap-1 text-sm sm:basis-56"
        >Search
        <input
            class="input w-full input-sm"
            type="search"
            placeholder="Name, billing name or email"
            value={search.text}
            oninput={(event) => search.type(event.currentTarget.value)}
            onkeydown={searchKey}
            onblur={() => search.flush()}
            use:searchKeys={{clear: () => search.clear()}}
            data-testid="customer-search"
        />
    </label>
    <label class="flex min-w-0 grow basis-40 flex-col gap-1 text-sm sm:grow-0"
        >Kind
        <select
            class="select w-full select-sm"
            value={query.kind}
            onchange={(event) => set({kind: event.currentTarget.value as CustomerKind | ""})}
            data-testid="customer-filter-kind"
        >
            <option value="">People and organizations</option>
            <option value="organization">Organizations</option>
            <option value="person">People</option>
        </select>
    </label>
    <div class="flex flex-wrap items-center gap-2" role="group" aria-label="Show only">
        <button
            type="button"
            class="btn btn-sm {query.owes ? 'btn-primary' : 'btn-outline'}"
            aria-pressed={query.owes}
            onclick={() => set({owes: !query.owes})}
            data-testid="customer-filter-owes">Owes money</button
        >
        <button
            type="button"
            class="btn btn-sm {query.overdue ? 'btn-error' : 'btn-outline'}"
            aria-pressed={query.overdue}
            onclick={() => set({overdue: !query.overdue})}
            data-testid="customer-filter-overdue">Overdue</button
        >
    </div>
    {#if filtered}
        <button type="button" class="btn btn-ghost btn-sm" onclick={onClear} data-testid="customer-filters-clear">Clear filters</button>
    {/if}
</div>
