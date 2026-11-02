<script lang="ts">
    // The New Transactions tab: drop a statement, see what it becomes, choose
    // how to read it, and watch every step before anything is written.
    //
    // This component is the ORCHESTRATOR and holds no decisions. It reads the
    // capabilities probe, asks `visibleSections` which parts of the flow exist,
    // and hands each one its slice of the store. Every judgement it looks like
    // it is making — whether hledger gates the screen, which CSV path follows a
    // chosen rules file, what a `ConvertNote` says, whether a balance
    // reconciles — is a tested function in `importModel.ts`.
    //
    // Which leaves exactly one thing that can go wrong here, and it did: asking
    // the WRONG tested function. `NewTransactionsPanel.svelte.test.ts` and its
    // `.staged.` sibling mount this file for that, in the `components` vitest
    // project.
    //
    // Review before writing (plan 32 D23–D26, D30) starts here too: the dry run
    // and the QuickBooks panel ask for a staged run, and the overlay is mounted
    // HERE, outside both, because a rule saved from inside it re-runs the dry
    // run — which unmounts the dry-run panel while it loads.
    //
    // The capabilities probe is the outer async surface and comes FIRST for a
    // reason: it is what says whether hledger can be run, and with no usable
    // hledger every affordance below it is an invitation to press a button that
    // cannot work. That is the state a new user hits.
    import {onDestroy} from "svelte";
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
    import ReviewOverlay from "#lib/review/ui/ReviewOverlay.svelte";
    import {actionRunsDryRun, importAction, shows, visibleSections} from "../importModel";
    import {importStore} from "../importStore.svelte";
    import {isQuickbooksJournalStage} from "../qbJournalModel";
    import type {StagedRef} from "../reviewTypes";
    import {stagedRunCounts, stagedRunItems, type StagedReviewProps, type StagedRunKind} from "../stagedReview";
    import DropTarget from "./DropTarget.svelte";
    import DryRunPanel from "./DryRunPanel.svelte";
    import HledgerBanner from "./HledgerBanner.svelte";
    import QbJournalPanel from "./QbJournalPanel.svelte";
    import ResultPanel from "./ResultPanel.svelte";
    import StagedPanel from "./StagedPanel.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {provideRevealArm, reveal} from "#lib/ui/reveal.js";

    const sections = $derived(
        visibleSections({
            capabilitiesLoaded: importStore.capabilities !== null,
            hledgerAvailable: importStore.capabilities?.hledger.available === true,
            editable: importStore.capabilities?.editable === true,
            staged: importStore.hasStagedOutcome,
            dryRunRequested: importStore.dryRunRequested,
            committed: importStore.writeRequested,
        })
    );

    /**
     * The ONE branch WP-17 Phase C's contract allows: `POST /api/import/stage`
     * already decided, server-side, whether this upload is a QuickBooks
     * Journal export — `staged.format` is `"quickbooks-journal"` XOR an
     * ordinary CSV/spreadsheet format. True only once a successful stage has
     * landed (`importStore.staged` is null while loading or on a staging
     * failure), so those two cases keep falling through to `StagedPanel`'s own
     * `AsyncSection`, exactly like the CSV path's loading/error UI.
     */
    const qbJournal = $derived(isQuickbooksJournalStage(importStore.staged));

    /**
     * Every section below that a press makes appear scrolls itself into view
     * (`reveal`), and THIS is what arms it: each handler that starts something
     * arms first. Fresh per mount, so a dry run restored on coming back to the
     * tab stays where it is. The panels inside (`StagedPanel`, `DryRunPanel`,
     * `QbJournalPanel`, …) take it from context.
     */
    const arm = provideRevealArm();

    /** `run`, once the reveal arm is armed: for every handler a press reaches. */
    function pressed<A extends unknown[]>(run: (...args: A) => void): (...args: A) => void {
        return (...args) => {
            arm.arm();
            run(...args);
        };
    }

    function retryCapabilities(): void {
        const url = settings.serverUrl;
        if (url !== null) void importStore.reloadCapabilities(url);
    }

    /**
     * Open a staged run over the dry run's transactions (D23). Closing it after
     * any save re-runs the dry run, so the preview shows the bytes the edits
     * will write.
     */
    function reviewCsv(kind: StagedRunKind): void {
        reviewLaunch.openStaged({
            kind,
            items: stagedRunItems(kind, importStore.stagedTxns, importStore.overrides),
            sink: importStore.reviewSink,
            rules: importStore.ruleSource(),
            onClose: (saved) => {
                if (saved) void importStore.refreshDryRun();
            },
        });
    }

    const csvReview = $derived.by((): StagedReviewProps => {
        const counts = stagedRunCounts(importStore.stagedTxns, importStore.overrides);
        return {
            ...counts,
            overrideCount: importStore.overrideCount,
            journalEditCount: importStore.journalEditCount,
            flagged: importStore.flaggedOverrides,
            onReview: pressed(reviewCsv),
            onDiscard: pressed((refs: readonly StagedRef[]) => void importStore.discardOverrides(refs)),
        };
    });

    // Leaving the tab mid-run closes the run: the overlay is unmounted with this
    // panel, and a launch left behind would reopen it on the way back.
    onDestroy(() => {
        if (reviewLaunch.staged !== null) reviewLaunch.close(null);
    });

    /** `Test Import` dry-runs first; `Save CSV` has nothing to propose and writes. */
    function submit(): void {
        if (actionRunsDryRun(importAction(importStore.selectedRulesId))) void importStore.runDryRun();
        else void importStore.writeChanges();
    }
