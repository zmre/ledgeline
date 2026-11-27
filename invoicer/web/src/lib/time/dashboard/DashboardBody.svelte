<!-- The time dashboard's figures (plan 46 T-dash, W2), top to bottom: the
     stat tiles, hours over time, the account tree, then the top-ten lists
     (accounts at the chosen depth, customers, people, notes). Everything is
     computed here from the period's fact table (`model.ts`), so the split,
     grain, depth and target ask the server for nothing. Every figure links
     into Entries with the page's filters (`entriesHref`), and every section
     carries the hledger command behind it.

     The page is a container: the lists go two-up at `@3xl` of its own
     width, which follows the UI scale. -->
<script lang="ts">
    import type {Summary} from "../api/summary.js";
    import AccountTreeTable from "./AccountTreeTable.svelte";
    import HoursChart from "./HoursChart.svelte";
    import KpiTiles from "./KpiTiles.svelte";
    import TopList from "./TopList.svelte";
    import {
        billableCommand,
        chartCommand,
        dailyCommand,
        notesCommand,
        topAccountsCommand,
        topCustomersCommand,
        topPeopleCommand,
        totalCommand,
        treeCommand,
        unbilledCommand,
    } from "./hledger.js";
    import type {DashMeta} from "./meta.js";
    import {accountsAt, byCustomer, byPerson, chartOf, kpisOf, notesOf, splitDrill, splitLabel, topOf, treeOf, withTime, type Fact} from "./model.js";
    import {withQuery, type Range} from "../filters/query.js";
    import {entriesQuery, grainOf, type DashQuery, type Drill, type Split} from "./query.js";

    let {
        query,
        range,
        previous,
        current,
        prior,
        notes,
        priorNotes,
        meta,
        file,
        entriesBase,
        onQuery,
    }: {
        query: DashQuery;
        range: Range;
        /** The comparison's dates; null with comparison off. */
        previous: Range | null;
        current: readonly Fact[];
        /** The previous period's facts; null with comparison off. */
        prior: readonly Fact[] | null;
        notes: Summary;
        priorNotes: Summary | null;
        meta: DashMeta;
        /** The roll-up, for the hledger commands. */
        file: string;
        /** Entries' URL, resolved (`/app/time/entries`). */
        entriesBase: string;
        onQuery: (next: DashQuery, options: {replace: boolean}) => void;
    } = $props();

    const grain = $derived(grainOf(query, range));
    const kpis = $derived(kpisOf(current, range));
    const priorKpis = $derived(prior === null || previous === null ? null : kpisOf(prior, previous));

    const ORDER: Record<Split, (m: DashMeta) => readonly string[]> = {
        branch: (m) => m.branchOrder,
        customer: (m) => m.customerOrder,
        person: (m) => m.people,
        billable: () => [],
    };
    const chart = $derived(chartOf(current, range, grain, query.split, ORDER[query.split](meta)));
    const tree = $derived(withTime(treeOf(current, prior ?? [])));

    const onePerson = $derived(meta.people.length <= 1 || query.person !== "");
    const topAccounts = $derived(
        topOf(accountsAt(current, query.depth), prior === null ? null : accountsAt(prior, query.depth), {drill: (account) => ({account})})
    );
    const topCustomers = $derived(
        topOf(byCustomer(current), prior === null ? null : byCustomer(prior), {
            label: (k) => splitLabel("customer", k),
            drill: (k) => splitDrill("customer", k),
        })
    );
    const topPeople = $derived(
        topOf(byPerson(current), prior === null ? null : byPerson(prior), {label: (k) => splitLabel("person", k), drill: (k) => ({person: k})})
    );
    const topNotes = $derived(topOf(notesOf(notes.rows), priorNotes === null ? null : notesOf(priorNotes.rows), {by: "entries", drill: (q) => ({q})}));

    const href = (drill: Drill = {}): string => withQuery(entriesBase, entriesQuery(query, drill));
    const set = (patch: Partial<DashQuery>): void => onQuery({...query, ...patch}, {replace: false});
    const compare = $derived(previous !== null);
</script>

<div class="flex flex-col gap-4" data-testid="time-dash-body">
    <KpiTiles
        current={kpis}
        previous={priorKpis}
        showUnbilled={meta.billableCustomers}
        commands={{
            total: totalCommand(file, range, query),
            billable: billableCommand(file, range, query),
            daily: dailyCommand(file, range, query),
            unbilled: unbilledCommand(file, range, query),
        }}
        links={{all: href(), billable: href({billable: "yes"}), unbilled: href({billed: "unbilled"})}}
    />

    <HoursChart
        {chart}
        {grain}
        split={query.split}
        hidePerson={meta.people.length <= 1}
        weeklyTarget={meta.weeklyTarget}
        showTarget={query.target}
        command={chartCommand(file, range, query, grain, query.split)}
        bucketHref={(from, to) => href({from, to})}
        seriesHref={href}
        onSplit={(split) => set({split})}
        onGrain={(g) => set({grain: g})}
        onTarget={(target) => set({target})}
    />

    <AccountTreeTable
        roots={tree.roots}
        hidden={tree.hidden}
        hiddenPrevious={tree.hiddenPrevious}
        depth={query.depth}
        total={kpis.total}
        {compare}
        command={treeCommand(file, range, query, query.depth)}
        accountHref={(account) => href({account})}
    />

    <div class="grid gap-4 @3xl:grid-cols-2">
        <TopList
            title="Top accounts"
            list={topAccounts}
            command={topAccountsCommand(file, range, query, query.depth)}
            {compare}
            {href}
            testid="time-dash-top-accounts"
        />
        {#if meta.customers.length > 0}
            <TopList
                title="Top customers"
                list={topCustomers}
                command={topCustomersCommand(file, range, query)}
                {compare}
                {href}
                testid="time-dash-top-customers"
            />
        {/if}
        {#if !onePerson}
            <TopList title="Top people" list={topPeople} command={topPeopleCommand(file, range, query)} {compare} {href} testid="time-dash-top-people" />
        {/if}
        <TopList
            title="Most frequent notes"
            list={topNotes}
            by="entries"
            command={notesCommand(file, range, query)}
            {compare}
            {href}
            testid="time-dash-top-notes"
        />
    </div>
</div>
