<!-- Entries' filter bar (T-entries, the invoicer's URL contract): search, the
     period (a preset or a custom range), person, customer, billable, billing
     state and account. It shows the URL's state (`query`) and asks for a new
     one (`onChange`); the page owns the URL. Choices are a history step each;
     the search text is debounced and replaces the current step.

     The week grid picks its own week, so the period hides while it shows.
     Narrow widths: the controls wrap, the search taking a whole row below
     `sm`. Keyboard: `/` focuses the search; Escape in it clears it, a second
     Escape leaves it. -->
<script lang="ts">
    import {untrack} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import Combobox from "#ui/components/Combobox.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import {searchKeys} from "#lib/keys/pageSearch.svelte.js";
    import {SearchBox} from "#lib/searchBox.svelte.js";
    import {
        BILLED_LABELS,
        BILLED_STATES,
        PERIOD_LABELS,
        PERIODS,
        UNATTRIBUTED,
        entriesFiltered,
        periodRange,
        type BilledState,
        type EntriesQuery,
        type Period,
    } from "./query.js";

    let {
        query,
        today,
        people,
        customers,
        accounts,
        onChange,
        onClear,
        debounceMs = 250,
    }: {
        query: EntriesQuery;
        today: string;
        people: readonly string[];
        customers: readonly string[];
        accounts: string[];
        onChange: (next: EntriesQuery, options: {replace: boolean}) => void;
        onClear: () => void;
        debounceMs?: number;
    } = $props();

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

    const set = (patch: Partial<EntriesQuery>): void => onChange({...query, ...patch}, {replace: false});

    const custom = $derived(query.from !== "" || query.to !== "");

    function setPeriod(value: string): void {
        if (value === "custom") {
            const range = periodRange(query.period, today);
            set({from: range.from ?? today, to: range.to ?? today});
        } else {
            set({period: value as Period, from: "", to: ""});
        }
    }

    // Customer and account: typed or picked, sent when chosen or committed.
    let customerText = $state(untrack(() => query.customer));
    let accountText = $state(untrack(() => query.account));
    $effect(() => {
        const name = query.customer;
        untrack(() => {
            if (name !== customerText.trim()) customerText = name;
        });
    });
    $effect(() => {
        const name = query.account;
        untrack(() => {
            if (name !== accountText.trim()) accountText = name;
        });
    });
    const customerOptions = $derived<ComboOption[]>(customers.map((name) => ({value: name, label: name})));

    function setCustomer(name: string): void {
        customerText = name.trim();
        if (customerText !== query.customer) set({customer: customerText});
    }

    function setAccount(): void {
        const wanted = accountText.trim();
        if (wanted !== query.account) set({account: wanted});
    }

    const peopleChoices = $derived([...new Set([...people, ...(query.person !== "" && query.person !== UNATTRIBUTED ? [query.person] : [])])].sort());
    const filtered = $derived(entriesFiltered(query));
</script>

<div role="search" aria-label="Filter time entries" class="flex flex-wrap items-end gap-x-3 gap-y-2" data-testid="entry-filters">
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
            data-testid="entry-search"
        />
    </label>
    {#if query.group !== "week"}
        <label class="flex min-w-0 grow basis-36 flex-col gap-1 text-sm sm:grow-0"
            >Period
            <select
                class="select w-full select-sm"
                value={custom ? "custom" : query.period}
                onchange={(event) => setPeriod(event.currentTarget.value)}
                data-testid="entry-filter-period"
            >
                {#each PERIODS as period (period)}
                    <option value={period}>{PERIOD_LABELS[period]}</option>
                {/each}
                <option value="custom">Custom…</option>
            </select>
        </label>
        {#if custom}
            <label class="flex min-w-0 grow basis-36 flex-col gap-1 text-sm sm:grow-0"
                >From
                <DateInput
                    value={query.from}
                    class="w-full"
                    max={query.to === "" ? undefined : query.to}
                    onchange={(event) => set({from: event.currentTarget.value})}
                    data-testid="entry-filter-from"
                />
            </label>
            <label class="flex min-w-0 grow basis-36 flex-col gap-1 text-sm sm:grow-0"
                >to
                <DateInput
                    value={query.to}
                    class="w-full"
                    min={query.from === "" ? undefined : query.from}
                    onchange={(event) => set({to: event.currentTarget.value})}
                    data-testid="entry-filter-to"
                />
            </label>
        {/if}
    {/if}
    <label class="flex min-w-0 grow basis-32 flex-col gap-1 text-sm sm:grow-0"
        >Person
        <select
            class="select w-full select-sm"
            value={query.person}
            onchange={(event) => set({person: event.currentTarget.value})}
            data-testid="entry-filter-person"
        >
            <option value="">Anyone</option>
            {#each peopleChoices as person (person)}
                <option value={person}>{person}</option>
            {/each}
            <option value={UNATTRIBUTED}>No one named</option>
        </select>
    </label>
    <div class="flex min-w-0 grow basis-40 flex-col gap-1 text-sm sm:grow-0">
        <span aria-hidden="true">Customer</span>
        <Combobox
            bind:value={() => customerText, (v) => (customerText = v)}
            options={customerOptions}
            label="Customer"
            placeholder="Any customer"
            onChoose={(option) => setCustomer(option.value)}
            onCommit={() => setCustomer(customerText)}
            testid="entry-filter-customer"
        />
    </div>
    <label class="flex min-w-0 grow basis-28 flex-col gap-1 text-sm sm:grow-0"
        >Billable
        <select
            class="select w-full select-sm"
            value={query.billable}
            onchange={(event) => set({billable: event.currentTarget.value as EntriesQuery["billable"]})}
            data-testid="entry-filter-billable"
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
            data-testid="entry-filter-billed"
        >
            <option value="">Any</option>
            {#each BILLED_STATES as state (state)}
                <option value={state}>{BILLED_LABELS[state]}</option>
            {/each}
        </select>
    </label>
    <div class="flex min-w-0 grow basis-48 flex-col gap-1 text-sm sm:grow-0" data-testid="entry-filter-account">
        <span aria-hidden="true">Account</span>
        <AccountInput bind:value={accountText} accountNames={accounts} placeholder="Any account" label="Account (and below)" onCommit={setAccount} />
    </div>
    {#if filtered}
        <button type="button" class="btn btn-ghost btn-sm" onclick={onClear} data-testid="entry-filters-clear">Clear filters</button>
    {/if}
</div>
