<script lang="ts">
    // The Projections tab: a what-if you can edit, and the runway it implies.
    //
    // Top half is the editable scenario table, bottom half is that scenario read
    // forward as the runway and the three official statements (plan 28). They
    // are one subject read two ways, so they share a page and a recompute: every
    // edit bumps the store's
    // revision and schedules one debounced `POST /api/projections/run`, and the
    // charts below say so while it is in flight rather than showing a projection
    // of a table that is no longer on screen.
    //
    // The controls, the URL mirroring and the load effect follow
    // `routes/budget/+page.svelte` verbatim — same `searchMirror`, same
    // `onServerReady` latch, same `{url, nonce, query}` load key (FE-5d: a
    // reconnect usually leaves the URL identical, so keying on it alone means a
    // page never retries after one).
    //
    // # Where a scenario lives
    //
    // A scenario is an ordinary journal file that nothing includes, so it can be
    // edited, diffed and read by hledger itself. `fileStore` owns which file the
    // one on screen came from; `scenarioStore` owns the scenario. The two meet
    // in exactly three places, and nowhere else:
    //
    //   load  → `fileStore.open`   → `scenarioStore.adopt`
    //   save  → `fileStore.save`   → `scenarioStore.markSaved`
    //   seed  → `scenarioStore.ensureSeeded`, which declines over `dirty` work
    //
    // On first mount the tab reopens whatever was last loaded
    // (`settings.lastProjectionId` — decision 10, per-browser view state rather
    // than `prefs.json`). A remembered id that no longer names a file is not an
    // error: it is forgotten and the tab seeds instead, which is where a user
    // who deleted a file outside Ledgeline should end up.
    import {onMount} from "svelte";
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import DepthSlider from "#lib/insights/DepthSlider.svelte";
    import {fileStore} from "#lib/projections/fileStore.svelte.js";
    import {defaultProjectionParams, projectionParamsToSearch, searchToProjectionParams, type ProjectionParams} from "#lib/projections/params.js";
    import {assetRowsByGroup} from "#lib/projections/projectionView.js";
    import {sameProjectionQuery, scenarioStore, type ProjectionWindow} from "#lib/projections/scenarioStore.svelte.js";
    import ProjectionReports from "#lib/projections/ui/ProjectionReports.svelte";
    import ProjectionsTable from "#lib/projections/ui/ProjectionsTable.svelte";
    import SaveScenarioDialog from "#lib/projections/ui/SaveScenarioDialog.svelte";
    import ScenarioFileBar from "#lib/projections/ui/ScenarioFileBar.svelte";
    import {bucketStart, today} from "#lib/reports/periods.js";
    import {MAX_COUNT} from "#lib/reports/ui/params.js";
    import {dataView} from "#lib/stores/loadState.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {loadJournalWhenReady, onServerReady} from "#lib/stores/serverWatch.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {searchMirror} from "#lib/url/searchSync.js";

    let params = $state<ProjectionParams>(defaultProjectionParams());
    let restored = $state(false);

    onMount(() => {
        if (window.location.search !== "") Object.assign(params, searchToProjectionParams(window.location.search, defaultProjectionParams()));
        restored = true;
        return () => {
            mirror.stop();
            scenarioStore.stop();
        };
    });

    // Mirror params → URL, debounced, replaceState (no history entries, no loops).
    // Reading `params` before the `restored` guard is deliberate: the effect has
    // to depend on them even on the run where it declines to write.
    const mirror = searchMirror();
    $effect(() => {
        const search = projectionParamsToSearch(params);
        if (!restored) return;
        mirror.write(search);
    });

    // Account names, styles and account classes come from the journal round;
    // the seed and the projection are native. Never a gate — `formatTotals`
    // falls back per commodity.
    loadJournalWhenReady();
    const styles = $derived(journal.reportStyles);
    const declared = $derived(journal.accountClasses);
    const commodity = $derived(journal.dominantCommodity);
    const maxDepth = $derived(journal.accountNames.reduce((max, name) => Math.max(max, name.split(":").length), 1));

    // --- The scenario --------------------------------------------------------

    onServerReady((url) => void start(url));
    const seed = $derived(scenarioStore.seed);
    const seedView = $derived(dataView(seed.status, seed.value !== null));

    /**
     * Reopen the last-loaded file, or seed.
     *
     * The order is load-bearing. The listing comes first because the file
     * controls need it either way; the remembered file is tried next; and only
     * if that produces nothing does the journal get read for a seed. Doing it
     * the other way round would show the user an average of their history for a
     * moment before replacing it with the scenario they actually left open.
     */
    async function start(url: string): Promise<void> {
        await fileStore.ensureIndex(url, settings.serverNonce);
        const remembered = settings.lastProjectionId;
        if (fileStore.available && remembered !== null && (await openFile(url, remembered))) return;
        await scenarioStore.ensureSeeded(url, settings.serverNonce);
    }

    /** Load one file into the table. `false` means it could not be opened. */
    async function openFile(url: string, id: string): Promise<boolean> {
        const outcome = await fileStore.open(url, id);
        if (!outcome.ok) {
            // A remembered id that no longer resolves must not be retried on
            // every mount, which would put a failure on the screen each time the
            // tab is opened.
            if (outcome.failure.kind === "notFound" || outcome.failure.kind === "validation") fileStore.forget();
            fileError = outcome.failure.message;
            return false;
        }
        fileError = null;
        scenarioStore.adopt(outcome.file.scenario);
        return true;
    }

    // --- Saving --------------------------------------------------------------

    let saveAsOpen = $state(false);
    /** The engine's own sentence from the last failed load or save, or null. */
    let fileError = $state<string | null>(null);
    const fileIndex = $derived(fileStore.index.value);
    const busy = $derived(fileStore.saving || fileStore.loading);

    function chooseFile(id: string): void {
        const url = settings.serverUrl;
        if (url !== null) void openFile(url, id);
    }

    async function saveOver(): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        const outcome = await fileStore.save(url, scenarioStore.scenario);
        if (outcome.ok) {
            // `markSaved` rather than `adopt`: the scenario on screen is the one
            // that was written, and re-adopting the engine's echo of it would
            // discard a keystroke the user made while the request was in flight.
            scenarioStore.markSaved();
            fileError = null;
        } else {
            fileError = outcome.failure.message;
        }
    }

    async function saveAs(id: string, name: string): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        // The NAME is part of the scenario, and the file is named after it — so
        // it is set before the write rather than afterwards, or the file would
        // carry a `; projection:` line the picker does not show.
        scenarioStore.scenario.name = name;
        scenarioStore.touch();
        const outcome = await fileStore.saveAs(url, id, scenarioStore.scenario);
        if (outcome.ok) {
            scenarioStore.markSaved();
            saveAsOpen = false;
            fileError = null;
        } else {
            fileError = outcome.failure.message;
        }
    }

    // --- The projection ------------------------------------------------------

    const projectionWindow = $derived<ProjectionWindow>({interval: params.interval, count: params.count, lookback: params.lookback, depth: params.depth});
    // `journal.changes`: projections start from the journal's balances, so a
    // refresh or a change on disk recomputes them.
    const loadKey = $derived({
        url: settings.serverUrl,
        nonce: settings.serverNonce,
        changes: journal.changes,
        revision: scenarioStore.revision,
        window: projectionWindow,
    });
    $effect(() => {
        const {url, window: w} = loadKey;
        // `revision` is read through `loadKey`, so an edit re-runs this effect —
        // which is the whole recompute trigger. The store debounces from here.
        if (url !== null) scenarioStore.schedule(url, w);
    });

    const projection = $derived(scenarioStore.projection);

    /**
     * The held projection, IF it answers the table and the controls now on
     * screen (FE-1).
     */
    const current = $derived.by(() => {
        const q = projection.query;
        const value = projection.value;
        if (q === null || value === null) return null;
        return sameProjectionQuery(q, {...projectionWindow, revision: scenarioStore.revision}) ? value : null;
    });

    /**
     * A STALE projection is kept on screen and LABELLED, rather than replaced
     * by a spinner.
     *
     * This is a deliberate departure from the reports tabs, which pass
     * `matchesRequest: false` and go back to loading. There, a held payload
     * answers a DIFFERENT report and showing it under the new one's label was
     * FE-1. Here every payload answers the same three questions about the same
     * scenario, one edit older — and an edit happens on every keystroke, so
     * blanking the charts each time would make the tab unreadable exactly while
     * it is being used. So the last good answer stays, dimmed, under a chip that
     * says it is being recomputed. `current` is still what the page trusts
     * anywhere a claim is made about the CURRENT scenario (the start date above,
     * and the warnings).
     */
    const stale = $derived(current === null && projection.value !== null);
    const projectionView = $derived(dataView(projection.status, projection.value !== null));

    /**
     * The date "Add a step" opens on: the middle of the projected window.
     *
     * A step wants to be somewhere a reader can see its effect, and the middle
     * of the chart is that. Before a projection has landed there is no window to
     * take a middle of, so it falls back to today — which the user then edits,
     * as they would either way.
     */
    const stepDate = $derived.by(() => {
        const buckets = projection.value?.buckets ?? [];
        return buckets.length === 0 ? today() : bucketStart(buckets[Math.floor(buckets.length / 2)]);
    });

    const warnings = $derived(current?.warnings ?? []);

    /**
     * The journal's own balance per asset row, for the table's Balance column.
     *
     * Off `projection.value` — the LAST GOOD answer — and deliberately not off
     * `current`. Everywhere else on this page that rule is the other way round,
     * because a held payload makes a claim about a scenario that has since
     * changed. This one does not: it is a fact about the journal as of today,
     * and the journal does not change between two keystrokes. Keying it to
     * `current` would blank the column on every letter typed anywhere in the
     * table. What CAN go stale is which account a row names, and
     * `journalBalanceFor` refuses an entry whose account no longer matches.
     */
    const assetBalances = $derived(assetRowsByGroup(projection.value));
    /**
     * The engine's inferred counter, gains and cash accounts, greyed in the
     * table — off the last good answer for `assetBalances`' reason. A row whose
     * rule was just split has no entry until the next answer lands, and shows
     * "default" meanwhile rather than another rule's account.
     */
    const forecastDefaults = $derived(projection.value?.defaults ?? null);

    function setCount(value: string): void {
        const n = Number(value);
        if (Number.isInteger(n)) params.count = Math.min(MAX_COUNT, Math.max(1, n));
    }

    function setLookback(value: string): void {
        const n = Number(value);
        if (Number.isInteger(n)) params.lookback = Math.min(MAX_COUNT, Math.max(0, n));
    }

    function retry(): void {
        const url = settings.serverUrl;
        if (url !== null) void scenarioStore.runNow(url, projectionWindow);
    }

    function retrySeed(): void {
        const url = settings.serverUrl;
        if (url !== null) void scenarioStore.reseed(url);
    }
