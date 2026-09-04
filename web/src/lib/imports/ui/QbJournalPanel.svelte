<script lang="ts">
    // The QuickBooks Online Journal import screen (WP-17 Phase C): a staged
    // export's parsed groups, the accounts nothing maps yet, and a commit that
    // writes straight into the journal — no CSV, no rules file, no dry run.
    //
    // Mounted INSTEAD OF `StagedPanel`/`DryRunPanel`/`ResultPanel` the moment
    // `NewTransactionsPanel` sees `staged.format === "quickbooks-journal"` —
    // see `qbJournalModel.isQuickbooksJournalStage`, the ONE branch that
    // decision is allowed to make (the plan's Phase C contract: no new
    // client-side detection, no confidence UI, no confirmation step; the
    // engine already decided, server-side, at `POST /api/import/stage`).
    //
    // Every judgement here — whether the commit button may be pressed, what
    // the date-format notice says, which files need a re-sort offer — is a
    // tested function in `qbJournalModel.ts`; this file reads the store and
    // places values on the screen.
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {canCommitQbJournal, dateFormatNotice, filesNeedingSort, qbIdMatchesSummary, qbReorderOffer} from "../qbJournalModel";
    import {qbJournalStore} from "../qbJournalStore.svelte";
    import QbUnmappedAccounts from "./QbUnmappedAccounts.svelte";

    let {stageId, accountNames}: {stageId: string; accountNames: string[]} = $props();

    // Keyed on `stageId` (a prop) and `settings.serverUrl` (a store), neither
    // of which this effect writes — so it cannot become the self-feeding loop
    // `AliasPanel.svelte`'s own history warns about. `ensurePreview` itself is
    // idempotent per stage, so a re-render that changes neither dependency
    // re-runs this harmlessly.
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== null) void qbJournalStore.ensurePreview(url, stageId);
    });

    function retryPreview(): void {
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
        await qbJournalStore.saveMappings(url, stageId, preview.unmappedAccounts);
    }

    async function commit(): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        await qbJournalStore.commitStage(url, stageId);
    }

    function resort(journalId: string): void {
        const url = settings.serverUrl;
        if (url !== null) void qbJournalStore.resortFile(url, journalId);
    }
</script>

<AsyncSection
    view={qbJournalStore.previewView}
    value={qbJournalStore.preview}
    error={qbJournalStore.previewError}
    testid="qb-preview-error"
    label="this QuickBooks Journal export"
    loadingLabel="Reading the QuickBooks Journal export"
    onRetry={retryPreview}
>
    {#snippet children(preview)}
        <div class="flex flex-col gap-3" data-testid="qb-journal-panel">
            <section class="flex flex-col gap-2 rounded-box border border-base-content/10 p-3" aria-label="QuickBooks Journal export">
                <h2 class="text-sm font-semibold tracking-tight">QuickBooks Journal export</h2>
                <p class="text-xs text-base-content/60" data-testid="qb-export-help">
                    From QuickBooks Online: <strong>Reports → Journal</strong> → choose a time frame → optionally <strong>Customize</strong> to add
                    <strong>Vendor</strong>, <strong>Customer</strong> and <strong>Class</strong> → <strong>Export to Excel</strong>. Re-downloading the same or
                    overlapping time frame — including "All Dates" — is safe: transactions already imported are recognized by their QuickBooks id and never
                    duplicated.
                </p>
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

            {#if qbJournalStore.commitRequested}
                <AsyncSection
                    view={qbJournalStore.commitView}
                    value={qbJournalStore.commitResult}
                    error={qbJournalStore.commitError}
                    testid="qb-commit-error"
                    label="the QuickBooks import"
                    loadingLabel="Writing the import"
                    onRetry={commit}
                >
                    {#snippet children(result)}
                        <section class="flex flex-col gap-3 rounded-box border border-success/40 p-3" aria-label="Import result" data-testid="qb-result">
                            <h2 class="text-sm font-semibold tracking-tight">Done</h2>
                            <p class="text-sm" data-testid="qb-imported">
                                Imported {result.imported} transaction{result.imported === 1 ? "" : "s"}.
                            </p>
                            {#if qbIdMatchesSummary(result.idMatches) !== null}
                                <p class="text-sm text-warning">{qbIdMatchesSummary(result.idMatches)}</p>
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
                                    <p class="text-sm text-success" data-testid="qb-sorted">
                                        {file.journalId} re-sorted: {qbJournalStore.sortMovedFor(file.journalId)} transaction{qbJournalStore.sortMovedFor(
                                            file.journalId
                                        ) === 1
                                            ? ""
                                            : "s"} moved.
                                    </p>
                                {/if}
                                {#if qbJournalStore.sortErrorFor(file.journalId) !== null}
                                    <p class="text-sm text-error" role="alert" data-testid="qb-sort-error">{qbJournalStore.sortErrorFor(file.journalId)}</p>
                                {/if}
                            {/each}
                        </section>
                    {/snippet}
                </AsyncSection>
            {/if}
        </div>
    {/snippet}
</AsyncSection>
