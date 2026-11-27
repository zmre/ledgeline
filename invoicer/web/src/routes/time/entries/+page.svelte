<!-- Time → Entries (plan 46 T-entries): every line of the time book for the
     URL's period and filters, as a timesheet by day (the default), by
     account, or as the weekly grid. The second of the Time area's two tabs
     (`TimeTabs`), with Overview; both share one filter bar
     (`TimeFilterBar`, `#lib/time/filters/query.ts`), so a tab switch keeps
     the filters, and `[`/`]` step the period (the week, in the grid). Add (`a`), change in place (Enter),
     delete, and act on picked lines in bulk. The URL is the state (the
     invoicer's list contract): a link opens the same view, Back and Forward
     step through it. Again whenever a file changes on disk.

     Every write is followed by a fresh list: a line's id is its place in its
     file, and writes move lines.

     Bill… (or `b`) with lines picked opens the Bill dialog for exactly those
     lines (plan 46 B-invoice); they must be one customer's unbilled time, or
     the bar says why not. The draft it makes opens in the invoice editor. -->
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
    import {releaseBill, type BillCreated} from "#lib/time/api/billing.js";
    import BillDialog from "#lib/time/billing/BillDialog.svelte";
    import {entriesBillTarget} from "#lib/time/billing/words.js";
    import BulkBar from "#lib/time/entries/BulkBar.svelte";
    import EntryForm from "#lib/time/entries/EntryForm.svelte";
    import EntryTable from "#lib/time/entries/EntryTable.svelte";
    import WeekGrid from "#lib/time/entries/WeekGrid.svelte";
    import {EntriesStore, type WriteOutcome} from "#lib/time/entries/entriesStore.svelte.js";
    import {addDays, mondayOf, dayMonth, dayMonthYear, rangeText} from "#lib/time/dates.js";
    import {formatDuration} from "#lib/time/duration.js";
    import {groupByAccount, groupByDay, overDayWarnings, planCellEdit, weekGrid, type WeekCell} from "#lib/time/entries/group.js";
    import {entriesApiQuery, entriesQueryString, parseEntriesQuery, shownRange, shownWeek, type EntriesQuery, type Grouping} from "#lib/time/entries/query.js";
    import {
        clearFilters,
        filterPerson,
        filtersOf,
        filtersQueryString,
        isAllTime,
        isFiltered,
        resolveRange,
        stepPeriod,
        withQuery,
    } from "#lib/time/filters/query.js";
    import TimeFilterBar from "#lib/time/filters/TimeFilterBar.svelte";
    import TimeTabs from "#lib/time/filters/TimeTabs.svelte";

    const today = localToday();
    const query = $derived(parseEntriesQuery(page.url.searchParams));
    const apiQuery = $derived(entriesApiQuery(query, today));
    const filtered = $derived(isFiltered(query));
    const tabQuery = $derived(filtersQueryString(filtersOf(query)));

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

    // The URL changes only once a navigation lands, so a second `[` pressed
    // before then would step from the old period. `pending` is the query the
    // last navigation is heading to; steps and choices build on it.
    let pending: EntriesQuery | null = null;
    const current = (): EntriesQuery => pending ?? query;

    function setQuery(next: EntriesQuery, options: {replace: boolean} = {replace: false}): void {
        const qs = entriesQueryString(next);
        if (qs === entriesQueryString(current())) return;
        pending = next;
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
        void Promise.resolve(goto(withQuery(resolve("/time/entries"), qs), {replace: options.replace, reset: false})).finally(() => {
            if (pending === next) pending = null;
        });
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

    // ---- Bill the picked lines
    let billing = $state<{customer: string; entries: string[]} | null>(null);

    function billPicked(): void {
        const target = entriesBillTarget(picked);
        if ("error" in target) {
            bulkError = target.error;
            return;
        }
        bulkError = null;
        billing = target;
    }

    function billed(created: BillCreated): void {
        billing = null;
        store.selected.clear();
        void goto(resolve("/invoices/[number]/edit", {number: created.number}), {state: {createWarnings: created.warnings}});
    }

    // ---- Void a booked bill and release its time
    let releasing = $state<EntryView | null>(null);
    let releaseBusy = $state(false);
    let releaseError = $state<string | null>(null);

    async function confirmReleaseBill(): Promise<void> {
        const bill = releasing?.bill ?? null;
        if (bill === null) return;
        releaseBusy = true;
        releaseError = null;
        const outcome = await store.write(() => releaseBill(bill));
        releaseBusy = false;
        if (outcome.ok) {
            releasing = null;
            lockedNotice = null;
        } else releaseError = outcome.message;
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

    /** `[`/`]`: the week in the grid, else the period (All time doesn't step). */
    function step(direction: -1 | 1): void {
        const from = current();
        if (from.group === "week") {
            const monday = addDays(shownWeek(from, today), 7 * direction);
            setQuery({...from, week: monday === mondayOf(today) ? "" : monday});
        } else setQuery(stepPeriod(from, today, direction));
    }
    const canStep = (): boolean => current().group === "week" || !isAllTime(current());

    /** The dates shown, for the line under the period. */
    const rangeLine = $derived.by(() => {
        const r = shownRange(query, today);
        if (r.from !== null && r.to !== null) return rangeText(r.from, r.to);
        if (r.from !== null) return `From ${dayMonthYear(r.from)}`;
        if (r.to !== null) return `To ${dayMonthYear(r.to)}`;
        return "All time";
    });

    // ---- Keys
    registerKeys({
        id: "time-entries-page",
        priority: PRIORITY.page,
        bindings: [
            {keys: "[", label: "Previous period", group: "Time", enabled: canStep, run: () => step(-1)},
            {keys: "]", label: "Next period", group: "Time", enabled: canStep, run: () => step(1)},
            {keys: "a", label: "Add time", group: "Time entries", enabled: () => store.list !== null && !adding, run: startAdd},
            {keys: "b", label: "Bill the picked time…", group: "Time entries", enabled: () => picked.length > 0 && billing === null, run: billPicked},
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
    <TimeTabs current="entries" overviewHref={withQuery(resolve("/time"), tabQuery)} entriesHref={`${page.url.pathname}${page.url.search}`}>
        {#snippet actions()}
            {#if store.list !== null && query.group !== "week"}
                <button type="button" class="btn btn-primary btn-sm" onclick={startAdd} disabled={adding} data-testid="entries-add">Add time</button>
            {/if}
        {/snippet}
    </TimeTabs>

    {#if !notSetUp}
        <TimeFilterBar
            {query}
            range={resolveRange(query, today)}
            people={[...store.people].sort()}
            {customers}
            {accounts}
            showPeriod={query.group !== "week"}
            onChange={(next, options) => setQuery({...query, ...next}, options)}
            onStep={isAllTime(query) ? null : step}
            onClear={() => setQuery(clearFilters(query))}
        >
            {#snippet options()}
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
            {/snippet}
            {#snippet status()}
                {#if query.group !== "week"}
                    <p class="text-sm text-base-content/70" data-testid="entries-range">{rangeLine}</p>
                {/if}
            {/snippet}
        </TimeFilterBar>
    {/if}

    {#if query.group === "week"}
        <div class="flex flex-wrap items-center gap-2" data-testid="week-nav">
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => setWeek(addDays(monday, -7))} aria-label="Previous week">←</button>
            <span class="font-medium" data-testid="week-label">Week of {dayMonth(monday)}, {monday.slice(0, 4)}</span>
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
            onBill={billPicked}
        />
    {/if}

    {#if notSetUp}
        <EmptyState
            title="Time tracking isn't set up"
            text="These books have no time files yet. Set up time tracking, then your time shows here."
            action="Set up time tracking"
            href={resolve("/time/track")}
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
                        <button type="button" class="btn mt-3 btn-outline btn-sm" onclick={() => setQuery(clearFilters(query))}>Clear filters</button>
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
                        onReleaseBill={(entry) => {
                            releaseError = null;
                            releasing = entry;
                        }}
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

{#if billing !== null}
    <BillDialog customer={billing.customer} name={billing.customer} entries={billing.entries} onCreated={billed} onCancel={() => (billing = null)} />
{/if}

{#if releasing !== null}
    <ConfirmDialog
        title="Void the bill “{releasing.bill}”?"
        confirmLabel="Void bill and release time"
        tone="error"
        busy={releaseBusy}
        error={releaseError}
        onConfirm={() => void confirmReleaseBill()}
        onCancel={() => (releasing = null)}
        testid="entry-release-bill-dialog"
    >
        <p>
            It writes a reversing entry for the bill in the books, so nothing is owed on it, and its time is unpaid again: every line on it can be changed and
            billed anew.
        </p>
    </ConfirmDialog>
{/if}

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
