<!-- The Time area's one filter bar, the same on Overview and Entries
     (`query.ts` here): the period (presets, custom dates, previous/next),
     then search, person (hidden while only one person has time), customer
     (hidden without any), billable, billing state and account. It shows the
     URL's filters (`query`) and asks for new ones (`onChange`); the page
     owns the URL. Choices are a history step each; the search text is
     debounced and replaces the current step, so typing doesn't fill history.

     The tab's own view options (`options`: Overview's comparison and depth,
     Entries' grouping) sit at the end of the period row, and `status` (the
     dates shown) under it, so both tabs read the same. The week grid picks
     its own week: `showPeriod={false}` hides the period.

     Keyboard: everything is a native control in tab order; `/` focuses the
     search (Escape clears it, a second Escape leaves it); `[` and `]` step
     the period (each page's key layer). Narrow windows: the controls wrap,
     the search taking a whole row below `sm`. -->
<script lang="ts">
    import {untrack, type Snippet} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import {searchKeys} from "#lib/keys/pageSearch.svelte.js";
    import {SearchBox} from "#lib/searchBox.svelte.js";
    import {
        BILLED_LABELS,
        BILLED_STATES,
        isCustom,
        isFiltered,
        PERIOD_LABELS,
        PERIODS,
        UNATTRIBUTED,
        type BilledState,
        type Period,
        type Range,
        type TimeFilters,
    } from "./query.js";

    let {
        query,
        range,
        people,
        customers,
        accounts,
        showPeriod = true,
        options,
        status,
        onChange,
        onStep,
        onClear,
        debounceMs = 250,
    }: {
        query: TimeFilters;
        /** The period's dates, both ends known: where Custom starts. */
        range: Pick<Range, "from" | "to">;
        /** Everyone with time (`""`: unattributed), in the order to list them. */
        people: readonly string[];
        customers: readonly string[];
        /** The account filter's suggestions. */
        accounts: string[];
        showPeriod?: boolean;
        options?: Snippet;
        status?: Snippet;
        onChange: (next: TimeFilters, options: {replace: boolean}) => void;
        /** Null: the period doesn't step (All time), so the arrows are off. */
        onStep: ((direction: -1 | 1) => void) | null;
        onClear: () => void;
        debounceMs?: number;
    } = $props();

    const custom = $derived(isCustom(query));
    const set = (patch: Partial<TimeFilters>): void => onChange({...query, ...patch}, {replace: false});

    function setPeriod(value: string): void {
        if (value === "custom") set({from: range.from, to: range.to});
        else set({period: value as Period, from: "", to: ""});
    }

    // The search: sent after a pause, as the invoice list's is.
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

    // The account: typed or picked, sent when chosen or committed.
    let accountText = $state(untrack(() => query.account));
    $effect(() => {
        const name = query.account;
        untrack(() => {
            if (name !== accountText.trim()) accountText = name;
        });
    });
    function setAccount(): void {
        const wanted = accountText.trim();
        if (wanted !== query.account) set({account: wanted});
    }

    const named = $derived(people.filter((p) => p !== ""));
    const showPeople = $derived(people.length > 1 || query.person !== "");
    const strayPerson = $derived(query.person !== "" && query.person !== UNATTRIBUTED && !named.includes(query.person));
    const strayCustomer = $derived(query.customer !== "" && !customers.includes(query.customer));
    const id = $props.id();
</script>

<div role="search" aria-label="Filter time" class="flex flex-col gap-2" data-testid="time-filters">
    <div class="flex flex-wrap items-end gap-x-3 gap-y-2">
        {#if showPeriod}
            <div class="flex min-w-0 flex-col gap-1 text-sm">
                <label for="{id}-period">Period</label>
                <div class="join">
                    <button
                        type="button"
                        class="btn join-item btn-sm"
                        aria-label="Previous period"
                        title="Previous period ([)"
                        disabled={onStep === null}
                        onclick={() => onStep?.(-1)}
                        data-testid="time-filter-prev">‹</button
                    >
                    <select
                        id="{id}-period"
                        class="select join-item w-40 select-sm"
                        value={custom ? "custom" : query.period}
                        onchange={(event) => setPeriod(event.currentTarget.value)}
                        data-testid="time-filter-period"
                    >
                        {#each PERIODS as period (period)}
                            <option value={period}>{PERIOD_LABELS[period]}</option>
                        {/each}
                        <option value="custom">Custom…</option>
                    </select>
                    <button
                        type="button"
                        class="btn join-item btn-sm"
                        aria-label="Next period"
                        title="Next period (])"
                        disabled={onStep === null}
                        onclick={() => onStep?.(1)}
                        data-testid="time-filter-next">›</button
                    >
                </div>
            </div>
            {#if custom}
                <label class="flex min-w-0 basis-36 flex-col gap-1 text-sm"
                    >From
                    <DateInput
                        value={query.from === "" ? range.from : query.from}
                        class="w-full"
                        aria-label="From"
                        max={range.to}
                        onchange={(event) => event.currentTarget.value !== "" && set({from: event.currentTarget.value})}
                        data-testid="time-filter-from"
                    />
                </label>
                <label class="flex min-w-0 basis-36 flex-col gap-1 text-sm"
                    >to
                    <DateInput
                        value={query.to === "" ? range.to : query.to}
                        class="w-full"
                        aria-label="To"
                        min={range.from}
                        onchange={(event) => event.currentTarget.value !== "" && set({to: event.currentTarget.value})}
                        data-testid="time-filter-to"
                    />
                </label>
            {/if}
        {/if}
        {#if options !== undefined}
            <div class="flex flex-wrap items-end gap-x-3 gap-y-2 sm:ml-auto" data-testid="time-view-options">{@render options()}</div>
        {/if}
    </div>
    {@render status?.()}
    <div class="flex flex-wrap items-end gap-x-3 gap-y-2">
        <label class="flex min-w-0 grow basis-full flex-col gap-1 text-sm sm:basis-48"
            >Search
            <input
                class="input w-full input-sm"
                type="search"
                placeholder="Note or account"
                value={search.text}
                oninput={(event) => search.type(event.currentTarget.value)}
                onkeydown={(event) => {
                    if (event.key === "Enter") search.flush();
                }}
                onblur={() => search.flush()}
                use:searchKeys={{clear: () => search.clear()}}
                data-testid="time-filter-search"
            />
        </label>
        {#if showPeople}
            <label class="flex min-w-0 grow basis-32 flex-col gap-1 text-sm sm:grow-0"
                >Person
                <select
                    class="select w-full select-sm"
                    value={query.person}
                    onchange={(event) => set({person: event.currentTarget.value})}
                    data-testid="time-filter-person"
                >
                    <option value="">Everyone</option>
                    {#each named as person (person)}
                        <option value={person}>{person}</option>
                    {/each}
                    {#if strayPerson}
                        <option value={query.person}>{query.person}</option>
                    {/if}
                    <option value={UNATTRIBUTED}>No one named</option>
                </select>
            </label>
        {/if}
        {#if customers.length > 0 || query.customer !== ""}
            <label class="flex min-w-0 grow basis-40 flex-col gap-1 text-sm sm:grow-0"
                >Customer
                <select
                    class="select w-full select-sm"
                    value={query.customer}
                    onchange={(event) => set({customer: event.currentTarget.value})}
                    data-testid="time-filter-customer"
                >
                    <option value="">Any customer</option>
                    {#each customers as customer (customer)}
                        <option value={customer}>{customer}</option>
                    {/each}
                    {#if strayCustomer}
                        <option value={query.customer}>{query.customer}</option>
                    {/if}
                </select>
            </label>
        {/if}
        <label class="flex min-w-0 grow basis-28 flex-col gap-1 text-sm sm:grow-0"
            >Billable
            <select
                class="select w-full select-sm"
                value={query.billable}
                onchange={(event) => set({billable: event.currentTarget.value as TimeFilters["billable"]})}
                data-testid="time-filter-billable"
            >
                <option value="">Either</option>
                <option value="yes">Billable</option>
                <option value="no">Not billable</option>
            </select>
        </label>
        <label class="flex min-w-0 grow basis-40 flex-col gap-1 text-sm sm:grow-0"
            >Billing
            <select
                class="select w-full select-sm"
                value={query.billed}
                onchange={(event) => set({billed: event.currentTarget.value as BilledState | ""})}
                data-testid="time-filter-billed"
            >
                <option value="">Any</option>
                {#each BILLED_STATES as state (state)}
                    <option value={state}>{BILLED_LABELS[state]}</option>
                {/each}
            </select>
        </label>
        <div class="flex min-w-0 grow basis-48 flex-col gap-1 text-sm sm:grow-0" data-testid="time-filter-account">
            <span aria-hidden="true">Account</span>
            <AccountInput bind:value={accountText} accountNames={accounts} placeholder="Any account" label="Account (and below)" onCommit={setAccount} />
        </div>
        {#if isFiltered(query)}
            <button type="button" class="btn btn-ghost btn-sm" onclick={onClear} data-testid="time-filters-clear">Clear filters</button>
        {/if}
    </div>
</div>
