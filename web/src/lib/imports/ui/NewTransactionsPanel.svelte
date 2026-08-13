<script lang="ts">
    // The New Transactions tab: drop a statement, see what it becomes, choose
    // how to read it, and watch every step before anything is written.
    //
    // This component is the ORCHESTRATOR and holds no decisions. It reads the
    // capabilities probe, asks `visibleSections` which parts of the flow exist,
    // and hands each one its slice of the store. Every judgement it looks like
    // it is making — whether hledger gates the screen, which CSV path follows a
    // chosen rules file, what a `ConvertNote` says, whether a balance
    // reconciles — is a tested function in `importModel.ts`, because a
    // condition written in a `.svelte` file here is a condition that cannot be
    // tested at all.
    //
    // The capabilities probe is the outer async surface and comes FIRST for a
    // reason: it is what says whether hledger can be run, and with no usable
    // hledger every affordance below it is an invitation to press a button that
    // cannot work. That is the state a new user hits.
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {actionRunsDryRun, importAction, shows, visibleSections} from "../importModel";
    import {importStore} from "../importStore.svelte";
    import DropTarget from "./DropTarget.svelte";
    import DryRunPanel from "./DryRunPanel.svelte";
    import HledgerBanner from "./HledgerBanner.svelte";
    import ResultPanel from "./ResultPanel.svelte";
    import StagedPanel from "./StagedPanel.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    const sections = $derived(
        visibleSections({
            capabilitiesLoaded: importStore.capabilities !== null,
            hledgerAvailable: importStore.capabilities?.hledger.available === true,
            editable: importStore.capabilities?.editable === true,
            staged: importStore.hasStageRequest,
            dryRunRequested: importStore.dryRunRequested,
            committed: importStore.writeRequested,
        })
    );

    /** A dry run or a write is in flight — the form is frozen while either is. */
    const busy = $derived(importStore.dryRunView === "loading" || importStore.committedView === "loading");

    function retryCapabilities(): void {
        const url = settings.serverUrl;
        if (url !== null) void importStore.reloadCapabilities(url);
    }

    /** `Save and Import` dry-runs first; `Save CSV` has nothing to propose and writes. */
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
        <div class="flex flex-col gap-3">
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
                <div class="alert alert-info rounded-box items-start py-3 text-sm" role="status" data-testid="imports-read-only">
                    <span>
                        Ledgeline has no journal file bound to an editor here, so there is nowhere for an import to go. Start the engine with a journal file and
                        this screen becomes usable — everything else on the Imports tab still works read-only.
                    </span>
                </div>
            {/if}

            {#if shows(sections, "drop")}
                <DropTarget
                    formats={capabilities.formats}
                    busy={importStore.stagedView === "loading"}
                    rejection={importStore.rejection}
                    onFile={(file) => void importStore.offerFile(file)}
                />
            {/if}

            {#if importStore.hasStageRequest}
                <StagedPanel
                    {sections}
                    view={importStore.stagedView}
                    staged={importStore.staged}
                    error={importStore.stagedError}
                    journals={capabilities.journals}
                    accountNames={journal.accountNames}
                    selectedRulesId={importStore.selectedRulesId}
                    csvPath={importStore.csvPath}
                    journalId={importStore.journalId}
                    balance={importStore.balance}
                    balanceAccount={importStore.balanceAccount}
                    writeAssertion={importStore.writeAssertion}
                    {busy}
                    onRetry={() => void importStore.retryStage()}
                    onSelect={(id) => importStore.selectCandidate(id)}
                    onCsvPath={(value) => importStore.setCsvPath(value)}
                    onJournal={(value) => importStore.setJournalId(value)}
                    onBalance={(value) => importStore.setBalance(value)}
                    onBalanceAccount={(value) => importStore.setBalanceAccount(value)}
                    onWriteAssertion={(value) => importStore.setWriteAssertion(value)}
                    onSubmit={submit}
                />
            {/if}

            {#if shows(sections, "dryRun")}
                <DryRunPanel
                    view={importStore.dryRunView}
                    result={importStore.dryRun}
                    error={importStore.dryRunError}
                    writing={importStore.committedView === "loading"}
                    onRetry={() => void importStore.runDryRun()}
                    onWrite={() => void importStore.writeChanges()}
                />
            {/if}

            {#if shows(sections, "result")}
                <ResultPanel
                    view={importStore.committedView}
                    result={importStore.committed}
                    error={importStore.committedError}
                    sorting={importStore.sorting}
                    sortMoved={importStore.sortMoved}
                    sortError={importStore.sortError}
                    onRetry={() => void importStore.writeChanges()}
                    onSort={() => void importStore.resort()}
                />
            {/if}
        </div>
    {/snippet}
</AsyncSection>
