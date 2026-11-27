<!-- Time → Entries (plan 46 T-entries): every line of the time book for the
     URL's period and filters, as a timesheet by day (the default), by
     account, or as the weekly grid. Add (`a`), change in place (Enter),
     delete, and act on picked lines in bulk. The URL is the state (the
     invoicer's list contract): a link opens the same view, Back and Forward
     step through it. Again whenever a file changes on disk.

     Every write is followed by a fresh list: a line's id is its place in its
     file, and writes move lines. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {EditorApiError} from "#lib/editorApi.js";
    import {localToday} from "#lib/invoices.js";
    import {countText} from "#lib/listQuery.js";
    import ConfirmDialog from "#lib/components/ConfirmDialog.svelte";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import {addEntry, bulkEntries, deleteEntry, editEntry, type BulkOp, type EntryEdit, type EntryView, type NewEntry} from "#lib/time/api/entries.js";
    import {fetchAccounts, type AccountsView} from "#lib/time/api/accounts.js";
    import {pickerAccounts, customersOf} from "#lib/time/accounts/tree.js";
    import BulkBar from "#lib/time/entries/BulkBar.svelte";
    import EntryFilters from "#lib/time/entries/EntryFilters.svelte";
    import EntryForm from "#lib/time/entries/EntryForm.svelte";
    import EntryTable from "#lib/time/entries/EntryTable.svelte";
    import WeekGrid from "#lib/time/entries/WeekGrid.svelte";
    import {EntriesStore, type WriteOutcome} from "#lib/time/entries/entriesStore.svelte.js";
    import {addDays, mondayOf, shortDate} from "#lib/time/entries/dates.js";
    import {formatDuration} from "#lib/time/entries/duration.js";
    import {groupByAccount, groupByDay, overDayWarnings, planCellEdit, weekGrid, type WeekCell} from "#lib/time/entries/group.js";
    import {
        clearEntriesFilters,
        entriesApiQuery,
        entriesFiltered,
        entriesQueryString,
        filterPerson,
        parseEntriesQuery,
        shownWeek,
        type EntriesQuery,
        type Grouping,
    } from "#lib/time/entries/query.js";

    const today = localToday();
    const query = $derived(parseEntriesQuery(page.url.searchParams));
    const queryString = $derived(entriesQueryString(query));
    const apiQuery = $derived(entriesApiQuery(query, today));
    const filtered = $derived(entriesFiltered(query));

    const store = new EntriesStore();
    let accountsView = $state<AccountsView | null>(null);

    $effect(() => {
        const qs = apiQuery;
        untrack(() => void store.load(qs));
    });

    function loadAccounts(): void {
        fetchAccounts()
            .then((view) => (accountsView = view))
            .catch((e: unknown) => console.warn("time accounts:", e));
    }
    onMount(loadAccounts);
    onFilesChanged(() => {
        void store.load();
        loadAccounts();
    });

    const accounts = $derived(accountsView === null ? [] : pickerAccounts(accountsView));
    const customers = $derived(accountsView === null ? [] : customersOf(accountsView));
    const notSetUp = $derived(store.error instanceof EditorApiError && store.error.code === "not-set-up");

    function setQuery(next: EntriesQuery, options: {replace: boolean} = {replace: false}): void {
        const qs = entriesQueryString(next);
        if (qs === queryString) return;
        const base = resolve("/time/entries");
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
        void goto(qs === "" ? base : `${base}?${qs}`, {replace: options.replace, reset: false});
    }

    // ---- Views
    const entries = $derived(store.list?.entries ?? []);
    const groups = $derived(query.group === "account" ? groupByAccount(entries) : groupByDay(entries));
    const monday = $derived(shownWeek(query, today));
    /** Rows added to the week grid with no time yet; dropped when the week changes. */
    let extraRows = $state<string[]>([]);
    $effect(() => {
        void monday;
        untrack(() => (extraRows = []));
    });
    const grid = $derived(weekGrid(entries, monday, extraRows));

    const dayWarnings = $derived(overDayWarnings(entries));
    const findings = $derived((store.list?.findings ?? []).filter((f) => f.severity !== "info"));

    // ---- Add and edit
    let adding = $state(false);
    let editing = $state<string | null>(null);
    let lockedNotice = $state<string | null>(null);

    function startAdd(): void {
        editing = null;
        lockedNotice = null;
        adding = true;
    }

    function startEdit(entry: EntryView): void {
        adding = false;
        if (entry.locked !== null) {
            editing = null;
            lockedNotice = lockedNotice === entry.id ? null : entry.id;
            return;
        }
        lockedNotice = null;
        editing = entry.id;
    }

    async function add(entry: NewEntry): Promise<WriteOutcome> {
        const outcome = await store.write(() => addEntry(entry));
        if (outcome.ok) adding = false;
        return outcome;
    }

    async function edit(change: EntryEdit): Promise<WriteOutcome> {
        const outcome = await store.write(() => editEntry(change));
        if (outcome.ok) editing = null;
        return outcome;
    }

    // ---- Delete
    let deleting = $state<EntryView | null>(null);
    let deleteBusy = $state(false);
    let deleteError = $state<string | null>(null);

    async function confirmDelete(): Promise<void> {
        if (deleting === null) return;
        deleteBusy = true;
        const id = deleting.id;
        const outcome = await store.write(() => deleteEntry(id));
        deleteBusy = false;
        if (outcome.ok) deleting = null;
        else deleteError = outcome.message;
    }

    // ---- Bulk
    let bulkBusy = $state(false);
    let bulkError = $state<string | null>(null);
    const picked = $derived(entries.filter((e) => store.selected.has(e.id)));

    async function applyBulk(op: BulkOp): Promise<void> {
        bulkBusy = true;
        bulkError = null;
        const ids = picked.map((e) => e.id);
        const outcome = await store.write(() => bulkEntries(ids, op));
        bulkBusy = false;
        if (!outcome.ok) bulkError = outcome.message;
    }

    // ---- The week grid's cells
    async function setCell(account: string, cell: WeekCell, seconds: number): Promise<string | null> {
        const plan = planCellEdit(cell, account, seconds);
        const person = filterPerson(query);
        let outcome: WriteOutcome = {ok: true};
        switch (plan.kind) {
            case "none":
                return null;
            case "refuse":
                return plan.reason;
            case "add":
                outcome = await store.write(() =>
                    addEntry({date: plan.date, account: plan.account, duration: plan.duration, ...(person === undefined ? {} : {person})})
                );
                break;
            case "edit":
                outcome = await store.write(() => editEntry({id: plan.id, duration: plan.duration}));
                break;
            case "delete":
                outcome = await store.write(() => deleteEntry(plan.id));
                break;
        }
        return outcome.ok ? null : outcome.message;
    }

    function addRow(account: string): void {
        if (!extraRows.includes(account)) extraRows = [...extraRows, account];
    }

    /** Show the week holding `date`; this week is written as no `week`. */
    const setWeek = (date: string): void => setQuery({...query, week: mondayOf(date) === mondayOf(today) ? "" : mondayOf(date)});
    const setGroup = (group: Grouping): void => setQuery({...query, group});

    // ---- Keys
    registerKeys({
        id: "time-entries-page",
        priority: PRIORITY.page,
        bindings: [
            {keys: "a", label: "Add time", group: "Time entries", enabled: () => store.list !== null && !adding, run: startAdd},
            {
                keys: "Escape",
                label: "Clear the picked entries",
                group: "Time entries",
                enabled: () => store.selected.size > 0,
                run: () => store.selected.clear(),
            },
        ],
    });

    const GROUP_LABELS: Record<Grouping, string> = {day: "By day", week: "Week", account: "By account"};
    const countLine = $derived(store.list === null ? "" : `${countText(entries.length, "entry", "entries")} · ${store.list.total.text}`);
</script>

<svelte:head>
    <title>Time entries · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-6xl flex-col gap-4">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">Time entries</h1>
        <div class="flex flex-wrap items-center gap-2">
            <div class="join" role="group" aria-label="Show">
                {#each ["day", "week", "account"] as const as group (group)}
                    <button
                        type="button"
                        class={["btn join-item btn-sm", query.group === group ? "btn-active" : "btn-ghost"]}
                        aria-pressed={query.group === group}
                        onclick={() => setGroup(group)}
                        data-testid="entries-group-{group}">{GROUP_LABELS[group]}</button
                    >
                {/each}
            </div>
            {#if store.list !== null && query.group !== "week"}
                <button type="button" class="btn btn-primary btn-sm" onclick={startAdd} disabled={adding} data-testid="entries-add">Add time</button>
            {/if}
        </div>
    </div>

    {#if !notSetUp}
        <EntryFilters
            {query}
            {today}
            people={[...store.people]}
            {customers}
            {accounts}
            onChange={setQuery}
            onClear={() => setQuery(clearEntriesFilters(query))}
        />
    {/if}

    {#if query.group === "week"}
        <div class="flex flex-wrap items-center gap-2" data-testid="week-nav">
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => setWeek(addDays(monday, -7))} aria-label="Previous week">←</button>
            <span class="font-medium" data-testid="week-label">Week of {shortDate(monday)}, {monday.slice(0, 4)}</span>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => setWeek(addDays(monday, 7))} aria-label="Next week">→</button>
            {#if query.week !== ""}
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => setWeek(today)}>This week</button>
            {/if}
        </div>
    {/if}

    {#if adding && query.group !== "week"}
        <EntryForm {today} {accounts} person={filterPerson(query)} onAdd={add} onCancel={() => (adding = false)} testid="entry-add" />
    {/if}

    {#if findings.length > 0 || dayWarnings.length > 0}
        <section class="alert flex-col items-start gap-1 alert-warning" aria-label="Warnings" data-testid="entries-warnings">
            {#each dayWarnings as warning (`${warning.date}-${warning.person}`)}
                <p class="text-sm" data-testid="entries-day-warning">
                    {warning.person ?? "Someone not named"} has {formatDuration(warning.seconds)} on {warning.date}, more than a day holds.
                </p>
            {/each}
            {#each findings as finding (`${finding.file}:${finding.line}:${finding.message}`)}
                <p class="text-sm" data-testid="entries-finding">
                    {finding.message} <span class="text-xs opacity-70">({finding.file}, line {finding.line})</span>
                </p>
            {/each}
        </section>
    {/if}

    {#if picked.length > 0}
        <BulkBar
            count={picked.length}
            seconds={picked.reduce((t, e) => t + e.quantity.seconds, 0)}
            {accounts}
            busy={bulkBusy}
            error={bulkError}
            onApply={(op) => void applyBulk(op)}
            onClear={() => store.selected.clear()}
        />
    {/if}

    {#if notSetUp}
        <EmptyState
            title="Time tracking isn't set up"
            text="These books have no time files yet. Set up time tracking from Track in the sidebar, then your time shows here."
            testid="entries-not-set-up"
        />
    {:else}
        <AsyncSection
            view={store.view}
            value={store.list}
            error={store.error}
            testid="entries-error"
            label="the time entries"
            loadingLabel="Loading time"
            onRetry={() => void store.retry()}
        >
            {#snippet children(list)}
                {#if query.group === "week"}
                    <WeekGrid {grid} {today} {accounts} onCell={setCell} onAddRow={addRow} />
                {:else if list.entries.length === 0 && filtered}
                    <div class="rounded-box border border-base-300 px-4 py-8 text-center" data-testid="entries-no-match">
                        <p class="text-base-content/70">No time matches these filters.</p>
                        <button type="button" class="btn mt-3 btn-outline btn-sm" onclick={() => setQuery(clearEntriesFilters(query))}>Clear filters</button>
                    </div>
                {:else if list.entries.length === 0}
                    {#if !adding}
                        <EmptyState
                            title="No time in this period"
                            text="Time you track or add shows here, a day at a time. Pick another period above, or add time by hand."
                            action="Add time"
                            onAction={startAdd}
                            testid="entries-empty"
                            actionTestid="entries-empty-add"
                        />
                    {/if}
                {:else}
                    <p class="-mb-2 text-sm text-base-content/70" role="status" data-testid="entries-count">{countLine}</p>
                    <EntryTable
                        {groups}
                        grouping={query.group === "account" ? "account" : "day"}
                        selected={store.selected}
                        {editing}
                        {lockedNotice}
                        onEdit={startEdit}
                        onDelete={(entry) => {
                            deleteError = null;
                            deleting = entry;
                        }}
                        onToggle={(entry) => store.toggle(entry.id)}
                        onToggleRange={(ids) => store.toggleRange(ids)}
                    >
                        {#snippet editor(entry)}
                            <EntryForm {entry} {today} {accounts} onEdit={edit} onCancel={() => (editing = null)} testid="entry-edit-form" />
                        {/snippet}
                    </EntryTable>
                {/if}
            {/snippet}
        </AsyncSection>
    {/if}
</div>

{#if deleting !== null}
    <ConfirmDialog
        title="Delete this time?"
        confirmLabel="Delete"
        tone="error"
        busy={deleteBusy}
        error={deleteError}
        onConfirm={() => void confirmDelete()}
        onCancel={() => (deleting = null)}
        testid="entry-delete-dialog"
    >
        <p>
            {deleting.quantity.text} on {deleting.account}, {deleting.date}{deleting.note === "" ? "" : ` (“${deleting.note}”)`}. The line is taken out of {deleting.file}.
        </p>
    </ConfirmDialog>
{/if}
