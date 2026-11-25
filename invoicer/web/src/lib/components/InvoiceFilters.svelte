<!-- The invoice list's filter bar (invoicer PLAN 5b, S1–S3): search, status,
     customer, a date range and the age of what's late. It shows the URL's
     state (`query`) and asks for a new one (`onChange`); the page owns the
     URL. Choices are a history step each (Back undoes one); the search text is
     debounced and replaces the current step, so typing doesn't fill history.

     Enter in the search box is the quick jump (S3): the page opens the
     invoice whose number was typed, or the only one the search leaves.

     Narrow windows: the controls wrap, the search taking a whole row below
     `sm` (container queries follow the UI scale, plan 39). -->
<script lang="ts">
    import {untrack} from "svelte";
    import {SearchBox} from "#lib/searchBox.svelte.js";
    import Combobox from "#ui/components/Combobox.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import {AGED_BUCKETS, INVOICE_STATUSES, invoiceFiltered, type InvoiceQuery, type InvoiceStatus} from "#lib/listQuery.js";

    let {
        query,
        customers,
        onChange,
        onJump,
        onClear,
        debounceMs = 250,
    }: {
        query: InvoiceQuery;
        /** Customer names for the picker. */
        customers: readonly string[];
        onChange: (next: InvoiceQuery, options: {replace: boolean}) => void;
        /** Enter in the search box, with what it holds. */
        onJump: (text: string) => void;
        onClear: () => void;
        debounceMs?: number;
    } = $props();

    const STATUS_LABELS: Record<InvoiceStatus, string> = {
        draft: "Draft",
        sent: "Sent",
        paid: "Paid",
        void: "Void",
        open: "Open (overdue too)",
        overdue: "Overdue",
    };

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

    // The customer: typed or picked by name, sent when chosen or committed.
    let customerText = $state(untrack(() => query.customer));
    $effect(() => {
        const name = query.customer;
        untrack(() => {
            if (name !== customerText.trim()) customerText = name;
        });
    });
    const customerOptions = $derived<ComboOption[]>(customers.map((name) => ({value: name, label: name})));

    function setCustomer(name: string): void {
        const wanted = name.trim();
        customerText = wanted;
        if (wanted !== query.customer) onChange({...query, customer: wanted}, {replace: false});
    }

    const set = (patch: Partial<InvoiceQuery>): void => onChange({...query, ...patch}, {replace: false});

    const agedKnown = $derived(AGED_BUCKETS.some((b) => b.value === query.aged));
    const filtered = $derived(invoiceFiltered(query));
</script>

<div role="search" aria-label="Filter invoices" class="flex flex-wrap items-end gap-x-3 gap-y-2" data-testid="invoice-filters">
    <label class="flex min-w-0 grow basis-full flex-col gap-1 text-sm sm:basis-56"
        >Search
        <input
            class="input w-full input-sm"
            type="search"
            placeholder="Number, customer, reference or total"
            value={search.text}
            oninput={(event) => search.type(event.currentTarget.value)}
            onkeydown={searchKey}
            onblur={() => search.flush()}
            aria-describedby="invoice-search-hint"
            data-testid="invoice-search"
        />
        <span id="invoice-search-hint" class="sr-only">Enter opens the invoice with that number, or the only one found.</span>
    </label>
    <label class="flex min-w-0 grow basis-36 flex-col gap-1 text-sm sm:grow-0"
        >Status
        <select
            class="select w-full select-sm"
            value={query.status}
            onchange={(event) => set({status: event.currentTarget.value as InvoiceStatus | ""})}
            data-testid="invoice-filter-status"
        >
            <option value="">Any status</option>
            {#each INVOICE_STATUSES as status (status)}
                <option value={status}>{STATUS_LABELS[status]}</option>
            {/each}
        </select>
    </label>
    <div class="flex min-w-0 grow basis-48 flex-col gap-1 text-sm sm:grow-0">
        <span aria-hidden="true">Customer</span>
        <Combobox
            bind:value={() => customerText, (v) => (customerText = v)}
            options={customerOptions}
            label="Customer"
            placeholder="Any customer"
            onChoose={(option) => setCustomer(option.value)}
            onCommit={() => setCustomer(customerText)}
            testid="invoice-filter-customer"
        />
    </div>
    <label class="flex min-w-0 grow basis-36 flex-col gap-1 text-sm sm:grow-0"
        >Dated from
        <DateInput
            value={query.from}
            class="w-full"
            aria-label="Dated from"
            max={query.to === "" ? undefined : query.to}
            onchange={(event) => set({from: event.currentTarget.value})}
            data-testid="invoice-filter-from"
        />
    </label>
    <label class="flex min-w-0 grow basis-36 flex-col gap-1 text-sm sm:grow-0"
        >to
        <DateInput
            value={query.to}
            class="w-full"
            aria-label="Dated to"
            min={query.from === "" ? undefined : query.from}
            onchange={(event) => set({to: event.currentTarget.value})}
            data-testid="invoice-filter-to"
        />
    </label>
    <label class="flex min-w-0 grow basis-40 flex-col gap-1 text-sm sm:grow-0"
        >Late by
        <select
            class="select w-full select-sm"
            value={query.aged}
            onchange={(event) => set({aged: event.currentTarget.value})}
            data-testid="invoice-filter-aged"
        >
            <option value="">Any</option>
            {#each AGED_BUCKETS as bucket (bucket.value)}
                <option value={bucket.value}>{bucket.label}</option>
            {/each}
            {#if query.aged !== "" && !agedKnown}
                <option value={query.aged}>{query.aged.replace("-", "–")} days late</option>
            {/if}
        </select>
    </label>
    {#if filtered}
        <button type="button" class="btn btn-ghost btn-sm" onclick={onClear} data-testid="invoice-filters-clear">Clear filters</button>
    {/if}
</div>
