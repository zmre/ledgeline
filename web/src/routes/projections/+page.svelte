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
    import {sameProjectionQuery, scenarioStore, scenarioTableView, type ProjectionWindow} from "#lib/projections/scenarioStore.svelte.js";
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
    import {pollWhileVisible} from "#lib/stores/visiblePoll.js";
    import {searchMirror} from "#lib/url/searchSync.js";

    let params = $state<ProjectionParams>(defaultProjectionParams());
    let restored = $state(false);

    onMount(() => {
        if (window.location.search !== "") Object.assign(params, searchToProjectionParams(window.location.search, defaultProjectionParams()));
        restored = true;
        // The open file is not part of the main journal, so the layout's
        // journal poll never notices it change. Same cadence, its own check.
        const stopDiskWatch = pollWhileVisible(() => void syncWithDisk(), DISK_POLL_MS);
        return () => {
            stopDiskWatch();
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
    // Not the seed's own view: a scenario reopened from a file never reads the
    // seed, and gating on it alone left a spinner over the table forever.
    const tableView = $derived(scenarioTableView(scenarioStore.adopted, seed.status, seed.value !== null));

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
        // At the page's depth: the slider is where a user says how finely they
        // plan, and the seed is where that plan starts. Only where it STARTS —
        // see `canReseed`.
        await scenarioStore.ensureSeeded(url, settings.serverNonce, params.depth);
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
        diskNotice = null;
        scenarioStore.adopt(outcome.file.scenario);
        return true;
    }

    // --- Edits made outside Ledgeline -----------------------------------------

    /** The journal poll's own cadence (`startPolling`). */
    const DISK_POLL_MS = 5_000;

    /**
     * What the last disk check found that the user should hear about: the file
     * changed under unsaved edits, or it can no longer be read. Null otherwise.
     */
    let diskNotice = $state<{kind: "changed"} | {kind: "failed"; message: string} | null>(null);

    /**
     * Pick up an edit made to the open file in another program.
     *
     * With no unsaved work the table simply follows the file — it is adopted
     * like any load, and the projection below it stays on screen, labelled
     * stale, until the recompute lands (amendment 23). With unsaved work NOTHING
     * is replaced: the page says the file moved and offers to reload it, and
     * the held revision stays the old one, so a Save is refused by the engine
     * with a 409 rather than overwriting the other program's edit.
     */
    async function syncWithDisk(): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        const check = await fileStore.checkDisk(url);
        switch (check.kind) {
            case "skipped":
                return;
            case "unchanged":
                diskNotice = null;
                return;
            case "changed":
                if (scenarioStore.dirty) {
                    diskNotice = {kind: "changed"};
                    return;
                }
                fileStore.adopted(check.file);
                scenarioStore.adopt(check.file.scenario);
                diskNotice = null;
                return;
            case "failed":
                // A dropped connection is the app bar's to report, not this
                // file's; what is worth saying is that the FILE is now unreadable
                // or gone, and that the table on screen is untouched by it.
                if (check.failure.kind === "network" || check.failure.kind === "unavailable") return;
                diskNotice = {kind: "failed", message: check.failure.message};
                return;
        }
    }

    function reloadFromDisk(): void {
        const url = settings.serverUrl;
        const id = fileStore.currentId;
        if (url !== null && id !== null) void openFile(url, id);
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
            diskNotice = null;
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
            diskNotice = null;
        } else {
            fileError = outcome.failure.message;
        }
    }

    // --- The projection ------------------------------------------------------

    // The Accounts and Journal readings ride on the run only while their tab
    // is open: no other tab shows them, and every edit re-runs the projection.
    const projectionWindow = $derived<ProjectionWindow>({
        interval: params.interval,
        count: params.count,
        lookback: params.lookback,
        depth: params.depth,
        accounts: params.tab === "accounts" ? {empty: params.empty} : null,
        journal: params.tab === "journal",
    });
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
        if (url !== null) void scenarioStore.reseed(url, params.depth);
    }

    /**
     * Whether to offer "Re-seed at depth N".
     *
     * The seed is a STARTING POINT, so moving the depth slider never re-seeds
     * by itself: once rows exist they sit at whatever level the user put them,
     * leaves beside parents, and the slider only changes how the statements
     * below roll them up. But a user who has not touched the seed yet and moves
     * the slider is very likely asking for a finer (or coarser) start, so it is
     * offered — only then. Never over an edit, and never over a file, both of
     * which it would silently discard.
     */
    const canReseed = $derived(
        scenarioStore.seedDepth !== null && !scenarioStore.dirty && fileStore.currentId === null && scenarioStore.seedDepth !== params.depth
    );
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

    <!-- The open file was changed by another program. Never resolved by
         replacing unsaved work: the user chooses. -->
    {#if diskNotice !== null}
        <div class="alert flex flex-col items-start gap-2 rounded-box px-3 py-2 text-sm alert-warning" role="alert" data-testid="projection-disk-notice">
            {#if diskNotice.kind === "changed"}
                <span>
                    This scenario's file was changed outside Ledgeline since you opened it. Your edits here are not saved, and Save will refuse to overwrite the
                    other change — reload the file to see it (discarding your edits), or use Save as to keep yours in a new file.
                </span>
                <button type="button" class="btn btn-sm" disabled={busy} onclick={reloadFromDisk} data-testid="projection-disk-reload">Reload from disk</button>
            {:else}
                <span>The scenario's file could not be re-read, so the table shows it as it was: {diskNotice.message}</span>
            {/if}
        </div>
    {/if}

    <!-- The what-if. Seeded once from the journal: the budget's own `~` rules,
         plus one monthly line per category the budget does not mention. -->
    <AsyncSection
        view={tableView}
        value={scenarioStore.adopted ? scenarioStore.scenario : seed.value}
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
        {#if canReseed}
            <div class="flex items-center gap-2 text-xs text-base-content/70" data-testid="projection-reseed">
                <span>The starting rows were seeded at depth {scenarioStore.seedDepth}.</span>
                <button type="button" class="btn btn-outline btn-xs" onclick={retrySeed}>Re-seed at depth {params.depth}</button>
            </div>
        {/if}
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
        <!-- Always one line tall, text or not. Picking a reading scrolls the tab
             strip below to the top of the window, and when this line came and
             went with the run (every switch into or out of Accounts/Journal
             starts one) it moved the strip by its height plus the gap, so a
             scroll taken on a slow machine landed a line off. A live region
             that is always present is also the one screen readers announce. -->
        <p class="min-h-4 text-xs text-base-content/60" role="status" aria-live="polite" data-testid="projection-stale">
            {stale ? "Recomputing for your latest edit…" : ""}
        </p>
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
                    <ProjectionReports
                        bind:tab={params.tab}
                        bind:emptyAccounts={params.empty}
                        projection={held}
                        interval={params.interval}
                        {styles}
                        fallbackCommodity={commodity}
                        scenarioName={scenarioStore.scenario.name}
                    />
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