</script>

<svelte:head><title>Ledgeline — Projections</title></svelte:head>

<div class="flex flex-col gap-6">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
        <h1 class="text-lg font-semibold">Projections</h1>
        {#if current !== null}
            <p class="text-sm text-base-content/60">
                Actuals run to the last complete period; the first projected period starts {current.actualsEnd}.
            </p>
        {/if}
    </div>

    <!-- Which file this scenario came from, and where it goes. Absent on an
         engine that predates Phase 3, which still seeds and projects. -->
    {#if fileStore.available && fileIndex !== null}
        <ScenarioFileBar
            files={fileIndex.files}
            currentId={fileStore.currentId}
            canSave={fileStore.canSave}
            dirty={scenarioStore.dirty}
            editable={fileIndex.editable}
            {busy}
            truncated={fileIndex.truncated}
            onOpen={chooseFile}
            onSave={() => void saveOver()}
            onSaveAs={() => {
                fileError = null;
                saveAsOpen = true;
            }}
        />
    {/if}

    {#if fileError !== null && !saveAsOpen}
        <div class="alert items-start rounded-box px-3 py-2 text-sm alert-error" role="alert" data-testid="projection-file-error">{fileError}</div>
    {/if}

    <!-- The what-if. Seeded once from the journal: the budget's own `~` rules,
         plus one monthly line per category the budget does not mention. -->
    <AsyncSection
        view={seedView}
        value={seed.value}
        error={seed.error}
        testid="projections-seed-error"
        label="a starting scenario"
        loadingLabel="Loading a starting scenario"
        onRetry={retrySeed}
    >
        <!-- Implicit children: the table renders the STORE's scenario, not the
             seed payload. The seed is adopted once, cloned into `$state`, and
             then edited — so the held payload is the thing that gates this
             section, never the thing it draws. -->
        <ProjectionsTable
            scenario={scenarioStore.scenario}
            accountNames={journal.accountNames}
            {declared}
            {commodity}
            {stepDate}
            {assetBalances}
            defaults={forecastDefaults}
            {styles}
            onChange={() => scenarioStore.touch()}
        />
    </AsyncSection>

    <div class="divider my-0"></div>

    <div class="flex flex-wrap items-end gap-x-4 gap-y-2 rounded-box bg-base-200 px-3 py-2">
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Interval</span>
            <select class="select w-32 select-sm" bind:value={params.interval} aria-label="Interval">
                <option value="monthly">Monthly</option>
                <option value="quarterly">Quarterly</option>
                <option value="yearly">Yearly</option>
            </select>
        </label>
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Periods</span>
            <input
                type="number"
                class="input w-20 input-sm"
                min="1"
                max={MAX_COUNT}
                value={params.count}
                onchange={(e) => setCount(e.currentTarget.value)}
                aria-label="Number of periods"
            />
        </label>
        <!-- How many full ACTUAL periods the statements show before the
             forecast (plan 28 Decision 6). Zero is allowed: forecast only. -->
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Actuals</span>
            <input
                type="number"
                class="input w-20 input-sm"
                min="0"
                max={MAX_COUNT}
                value={params.lookback}
                onchange={(e) => setLookback(e.currentTarget.value)}
                aria-label="Actual periods before the projection"
            />
        </label>
        <!-- Keyed on maxDepth for the reason the reports bar keys it: the slider
             can mount while accounts are still loading (max=1), and the browser
             clamps the input's value to that max without updating bound state. -->
        {#key maxDepth}
            <DepthSlider bind:depth={params.depth} max={maxDepth} />
        {/key}
    </div>

    {#if warnings.length > 0}
        <!-- Never silent: a line the projection could not enumerate says so here
             rather than contributing a quiet zero. -->
        <div class="alert items-start rounded-box px-3 py-2 text-sm alert-warning" role="alert" data-testid="projection-warnings">
            <ul class="list-inside list-disc">
                {#each warnings as warning (warning)}
                    <li>{warning}</li>
                {/each}
            </ul>
        </div>
    {/if}

    <div class="flex flex-col gap-2">
        {#if stale}
            <p class="text-xs text-base-content/60" role="status" aria-live="polite" data-testid="projection-stale">Recomputing for your latest edit…</p>
        {/if}
        <div class={stale ? "opacity-60" : ""} aria-busy={stale}>
            <AsyncSection
                view={projectionView}
                value={projection.value}
                error={projection.error}
                testid="projection-error"
                label="the projection"
                loadingLabel="Projecting"
                onRetry={retry}
            >
                {#snippet children(held)}
                    <ProjectionReports bind:tab={params.tab} projection={held} interval={params.interval} {styles} fallbackCommodity={commodity} />
                {/snippet}
            </AsyncSection>
        </div>
    </div>
</div>

{#if saveAsOpen && fileIndex !== null}
    <SaveScenarioDialog
        name={scenarioStore.scenario.name}
        directories={fileIndex.directories}
        files={fileIndex.files}
        saving={fileStore.saving}
        error={fileError}
        onSave={(id, name) => void saveAs(id, name)}
        onCancel={() => (saveAsOpen = false)}
    />
{/if}
