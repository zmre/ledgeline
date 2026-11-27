<!-- Time tracking's dashboard (plan 46 T-dash): where the time went, for
     anyone tracking time, invoicing or not. The filter bar's state is the
     URL's (`query.ts`), so a reload, a link and Back/Forward land on the same
     page, and every figure drills into Entries with the same filters.

     Data, per period: one `GET /api/time/summary` grouped into a fact table
     (account, customer, person, billable, billed, day) and one grouped by
     note, for the period and, with comparison on, the one before it: four
     requests at once, nothing more when the split, grain, depth or target
     change. Once per visit (and after a file changes): the state, the
     accounts and everyone's hours (`meta.ts`). Only the latest request's
     answer is shown; while the next one loads, the figures stay on screen,
     dimmed (dataviz: refetch keeps the frame).

     Before setup, and with no time yet, the page points at Track.

     Keys (the page's layer, in the `?` sheet's Time group): `[` and `]`
     step the period; `/` focuses the account filter (the app's search key).

     Mount it on the dashboard route with the resolved links it needs:
         <TimeDashboard entriesBase={resolve("/time/entries")} trackHref={resolve("/time/track")} /> -->
<script lang="ts">
    import {untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import type {DataView} from "#ui/ui/dataView.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import {localToday} from "#lib/invoices.js";
    import {fetchSummary, type Summary} from "../api/summary.js";
    import DashboardBody from "./DashboardBody.svelte";
    import DashFilters from "./DashFilters.svelte";
    import {fetchDashMeta, type DashMeta} from "./meta.js";
    import {FACT_GROUPS, factsOf, type Fact} from "./model.js";
    import {apiFilter, clearFilters, dashQueryString, parseDashQuery, previousRange, resolveRange, stepPeriod, type DashQuery, type Range} from "./query.js";

    let {
        entriesBase,
        trackHref,
        today = localToday(),
    }: {
        /** Entries' URL, resolved (`/app/time/entries`): every drill-down's base. */
        entriesBase: string;
        /** Track's URL, resolved: where the empty states point. */
        trackHref: string;
        /** The local date; a test pins it. */
        today?: string;
    } = $props();

    const query = $derived(parseDashQuery(page.url.searchParams));
    const range = $derived(resolveRange(query, today));
    const previous = $derived<Range | null>(query.compare ? previousRange(range) : null);

    // ── The meta: once, and again after a change on disk ────────────────
    let meta = $state<DashMeta | null>(null);
    let metaError = $state<Error | null>(null);
    let metaSeq = 0;
    function loadMeta(): void {
        const mine = ++metaSeq;
        fetchDashMeta()
            .then((m) => {
                if (mine !== metaSeq) return;
                meta = m;
                metaError = null;
            })
            .catch((e: unknown) => {
                if (mine !== metaSeq) return;
                if (meta === null) metaError = e instanceof Error ? e : new Error(String(e));
                else console.warn("time dashboard:", e);
            });
    }

    // ── The period's figures ───────────────────────────────────────────
    interface Figures {
        current: Fact[];
        prior: Fact[] | null;
        notes: Summary;
        priorNotes: Summary | null;
        file: string;
    }

    let figures = $state<Figures | null>(null);
    let figuresError = $state<Error | null>(null);
    let loading = $state(false);
    let figuresSeq = 0;

    /** What the figures depend on: the filters and the two periods, not the view options. */
    const request = $derived({current: apiFilter(query, range), previous: previous === null ? null : apiFilter(query, previous)});
    const requestKey = $derived(JSON.stringify(request));

    async function fetchFigures(req: typeof request): Promise<Figures> {
        const [current, notes, prior, priorNotes] = await Promise.all([
            fetchSummary({...req.current, group: FACT_GROUPS}),
            fetchSummary({...req.current, group: ["note"]}),
            req.previous === null ? null : fetchSummary({...req.previous, group: FACT_GROUPS}),
            req.previous === null ? null : fetchSummary({...req.previous, group: ["note"]}),
        ]);
        return {current: factsOf(current), prior: prior === null ? null : factsOf(prior), notes, priorNotes, file: current.file};
    }

    function loadFigures(req: typeof request): void {
        const mine = ++figuresSeq;
        loading = true;
        fetchFigures(req)
            .then((f) => {
                if (mine !== figuresSeq) return;
                figures = f;
                figuresError = null;
            })
            .catch((e: unknown) => {
                if (mine !== figuresSeq) return;
                figuresError = e instanceof Error ? e : new Error(String(e));
            })
            .finally(() => {
                if (mine === figuresSeq) loading = false;
            });
    }

    $effect(() => {
        untrack(() => loadMeta());
    });

    // Asked again only when the filters or the periods change.
    let lastKey = "";
    $effect(() => {
        const key = requestKey;
        const configured = meta?.configured === true && meta.anyTime;
        if (!configured || key === lastKey) return;
        lastKey = key;
        untrack(() => loadFigures(request));
    });

    onFilesChanged(() => {
        loadMeta();
        if (lastKey !== "") loadFigures(request);
    });

    const view = $derived<DataView>(figuresError !== null ? "error" : figures !== null ? "data" : "loading");

    // ── The URL ─────────────────────────────────────────────────────────
    // The URL changes only once a navigation lands, so a second `[` pressed
    // before then would step from the old period. `pending` is the query the
    // last navigation is heading to; steps and choices build on it.
    let pending: DashQuery | null = null;
    const current = (): DashQuery => pending ?? query;

    function navigate(next: DashQuery, options: {replace: boolean} = {replace: false}): void {
        const qs = dashQueryString(next);
        if (qs === dashQueryString(current())) return;
        pending = next;
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the current page's own path, only the query changes
        void Promise.resolve(goto(`${page.url.pathname}${qs === "" ? "" : `?${qs}`}`, {replace: options.replace, reset: false})).finally(() => {
            if (pending === next) pending = null;
        });
    }

    const step = (direction: -1 | 1): void => navigate(stepPeriod(current(), today, direction));

    registerKeys({
        id: "time-dashboard",
        priority: PRIORITY.page,
        bindings: [
            {keys: "[", label: "Previous period", group: "Time", run: () => step(-1)},
            {keys: "]", label: "Next period", group: "Time", run: () => step(1)},
        ],
    });

    function retry(): void {
        figuresError = null;
        if (meta === null) {
            metaError = null;
            loadMeta();
        } else loadFigures(request);
    }
</script>

<!-- AsyncSection's data branch, never reached while the meta loads or fails. -->
{#snippet nothing()}{/snippet}

<div class="@container mx-auto flex max-w-6xl flex-col gap-4" data-testid="time-dashboard">
    <h1 class="text-2xl font-semibold tracking-tight">Time</h1>
    {#if metaError !== null || meta === null}
        <AsyncSection
            view={metaError !== null ? "error" : "loading"}
            value={null}
            error={metaError}
            testid="time-dash-error"
            label="time tracking"
            loadingLabel="Loading time tracking"
            onRetry={retry}
            children={nothing}
        />
    {:else if !meta.configured}
        <EmptyState
            title="Time tracking isn't set up"
            text="Set it up on Track: it creates the time files beside your books. Hours tracked there show here, by account, customer and week."
            action="Go to Track"
            href={trackHref}
            testid="time-dash-setup"
        />
    {:else if !meta.anyTime}
        <EmptyState
            title="No time yet"
            text="Start a timer on Track, or add an entry in Entries. Where your hours go shows here."
            action="Go to Track"
            href={trackHref}
            testid="time-dash-empty"
        />
    {:else}
        {@const known = meta}
        <DashFilters {query} meta={known} {range} {previous} onChange={navigate} onStep={step} onClear={() => navigate(clearFilters(query))} />
        <AsyncSection
            {view}
            value={figures}
            error={figuresError}
            testid="time-dash-error"
            label="the time dashboard"
            loadingLabel="Loading the time dashboard"
            onRetry={retry}
        >
            {#snippet children(f)}
                <div class={["transition-opacity motion-reduce:transition-none", loading && "opacity-60"]} aria-busy={loading}>
                    <DashboardBody
                        {query}
                        {range}
                        previous={f.prior === null ? null : previous}
                        current={f.current}
                        prior={previous === null ? null : f.prior}
                        notes={f.notes}
                        priorNotes={f.priorNotes}
                        meta={known}
                        file={f.file}
                        {entriesBase}
                        onQuery={navigate}
                    />
                </div>
            {/snippet}
        </AsyncSection>
    {/if}
</div>
