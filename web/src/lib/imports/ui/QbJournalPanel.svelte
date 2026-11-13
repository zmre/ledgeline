<script lang="ts">
    // The whole-book import screen (WP-17 Phase C; every source since plan
    // 39 — QuickBooks Journal, QIF …): a staged export's parsed groups, the
    // accounts nothing maps yet, and a commit that writes straight into the
    // journal — no CSV, no rules file, no dry run.
    //
    // Mounted INSTEAD OF `StagedPanel`/`DryRunPanel`/`ResultPanel` the moment
    // `NewTransactionsPanel` sees a whole-book `staged.format` (`quickbooks-
    // journal`, `qif` …) — see `qbJournalModel.isBookImportStage`, the ONE
    // branch that decision is allowed to make (the plan's Phase C contract: no new
    // client-side detection, no confidence UI, no confirmation step; the
    // engine already decided, server-side, at `POST /api/import/stage`).
    //
    // Every judgement here — whether the commit button may be pressed, what
    // the date-format notice says, which files need a re-sort offer — is a
    // tested function in `qbJournalModel.ts`; this file reads the store and
    // places values on the screen.
    //
    // Review (plan 32 D30): once every account is mapped, Review Unknowns and
    // Review All open the overlay over the built transactions, exactly as the
    // CSV dry run does. Its saves are held as overrides keyed by Trans # and
    // ride on the Import. There is no "Remember as a rule" here: an alias maps a
    // whole QuickBooks account, so there is no payee rule to write.
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import ImportMatches from "#lib/matching/ui/ImportMatches.svelte";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {
        bookChangedIn,
        bookExportTitle,
        bookSourceLabel,
        canCommitQbJournal,
        dateFormatNotice,
        filesNeedingSort,
        QB_JOURNAL_FORMAT,
        qbIdMatchesSummary,
        qbReorderOffer,
    } from "../qbJournalModel";
    import {qbJournalStore} from "../qbJournalStore.svelte";
    import {stagedRunCounts, stagedRunItems, type StagedRunKind} from "../stagedReview";
    import QbUnmappedAccounts from "./QbUnmappedAccounts.svelte";
    import StagedEditsNote from "./StagedEditsNote.svelte";
    import StagedReviewButtons from "./StagedReviewButtons.svelte";
    import {reveal, revealArm} from "#lib/ui/reveal.js";

    let {
        stageId,
        format = QB_JOURNAL_FORMAT,
        accountNames,
    }: {
        stageId: string;
        /** The stage's `format`: which source this is, before the preview has said so too. */
        format?: string;
        accountNames: string[];
    } = $props();

    const title = $derived(bookExportTitle(format));
    const sourceLabel = $derived(bookSourceLabel(format));

    // Keyed on `stageId` (a prop) and `settings.serverUrl` (a store), neither
    // of which this effect writes — so it cannot become the self-feeding loop
    // `AliasPanel.svelte`'s own history warns about. `ensurePreview` itself is
    // idempotent per stage, so a re-render that changes neither dependency
    // re-runs this harmlessly.
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== null) void qbJournalStore.ensurePreview(url, stageId);
    });

    /**
     * The host's reveal arm (`NewTransactionsPanel`), armed by every press here
     * too: coming back to a QuickBooks stage that was already on screen brings a
     * fresh, disarmed arm, and the Import after it must still be shown.
     */
    const arm = revealArm();

    function retryPreview(): void {
        arm?.arm();
        const url = settings.serverUrl;
        if (url !== null) void qbJournalStore.refreshPreview(url, stageId);
    }

    function onDraft(account: string, value: string): void {
        qbJournalStore.setDraft(account, value);
    }

    async function saveMappings(): Promise<void> {
        const url = settings.serverUrl;
        const preview = qbJournalStore.preview;
        if (url === null || preview === null) return;
        arm?.arm();
        await qbJournalStore.saveMappings(url, stageId, preview.unmappedAccounts);
    }

    async function commit(): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        arm?.arm();
        await qbJournalStore.commitStage(url, stageId);
    }

    const counts = $derived(stagedRunCounts(qbJournalStore.stagedTxns, qbJournalStore.overrides));

    function review(kind: StagedRunKind): void {
        arm?.arm();
        reviewLaunch.openStaged({
            kind,
            items: stagedRunItems(kind, qbJournalStore.stagedTxns, qbJournalStore.overrides),
            sink: qbJournalStore.reviewSink,
            rules: null,
            writeLabel: "Import",
        });
    }

    function resort(journalId: string): void {
        const url = settings.serverUrl;
        if (url === null) return;
        arm?.arm();
        void qbJournalStore.resortFile(url, journalId);
    }
</script>

