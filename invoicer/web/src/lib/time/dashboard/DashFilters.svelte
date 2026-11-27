<!-- The time dashboard's filter bar (plan 46 T-dash): the period (presets,
     custom dates, previous/next), compare with the previous period, person
     (hidden when only one person has time), customer (hidden without any),
     billable, account depth and the account filter. It shows the URL's
     state (`query`) and asks for a new one (`onChange`); the page owns the
     URL. Choices are a history step each; the account text is debounced and
     replaces the current step, so typing doesn't fill history.

     Keyboard: everything is a native control in tab order; `/` focuses the
     account filter (Escape clears it, a second Escape leaves it); `[` and
     `]` step the period (the page's key layer). Narrow windows: the
     controls wrap, the account filter taking a whole row below `sm`. -->
<script lang="ts">
    import {untrack} from "svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import {searchKeys} from "#lib/keys/pageSearch.svelte.js";
    import {SearchBox} from "#lib/searchBox.svelte.js";
    import {rangeText} from "../dates.js";
    import type {DashMeta} from "./meta.js";
    import {isFiltered, PRESET_LABELS, PRESETS, UNATTRIBUTED, type DashQuery, type Preset, type Range} from "./query.js";

    let {
        query,
        meta,
        range,
        previous,
        onChange,
        onStep,
        onClear,
        debounceMs = 250,
    }: {
        query: DashQuery;
        meta: DashMeta;
        range: Range;
        /** The comparison's dates; null with comparison off. */
        previous: Range | null;
        onChange: (next: DashQuery, options: {replace: boolean}) => void;
        onStep: (direction: -1 | 1) => void;
        onClear: () => void;
        debounceMs?: number;
    } = $props();

    const custom = $derived(query.from !== "" || query.to !== "");
    const set = (patch: Partial<DashQuery>): void => onChange({...query, ...patch}, {replace: false});

    function setPeriod(value: string): void {
        if (value === "custom") set({from: range.from, to: range.to});
        else set({period: value as Preset, from: "", to: ""});
    }

    // The account filter: a prefix, sent after a pause, as the invoice list's search is.
    const account = new SearchBox(
        untrack(() => query.account),
        (text) => onChange({...query, account: text}, {replace: true}),
        untrack(() => debounceMs)
    );
    $effect(() => {
        const text = query.account;
        untrack(() => account.sync(text));
    });
    $effect(() => () => account.dispose());

    function accountKey(event: KeyboardEvent): void {
        if (event.key !== "Enter") return;
        event.preventDefault();
        account.flush();
    }

    const showPeople = $derived(meta.people.length > 1 || query.person !== "");
    const depths = $derived(Array.from({length: Math.max(meta.maxDepth, query.depth)}, (_, i) => i + 1));
    const id = $props.id();
</script>

<div role="search" aria-label="Filter time" class="flex flex-col gap-2" data-testid="time-dash-filters">
    <div class="flex flex-wrap items-end gap-x-3 gap-y-2">
        <div class="flex min-w-0 flex-col gap-1 text-sm">
            <label for="{id}-period">Period</label>
            <div class="join">
                <button
                    type="button"
                    class="btn join-item btn-sm"
                    aria-label="Previous period"
                    title="Previous period ([)"
                    onclick={() => onStep(-1)}
                    data-testid="time-dash-prev">‹</button
                >
                <select
                    id="{id}-period"
                    class="select join-item w-40 select-sm"
                    value={custom ? "custom" : query.period}
                    onchange={(event) => setPeriod(event.currentTarget.value)}
                    data-testid="time-dash-period"
                >
                    {#each PRESETS as preset (preset)}
                        <option value={preset}>{PRESET_LABELS[preset]}</option>
                    {/each}
                    <option value="custom">Custom…</option>
                </select>
                <button
                    type="button"
                    class="btn join-item btn-sm"
                    aria-label="Next period"
                    title="Next period (])"
                    onclick={() => onStep(1)}
                    data-testid="time-dash-next">›</button
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
                    data-testid="time-dash-from"
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
                    data-testid="time-dash-to"
                />
            </label>
        {/if}
        <label class="flex items-center gap-2 self-end pb-1 text-sm">
            <input
                type="checkbox"
                class="checkbox checkbox-sm"
                checked={query.compare}
                onchange={(event) => set({compare: event.currentTarget.checked})}
                data-testid="time-dash-compare"
            />
            Compare with previous
        </label>
    </div>
    <p class="text-sm text-base-content/70" data-testid="time-dash-range">
        {`${rangeText(range.from, range.end)}${range.end !== range.to ? " so far" : ""}`}
        {#if previous !== null}<span class="text-base-content/60">{` · compared with ${rangeText(previous.from, previous.to)}`}</span>{/if}
    </p>
    <div class="flex flex-wrap items-end gap-x-3 gap-y-2">
        {#if showPeople}
            <label class="flex min-w-0 grow basis-32 flex-col gap-1 text-sm sm:grow-0"
                >Person
                <select
                    class="select w-full select-sm"
                    value={query.person}
                    onchange={(event) => set({person: event.currentTarget.value})}
                    data-testid="time-dash-person"
                >
                    <option value="">Everyone</option>
                    {#each meta.people as person (person)}
                        {#if person === ""}
                            <option value={UNATTRIBUTED}>Unattributed</option>
                        {:else}
                            <option value={person}>{person}</option>
                        {/if}
                    {/each}
                    {#if query.person !== "" && query.person !== UNATTRIBUTED && !meta.people.includes(query.person)}
                        <option value={query.person}>{query.person}</option>
                    {/if}
                </select>
            </label>
        {/if}
        {#if meta.customers.length > 0 || query.customer !== ""}
            <label class="flex min-w-0 grow basis-40 flex-col gap-1 text-sm sm:grow-0"
                >Customer
                <select
                    class="select w-full select-sm"
                    value={query.customer}
                    onchange={(event) => set({customer: event.currentTarget.value})}
                    data-testid="time-dash-customer"
                >
                    <option value="">Any customer</option>
                    {#each meta.customers as customer (customer)}
                        <option value={customer}>{customer}</option>
                    {/each}
                    {#if query.customer !== "" && !meta.customers.includes(query.customer)}
                        <option value={query.customer}>{query.customer}</option>
                    {/if}
                </select>
            </label>
        {/if}
        <label class="flex min-w-0 grow basis-32 flex-col gap-1 text-sm sm:grow-0"
            >Billable
            <select
                class="select w-full select-sm"
                value={query.billable}
                onchange={(event) => set({billable: event.currentTarget.value as DashQuery["billable"]})}
                data-testid="time-dash-billable"
            >
                <option value="">All time</option>
                <option value="yes">Billable</option>
                <option value="no">Non-billable</option>
            </select>
        </label>
        <label class="flex min-w-0 basis-24 flex-col gap-1 text-sm"
            >Depth
            <select
                class="select w-full select-sm"
                value={String(query.depth)}
                onchange={(event) => set({depth: Number(event.currentTarget.value)})}
                data-testid="time-dash-depth"
            >
                {#each depths as depth (depth)}
                    <option value={String(depth)}>{depth}</option>
                {/each}
            </select>
        </label>
        <label class="flex min-w-0 grow basis-full flex-col gap-1 text-sm sm:basis-56"
            >Account
            <input
                class="input w-full input-sm"
                type="search"
                placeholder="clients:acme, and below"
                list="{id}-accounts"
                value={account.text}
                oninput={(event) => account.type(event.currentTarget.value)}
                onkeydown={accountKey}
                onblur={() => account.flush()}
                use:searchKeys={{clear: () => account.clear()}}
                data-testid="time-dash-account"
            />
            <datalist id="{id}-accounts">
                {#each meta.accounts as name (name)}
                    <option value={name}></option>
                {/each}
            </datalist>
        </label>
        {#if isFiltered(query)}
            <button type="button" class="btn btn-ghost btn-sm" onclick={onClear} data-testid="time-dash-clear">Clear filters</button>
        {/if}
    </div>
</div>
