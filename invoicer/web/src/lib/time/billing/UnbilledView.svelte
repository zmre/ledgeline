<!-- To bill (plan 46 B-unbilled, B-ap): which invoices to create and whom to
     pay, in two tabs. *To bill*: the banner sums each customer's main
     suggestion (*3 customers have unbilled time through October: …*) with
     Bill all…; then a card per customer with its hours, estimate, oldest
     line and cadence, and Bill… per suggested period (or everything
     unbilled). Billable time no customer is linked to is listed under
     *Needs a customer*, with the way to Accounts; `invoice:` tags naming no
     invoice have Release. *To pay* lists each timekeeper's unpaid time with
     Create bill…. Each tab shows how many it lists. The tab is the route's
     (`tab`, `onTab`); 1 and 2 pick one, and ←/→ (Home, End) move along the
     tabs once one has focus.

     Bill all… opens the Bill dialog for each customer's main period in turn
     (Skip moves on, Cancel stops) and lists the drafts it made at the end;
     one Bill… opens the draft it makes. Shift+B is Bill all…, from either
     tab; `startBillAll` (the New menu's Bill time…) starts it once the lists
     are in, then says so (`onBillAllStarted`). Read again whenever a file
     changes. -->
<script lang="ts" module>
    /** The page's two tabs. */
    export type BillingTab = "bill" | "pay";
</script>

<script lang="ts">
    import {onMount, tick, untrack} from "svelte";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {EditorApiError} from "#lib/editorApi.js";
    import {errorText} from "#lib/labels.js";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import {fetchUnbilled, fetchUnpaid, releaseTime, type BillCreated, type CustomerUnbilled, type PersonUnpaid, type Unbilled} from "../api/billing.js";
    import {dayMonthYear} from "../dates.js";
    import BillDialog from "./BillDialog.svelte";
    import PayDialog from "./PayDialog.svelte";
    import {bannerText, cadenceWords, customerName, moneyText, periodLabel, personName} from "./words.js";

    let {
        tab = "bill",
        onTab = () => undefined,
        startBillAll = false,
        onBillAllStarted = () => undefined,
        onOpenDraft,
        draftHref,
        entriesHref,
        accountsHref,
        trackHref,
    }: {
        /** The tab shown. */
        tab?: BillingTab;
        /** Show another tab (the route puts it in the URL). */
        onTab?: (tab: BillingTab) => void;
        /** Start Bill all… once the lists are in. */
        startBillAll?: boolean;
        /** Bill all… was started for `startBillAll` (or there was nothing to bill). */
        onBillAllStarted?: () => void;
        /** A single Bill… made a draft: open it. */
        onOpenDraft: (created: BillCreated) => void;
        /** The editor's link for a draft (Bill all…'s list). */
        draftHref: (number: string) => string;
        /** Entries with this query (a customer's or a person's unbilled time). */
        entriesHref: (query: string) => string;
        accountsHref: string;
        trackHref: string;
    } = $props();

    interface Lists {
        unbilled: Unbilled;
        unpaid: PersonUnpaid[];
    }

    let lists = $state<Lists | null>(null);
    let error = $state<Error | null>(null);
    let view = $state<"loading" | "data" | "error">("loading");

    async function load(): Promise<void> {
        if (lists === null) view = "loading";
        try {
            const [unbilled, unpaid] = await Promise.all([fetchUnbilled(), fetchUnpaid()]);
            lists = {unbilled, unpaid};
            error = null;
            view = "data";
        } catch (e: unknown) {
            error = e instanceof Error ? e : new Error(String(e));
            view = lists === null ? "error" : "data";
        }
    }
    onMount(() => void load());
    onFilesChanged(() => void load());

    const notSetUp = $derived(error instanceof EditorApiError && error.code === "not-set-up");
    const banner = $derived(lists === null ? null : bannerText(lists.unbilled.customers));

    // ---- Bill… and Bill all…
    interface BillTarget {
        customer: string;
        name: string;
        from?: string;
        to?: string;
    }
    let billing = $state<BillTarget | null>(null);
    /** Bill all…'s run: the targets, where it is, and the drafts made so far. */
    let run = $state<{targets: BillTarget[]; at: number; made: BillCreated[]} | null>(null);
    let ranMade = $state<BillCreated[] | null>(null);

    function billAll(): void {
        if (lists === null) return;
        if (tab !== "bill") onTab("bill");
        const targets = lists.unbilled.customers
            .filter((c) => c.periods.length > 0)
            .map((c) => ({customer: c.customer, name: customerName(c), from: c.periods[0].from, to: c.periods[0].to}));
        if (targets.length === 0) return;
        ranMade = null;
        run = {targets, at: 0, made: []};
    }

    function next(made: BillCreated | null): void {
        if (run === null) return;
        const done = made === null ? run.made : [...run.made, made];
        if (run.at + 1 >= run.targets.length) {
            ranMade = done;
            run = null;
            void load();
        } else run = {...run, at: run.at + 1, made: done};
    }

    function stopRun(): void {
        if (run !== null && run.made.length > 0) ranMade = run.made;
        run = null;
        void load();
    }

    const everything = (c: CustomerUnbilled): boolean => c.periods.length === 0 || c.periods[0].quantity.seconds !== c.quantity.seconds;

    // ---- Release an orphaned tag
    let releasing = $state<string | null>(null);
    let releaseError = $state<string | null>(null);
    async function release(invoice: string): Promise<void> {
        releasing = invoice;
        releaseError = null;
        try {
            await releaseTime(invoice);
            await load();
        } catch (e: unknown) {
            releaseError = errorText(e);
        } finally {
            releasing = null;
        }
    }

    // ---- Create bill…
    interface PayTarget {
        person: string;
        name: string;
        from?: string;
        to?: string;
    }
    let paying = $state<PayTarget | null>(null);

    // The New menu's Bill time…: once, when the lists are in.
    $effect(() => {
        if (!startBillAll || lists === null) return;
        untrack(() => {
            if (run === null && billing === null && paying === null) billAll();
            onBillAllStarted();
        });
    });

    // ---- The tabs
    const TABS: readonly {id: BillingTab; label: string}[] = [
        {id: "bill", label: "To bill"},
        {id: "pay", label: "To pay"},
    ];
    const tabCount = (id: BillingTab, l: Lists): number => (id === "bill" ? l.unbilled.customers.length : l.unpaid.length);
    const tabCountLabel = (id: BillingTab, n: number): string =>
        id === "bill" ? (n === 1 ? "customer" : "customers") : n === 1 ? "person to pay" : "people to pay";

    async function choose(next: BillingTab, focus = false): Promise<void> {
        if (next !== tab) onTab(next);
        if (focus) {
            await tick();
            document.getElementById(`to-bill-tab-${next}`)?.focus();
        }
    }

    function onTabKey(event: KeyboardEvent): void {
        const at = TABS.findIndex((t) => t.id === tab);
        const to =
            event.key === "ArrowRight"
                ? (at + 1) % TABS.length
                : event.key === "ArrowLeft"
                  ? (at + TABS.length - 1) % TABS.length
                  : event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? TABS.length - 1
                      : null;
        if (to === null) return;
        event.preventDefault();
        void choose(TABS[to].id, true);
    }

    registerKeys({
        id: "to-bill-page",
        priority: PRIORITY.page,
        bindings: [
            {keys: "B", label: "Bill all…", group: "To bill", enabled: () => banner !== null && run === null && billing === null, run: billAll},
            {keys: "1", label: "Show To bill", group: "To bill", run: () => void choose("bill")},
            {keys: "2", label: "Show To pay", group: "To bill", run: () => void choose("pay")},
        ],
    });

    const entriesFor = (key: "customer" | "person", value: string): string =>
        entriesHref(new URLSearchParams({period: "all", [key]: value, ...(key === "customer" ? {billed: "unbilled"} : {})}).toString());
</script>

{#snippet toPay(l: Lists)}
    <section class="flex flex-col gap-2" aria-label="To pay" data-testid="unbilled-to-pay">
        {#if l.unpaid.length === 0}
            <p class="text-sm text-base-content/70">
                No contractor time is waiting. Time from someone with a timekeeper note (a person or organization note tagged timekeeper) shows here until it is
                on a bill.
            </p>
        {:else}
            <div class="grid gap-3 @3xl:grid-cols-2">
                {#each l.unpaid as p (p.person)}
                    <article
                        class="flex flex-col gap-2 rounded-box border border-base-300 p-4"
                        aria-label={personName(p)}
                        data-testid="unpaid-person"
                        data-person={p.person}
                    >
                        <div class="flex flex-wrap items-baseline justify-between gap-2">
                            <h3 class="font-semibold">{personName(p)}</h3>
                            <span class="tabular-nums">{p.quantity.text}</span>
                        </div>
                        <p class="text-xs text-base-content/70">
                            {p.entries === 1 ? "1 entry" : `${p.entries} entries`}, {p.oldest === p.newest
                                ? dayMonthYear(p.oldest)
                                : `${dayMonthYear(p.oldest)} to ${dayMonthYear(p.newest)}`}
                            ·
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- entriesHref resolves -->
                            <a class="link" href={entriesFor("person", p.person === "" ? "-" : p.person)}>See the entries</a>
                        </p>
                        {#if p.person === ""}
                            <p class="text-xs text-warning">Set who did this time in Entries before it can be paid.</p>
                        {:else if p.periods.length > 0}
                            <ul class="flex flex-col gap-1">
                                {#each p.periods as period (period.from)}
                                    <li class="flex flex-wrap items-center justify-between gap-2" data-testid="unpaid-period">
                                        <span class="text-sm">{periodLabel(period.from, period.to)} · {period.quantity.text}</span>
                                        <button
                                            type="button"
                                            class="btn btn-outline btn-xs"
                                            onclick={() => (paying = {person: p.person, name: personName(p), from: period.from, to: period.to})}
                                            aria-label="Create a bill for {personName(p)}, {periodLabel(period.from, period.to)}…"
                                            data-testid="unpaid-create-bill">Create bill…</button
                                        >
                                    </li>
                                {/each}
                            </ul>
                        {:else}
                            <button
                                type="button"
                                class="btn self-start btn-outline btn-xs"
                                onclick={() => (paying = {person: p.person, name: personName(p)})}
                                aria-label="Create a bill for {personName(p)}…"
                                data-testid="unpaid-create-bill">Create bill…</button
                            >
                        {/if}
                    </article>
                {/each}
            </div>
        {/if}
    </section>
{/snippet}

<div class="@container mx-auto flex max-w-6xl flex-col gap-4">
    <header class="flex flex-wrap items-center gap-x-4 gap-y-2">
        <h1 class="text-2xl font-semibold tracking-tight">To bill</h1>
        {#if !notSetUp}
            <div role="tablist" class="tabs tabs-border" aria-label="To bill" tabindex="-1" onkeydown={onTabKey}>
                {#each TABS as t (t.id)}
                    {@const n = lists === null ? null : tabCount(t.id, lists)}
                    <button
                        type="button"
                        role="tab"
                        id="to-bill-tab-{t.id}"
                        class={["tab gap-1.5", t.id === tab && "tab-active"]}
                        aria-selected={t.id === tab}
                        aria-controls="to-bill-panel-{t.id}"
                        tabindex={t.id === tab ? 0 : -1}
                        onclick={() => void choose(t.id)}
                        data-testid="to-bill-tab-{t.id}"
                        >{t.label}{#if n !== null && n > 0}
                            <span class="text-xs text-base-content/60 tabular-nums" title="{n} {tabCountLabel(t.id, n)}" data-testid="to-bill-tab-{t.id}-count"
                                >{n}<span class="sr-only">{` ${tabCountLabel(t.id, n)}`}</span></span
                            >{/if}</button
                    >
                {/each}
            </div>
        {/if}
    </header>

    {#if notSetUp}
        <EmptyState
            title="Time tracking isn't set up"
            text="These books have no time files yet. Set up time tracking, then the time to bill shows here."
            action="Set up time tracking"
            href={trackHref}
            testid="unbilled-not-set-up"
        />
    {:else}
        <AsyncSection
            {view}
            value={lists}
            {error}
            testid="unbilled-error"
            label="the unbilled time"
            loadingLabel="Loading unbilled time"
            onRetry={() => void load()}
        >
            {#snippet children(l)}
                <div role="tabpanel" id="to-bill-panel-{tab}" aria-labelledby="to-bill-tab-{tab}" tabindex="-1" class="flex flex-col gap-4">
                    {#if tab === "pay"}
                        {@render toPay(l)}
                    {:else}
                        {#if ranMade !== null}
                            <div class="alert text-sm alert-success" role="status" data-testid="unbilled-run-done">
                                {#if ranMade.length === 0}
                                    No drafts were made.
                                {:else}
                                    <span
                                        >Made {ranMade.length === 1 ? "a draft" : `${ranMade.length} drafts`}:
                                        {#each ranMade as made, i (made.number)}{i > 0
                                                ? ", "
                                                : ""}<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- draftHref resolves -->
                                            <a class="link" href={draftHref(made.number)}>invoice {made.number}</a>{/each}. Review and send each from its
                                        editor.</span
                                    >
                                {/if}
                                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (ranMade = null)}>Dismiss</button>
                            </div>
                        {/if}

                        {#if banner !== null}
                            <div class="alert flex-wrap alert-info" role="status" data-testid="unbilled-banner">
                                <span class="grow text-sm">{banner}</span>
                                <button type="button" class="btn btn-primary btn-sm" onclick={billAll} title="Shift+B" data-testid="unbilled-bill-all"
                                    >Bill all…</button
                                >
                            </div>
                        {/if}

                        {#if l.unbilled.customers.length === 0 && l.unbilled.needs_customer.length === 0 && l.unbilled.orphans.length === 0}
                            <EmptyState
                                title="Nothing to bill"
                                text="Billable time on an account linked to a customer shows here until it is on an invoice."
                                testid="unbilled-empty"
                            />
                        {/if}

                        {#if l.unbilled.customers.length > 0}
                            <section class="flex flex-col gap-2" aria-label="By customer">
                                <div class="grid gap-3 @3xl:grid-cols-2">
                                    {#each l.unbilled.customers as c (c.customer)}
                                        {@const estimate = moneyText(c.estimate, c.commodity)}
                                        <article
                                            class="flex flex-col gap-2 rounded-box border border-base-300 p-4"
                                            aria-label={customerName(c)}
                                            data-testid="unbilled-customer"
                                            data-customer={c.customer}
                                        >
                                            <div class="flex flex-wrap items-baseline justify-between gap-2">
                                                <h3 class="font-semibold">{customerName(c)}</h3>
                                                <span class="tabular-nums" data-testid="unbilled-customer-hours"
                                                    >{c.quantity.text}{#if estimate !== null}<span class="ml-2 text-sm text-base-content/70"
                                                            >about {estimate}</span
                                                        >{/if}</span
                                                >
                                            </div>
                                            <p class="text-xs text-base-content/70">
                                                Oldest {dayMonthYear(c.oldest)} · {cadenceWords(c.cadence)} ·
                                                <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- entriesHref resolves -->
                                                <a class="link" href={entriesFor("customer", c.customer)}>See the entries</a>
                                            </p>
                                            <ul class="flex flex-col gap-1">
                                                {#each c.periods as p (p.from)}
                                                    <li class="flex flex-wrap items-center justify-between gap-2" data-testid="unbilled-period">
                                                        <span class="text-sm"
                                                            >{periodLabel(p.from, p.to)} · {p.quantity.text} · {p.entries === 1
                                                                ? "1 entry"
                                                                : `${p.entries} entries`}</span
                                                        >
                                                        <button
                                                            type="button"
                                                            class="btn btn-outline btn-xs"
                                                            onclick={() => (billing = {customer: c.customer, name: customerName(c), from: p.from, to: p.to})}
                                                            aria-label="Bill {customerName(c)}, {periodLabel(p.from, p.to)}…"
                                                            data-testid="unbilled-bill">Bill…</button
                                                        >
                                                    </li>
                                                {/each}
                                                {#if everything(c)}
                                                    <li class="flex flex-wrap items-center justify-between gap-2" data-testid="unbilled-everything">
                                                        <span class="text-sm">Everything unbilled, to {dayMonthYear(c.newest)} · {c.quantity.text}</span>
                                                        <button
                                                            type="button"
                                                            class="btn btn-ghost btn-xs"
                                                            onclick={() =>
                                                                (billing = {customer: c.customer, name: customerName(c), from: c.oldest, to: c.newest})}
                                                            aria-label="Bill all of {customerName(c)}'s unbilled time…"
                                                            data-testid="unbilled-bill-everything">Bill…</button
                                                        >
                                                    </li>
                                                {/if}
                                            </ul>
                                        </article>
                                    {/each}
                                </div>
                            </section>
                        {/if}

                        {#if l.unbilled.needs_customer.length > 0}
                            <section class="flex flex-col gap-2" aria-labelledby="unbilled-needs-title" data-testid="unbilled-needs-customer">
                                <h2 id="unbilled-needs-title" class="text-lg font-semibold">Needs a customer</h2>
                                <p class="text-sm text-base-content/70">
                                    This time is billable, but its account isn't linked to a customer, so it can't go on an invoice yet.
                                </p>
                                <ul class="flex flex-col gap-1">
                                    {#each l.unbilled.needs_customer as n (n.account)}
                                        <li class="flex flex-wrap items-center justify-between gap-2 text-sm" data-testid="unbilled-needs-row">
                                            <span
                                                ><span class="font-medium break-all">{n.account}</span> · {n.quantity.text} · since {dayMonthYear(
                                                    n.oldest
                                                )}</span
                                            >
                                        </li>
                                    {/each}
                                </ul>
                                <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page resolves it -->
                                <a class="btn self-start btn-outline btn-sm" href={accountsHref} data-testid="unbilled-link-customer"
                                    >Link a customer on Accounts</a
                                >
                            </section>
                        {/if}

                        {#if l.unbilled.orphans.length > 0}
                            <section class="flex flex-col gap-2" aria-labelledby="unbilled-orphans-title" data-testid="unbilled-orphans">
                                <h2 id="unbilled-orphans-title" class="text-lg font-semibold">On an invoice that no longer exists</h2>
                                <ul class="flex flex-col gap-1">
                                    {#each l.unbilled.orphans as o (o.invoice)}
                                        <li class="flex flex-wrap items-center justify-between gap-2 text-sm" data-testid="unbilled-orphan">
                                            <span
                                                >{`${o.entries.length === 1 ? "1 line" : `${o.entries.length} lines`} (${o.quantity.text}) still say they're on invoice ${o.invoice}, which isn't in the files or the books.`}</span
                                            >
                                            <button
                                                type="button"
                                                class="btn btn-outline btn-xs"
                                                disabled={releasing !== null}
                                                onclick={() => void release(o.invoice)}
                                                aria-label="Release invoice {o.invoice}'s time"
                                                data-testid="unbilled-release"
                                            >
                                                {#if releasing === o.invoice}<span class="loading loading-xs loading-spinner"></span>{/if}
                                                Release</button
                                            >
                                        </li>
                                    {/each}
                                </ul>
                                <p class="text-xs text-base-content/60">Release makes the time unbilled again.</p>
                                {#if releaseError !== null}<p class="text-sm text-error" role="alert">{releaseError}</p>{/if}
                            </section>
                        {/if}
                    {/if}
                </div>
            {/snippet}
        </AsyncSection>
    {/if}
</div>

{#if billing !== null}
    <BillDialog
        customer={billing.customer}
        name={billing.name}
        from={billing.from}
        to={billing.to}
        onCreated={(created) => {
            billing = null;
            onOpenDraft(created);
        }}
        onCancel={() => (billing = null)}
    />
{/if}

{#if run !== null}
    {@const t = run.targets[run.at]}
    {#key run.at}
        <BillDialog
            customer={t.customer}
            name={t.name}
            from={t.from}
            to={t.to}
            step={{at: run.at + 1, of: run.targets.length}}
            onCreated={(created) => next(created)}
            onSkip={() => next(null)}
            onCancel={stopRun}
        />
    {/key}
{/if}

{#if paying !== null}
    <PayDialog person={paying.person} name={paying.name} from={paying.from} to={paying.to} onBooked={() => void load()} onClose={() => (paying = null)} />
{/if}