<!-- The preview is what the drop made appear: shown once it replaces its spinner. -->
<div use:reveal={{arm, key: qbJournalStore.previewView}}>
    <AsyncSection
        view={qbJournalStore.previewView}
        value={qbJournalStore.preview}
        error={qbJournalStore.previewError}
        testid="qb-preview-error"
        label={`this ${title}`}
        loadingLabel={`Reading the ${title}`}
        onRetry={retryPreview}
    >
        {#snippet children(preview)}
            <div class="flex flex-col gap-3" data-testid="qb-journal-panel">
                <section class="flex flex-col gap-2 rounded-box border border-base-content/10 p-3" aria-label={title}>
                    <h2 class="text-sm font-semibold tracking-tight">{title}</h2>
                    {#if format === QB_JOURNAL_FORMAT}
                        <p class="text-xs text-base-content/60" data-testid="qb-export-help">
                            From QuickBooks Online: <strong>Reports → Journal</strong> → choose a time frame → optionally <strong>Customize</strong> to add
                            <strong>Vendor</strong>, <strong>Customer</strong> and <strong>Class</strong> → <strong>Export to Excel</strong>. Re-downloading the
                            same or overlapping time frame — including "All Dates" — is safe: transactions already imported are recognized by their QuickBooks
                            id and never duplicated.
                        </p>
                    {:else if format === "qif"}
                        <p class="text-xs text-base-content/60" data-testid="qb-export-help">
                            Export QIF with every account selected (Banktivity: <strong>File → Export → QIF</strong>; Quicken:
                            <strong>File → Export → QIF file</strong>). Accounts, categories, splits, transfers and classes come through; a transfer written in
                            both accounts is kept once. Importing the same file again is safe: each transaction gets an id from its date, account, amount and
                            payee, so ones already imported are skipped.
                        </p>
                    {:else}
                        <p class="text-xs text-base-content/60" data-testid="qb-export-help">
                            Importing the same export again is safe: transactions already imported are recognized by their id and never duplicated.
                        </p>
                    {/if}
                    <p class="text-sm text-base-content/70" data-testid="qb-summary">
                        {preview.transactionCount} transaction{preview.transactionCount === 1 ? "" : "s"} from {preview.postingCount} posting{preview.postingCount ===
                        1
                            ? ""
                            : "s"}.
                    </p>
                    {#if dateFormatNotice(preview.dateFormat) !== null}
                        <p class="text-sm text-warning" role="status" data-testid="qb-date-notice">{dateFormatNotice(preview.dateFormat)}</p>
                    {/if}
                    {#if preview.sample.length > 0}
                        <span class="rounded bg-base-200 p-2 font-mono text-xs whitespace-pre-wrap" data-testid="qb-sample"
                            >{preview.sample
                                .map((txn) => [`${txn.date} ${txn.description}`, ...txn.postings.map((posting) => `    ${posting}`)].join("\n"))
                                .join("\n\n")}</span
                        >
                    {/if}
                </section>

                {#if preview.unmappedAccounts.length > 0}
                    <QbUnmappedAccounts
                        accounts={preview.unmappedAccounts}
                        sourceLabel={preview.sourceLabel}
                        {accountNames}
                        draftFor={(account) => qbJournalStore.draftFor(account)}
                        {onDraft}
                        saving={qbJournalStore.mappingSaving}
                        error={qbJournalStore.mappingError}
                        onSave={saveMappings}
                    />
                {/if}

                {#if qbIdMatchesSummary(preview.idMatches) !== null}
                    <p class="text-sm text-warning" data-testid="qb-id-matches-summary">{qbIdMatchesSummary(preview.idMatches)}</p>
                {/if}

                {#if preview.unmappedAccounts.length === 0 && preview.txns.length > 0}
                    <StagedReviewButtons unknowns={counts.unknowns} all={counts.all} writeLabel="Import" onReview={review} />
                {/if}

                <div class="flex flex-wrap items-center gap-3" data-testid="qb-actions">
                    <button
                        type="button"
                        class="btn btn-primary"
                        disabled={!canCommitQbJournal(preview) || qbJournalStore.committing}
                        onclick={commit}
                        data-testid="qb-commit"
                    >
                        {#if qbJournalStore.committing}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Import
                    </button>
                    {#if !canCommitQbJournal(preview)}
                        <span class="text-sm text-warning" role="status">Map every account above before importing.</span>
                    {/if}
                </div>
                <StagedEditsNote
                    overrideCount={qbJournalStore.overrideCount}
                    journalEditCount={qbJournalStore.journalEditCount}
                    flagged={qbJournalStore.flaggedOverrides}
                    busy={qbJournalStore.committing}
                    onDiscard={(refs) => {
                        arm?.arm();
                        qbJournalStore.discardOverrides(refs);
                    }}
                />
                <label class="flex items-start gap-2 text-sm" data-testid="qb-update-changed-label">
                    <input
                        type="checkbox"
                        class="checkbox mt-0.5 checkbox-sm"
                        checked={qbJournalStore.updateChanged}
                        onchange={(event) => qbJournalStore.setUpdateChanged(event.currentTarget.checked)}
                        disabled={qbJournalStore.committing}
                        data-testid="qb-update-changed"
                    />
                    <span>
                        Update transactions that changed in {bookChangedIn(format)}
                        <span class="block text-xs text-base-content/60">
                            Already-imported transactions the export now shows differently are replaced with the export's version, keeping their cleared status
                            and notes. Leave this off if you re-categorize imported transactions by hand — those edits would be overwritten.
                        </span>
                    </span>
                </label>

                {#if qbJournalStore.commitRequested}
                    <div use:reveal={{arm, key: qbJournalStore.commitView}} data-testid="qb-result-reveal">
                        <AsyncSection
                            view={qbJournalStore.commitView}
                            value={qbJournalStore.commitResult}
                            error={qbJournalStore.commitError}
                            testid="qb-commit-error"
                            label={`the ${sourceLabel} import`}
                            loadingLabel="Writing the import"
                            onRetry={commit}
                        >
                            {#snippet children(result)}
                                <section
                                    class="flex flex-col gap-3 rounded-box border border-success/40 p-3"
                                    aria-label="Import result"
                                    data-testid="qb-result"
                                >
                                    <h2 class="text-sm font-semibold tracking-tight">Done</h2>
                                    <p class="text-sm" data-testid="qb-imported">
                                        Imported {result.imported} transaction{result.imported === 1 ? "" : "s"}.
                                    </p>
                                    <!-- Find Matches (plan 35 D9 "Import"): a whole-book import inserts by date,
                                         so it offers the whole journal's pairs, not only the new ones. -->
                                    <ImportMatches since={null} imported={result.imported} />
                                    {#if result.updated > 0}
                                        <p class="text-sm" data-testid="qb-updated">
                                            Updated {result.updated} changed transaction{result.updated === 1 ? "" : "s"} from the export.
                                        </p>
                                    {/if}
                                    {#if qbIdMatchesSummary(result.idMatches, result.updated) !== null}
                                        <p class="text-sm text-warning">{qbIdMatchesSummary(result.idMatches, result.updated)}</p>
                                    {/if}

                                    {#each filesNeedingSort(result.ordering) as file (file.journalId)}
                                        {#if qbJournalStore.sortMovedFor(file.journalId) === null}
                                            <div
                                                class="alert flex flex-col items-start gap-2 rounded-box py-2 text-sm alert-warning"
                                                role="alert"
                                                data-testid="qb-out-of-order"
                                            >
                                                <span>{qbReorderOffer(file)}</span>
                                                <ul class="max-h-48 overflow-auto font-mono text-xs">
                                                    {#each file.moves as move (`${move.fromLine}-${move.toLine}`)}
                                                        <li>{move.date} {move.description} — line {move.fromLine} → {move.toLine}</li>
                                                    {/each}
                                                </ul>
                                                <button
                                                    type="button"
                                                    class="btn btn-sm"
                                                    disabled={qbJournalStore.sortingJournalId === file.journalId}
                                                    onclick={() => resort(file.journalId)}
                                                    data-testid="qb-sort"
                                                >
                                                    {#if qbJournalStore.sortingJournalId === file.journalId}<span class="loading loading-xs loading-spinner"
                                                        ></span>{/if}
                                                    Re-sort {file.journalId} by date
                                                </button>
                                            </div>
                                        {/if}
                                        {#if qbJournalStore.sortMovedFor(file.journalId) !== null}
                                            <p class="text-sm text-success" use:reveal={{arm}} data-testid="qb-sorted">
                                                {file.journalId} re-sorted: {qbJournalStore.sortMovedFor(file.journalId)} transaction{qbJournalStore.sortMovedFor(
                                                    file.journalId
                                                ) === 1
                                                    ? ""
                                                    : "s"} moved.
                                            </p>
                                        {/if}
                                        {#if qbJournalStore.sortErrorFor(file.journalId) !== null}
                                            <p class="text-sm text-error" role="alert" use:reveal={{arm}} data-testid="qb-sort-error">
                                                {qbJournalStore.sortErrorFor(file.journalId)}
                                            </p>
                                        {/if}
                                    {/each}
                                </section>
                            {/snippet}
                        </AsyncSection>
                    </div>
                {/if}
            </div>
        {/snippet}
    </AsyncSection>
</div>