</script>

<AsyncSection
    view={importStore.capabilitiesView}
    value={importStore.capabilities}
    error={importStore.capabilitiesError}
    testid="imports-capabilities-error"
    label="what this server can import"
    loadingLabel="Checking what this server can import"
    onRetry={retryCapabilities}
>
    {#snippet children(capabilities)}
        <!-- `imports-new` anchors "the New Transactions panel is what rendered",
             which is a different claim from any one section being visible. Every
             section below is conditional — a machine without hledger sees only
             the banner, a server with no journal bound sees only the read-only
             notice — so a test that asserted on the drop target would be
             asserting on the test machine's hledger installation. -->
        <div class="flex flex-col gap-3" data-testid="imports-new">
            {#if shows(sections, "hledgerBanner")}
                <HledgerBanner
                    {capabilities}
                    initialPath={importStore.prefs?.hledgerPath ?? null}
                    saving={importStore.prefsSaving}
                    error={importStore.prefsError}
                    onSave={(path) => void importStore.saveHledgerPath(path)}
                    onRecheck={retryCapabilities}
                />
            {/if}

            {#if shows(sections, "readOnlyBanner")}
                <div class="alert items-start rounded-box py-3 text-sm alert-info" role="status" data-testid="imports-read-only">
                    <span>
                        Ledgeline has no journal file bound to an editor here, so there is nowhere for an import to go. Start the engine with a journal file and
                        this screen becomes usable — everything else on the Imports tab still works read-only.
                    </span>
                </div>
            {/if}

            {#if shows(sections, "drop")}
                <!-- `stagingInFlight`, never `stagedView === "loading"`: the
                     view collapses "nothing has been asked for" into "loading",
                     so the drop target span before a file existed. -->
                <DropTarget
                    formats={capabilities.formats}
                    busy={importStore.stagingInFlight}
                    rejection={importStore.rejection}
                    onFile={pressed((file: File) => void importStore.offerFile(file))}
                />
            {/if}

            {#if qbJournal && importStore.staged !== null}
                <!-- WP-17 Phase C: a different panel entirely — no CSV, no
                     rules file, no dry run. `DryRunPanel`/`ResultPanel` below
                     stay dark for this stage because `dryRunRequested`/
                     `writeRequested` are only ever set by the ordinary CSV
                     flow's own buttons, which this branch never presses. -->
                <QbJournalPanel stageId={importStore.staged.stageId} accountNames={journal.accountNames} />
            {:else if importStore.hasStagedOutcome}
                <!-- Converting, then the preview and candidates: both are what
                     the drop made appear. -->
                <div use:reveal={{arm, key: importStore.stagedView}} data-testid="imports-staged-reveal">
                    <StagedPanel
                        {sections}
                        view={importStore.stagedView}
                        staged={importStore.staged}
                        error={importStore.stagedError}
                        journals={capabilities.journals}
                        accountNames={journal.accountNames}
                        selectedRulesId={importStore.selectedRulesId}
                        creating={importStore.creating}
                        createDraft={importStore.draft}
                        createForm={importStore.createForm}
                        createId={importStore.createId}
                        createDrafting={importStore.createDrafting}
                        createSaving={importStore.createSaving}
                        createError={importStore.createError}
                        createdId={importStore.createdId}
                        onCreateOpen={pressed(() => void importStore.openCreate())}
                        onCreateClose={() => importStore.closeCreate()}
                        onCreateId={(value) => importStore.setCreateId(value)}
                        onCreateItems={(items) => importStore.setCreateItems(items)}
                        onCreateSave={pressed(() => void importStore.saveCreate())}
                        onCreateRetry={pressed(() => void importStore.redraft())}
                        csvPath={importStore.csvPath}
                        journalId={importStore.journalId}
                        balance={importStore.balance}
                        balanceAccount={importStore.balanceAccount}
                        writeAssertion={importStore.writeAssertion}
                        busy={importStore.formBusy}
                        onRetry={pressed(() => void importStore.retryStage())}
                        onStatement={pressed((index: number) => void importStore.chooseStatement(index))}
                        onSelect={(id) => importStore.selectCandidate(id)}
                        onCsvPath={(value) => importStore.setCsvPath(value)}
                        onJournal={(value) => importStore.setJournalId(value)}
                        onBalance={(value) => importStore.setBalance(value)}
                        onBalanceAccount={(value) => importStore.setBalanceAccount(value)}
                        onWriteAssertion={(value) => importStore.setWriteAssertion(value)}
                        onSubmit={pressed(submit)}
                    />
                </div>
            {/if}

            {#if shows(sections, "dryRun")}
                <!-- Keyed on the view: the spinner becomes the result (or the
                     error) in place, and a re-run after a rule was saved does
                     too — `revealDelta` never scrolls up to one already passed. -->
                <div use:reveal={{arm, key: importStore.dryRunView}} data-testid="imports-dry-run-reveal">
                    <DryRunPanel
                        view={importStore.dryRunView}
                        result={importStore.dryRun}
                        error={importStore.dryRunError}
                        aliases={capabilities.aliases}
                        writing={importStore.writeInFlight}
                        editable={capabilities.editable}
                        confWriting={importStore.confWriting}
                        confWritten={importStore.confWritten}
                        confError={importStore.confError}
                        onRetry={pressed(() => void importStore.runDryRun())}
                        onWrite={pressed(() => void importStore.writeChanges())}
                        onInstallConf={pressed((revision: string) => void importStore.installConfAliases(revision))}
                        review={csvReview}
                    />
                </div>
            {/if}

            {#if shows(sections, "result")}
                <div use:reveal={{arm, key: importStore.committedView}} data-testid="imports-result-reveal">
                    <ResultPanel
                        view={importStore.committedView}
                        result={importStore.committed}
                        error={importStore.committedError}
                        sorting={importStore.sorting}
                        sortMoved={importStore.sortMoved}
                        sortError={importStore.sortError}
                        onRetry={pressed(() => void importStore.writeChanges())}
                        onSort={pressed(() => void importStore.resort())}
                    />
                </div>
            {/if}
        </div>
    {/snippet}
</AsyncSection>

<!-- The review overlay over an import (plan 32 Phase 4), CSV or QuickBooks: mounted while a staged run is open. -->
{#if reviewLaunch.staged !== null}
    <ReviewOverlay kind={reviewLaunch.staged.kind} staged={reviewLaunch.staged} />
{/if}
