<script lang="ts">
    // Everything the staged file reveals: the preview, the ranked candidates,
    // the destinations, the optional balance, and the one action button.
    //
    // Which of those exist is `visibleSections`'s decision, not this file's —
    // the `sections` prop is the machine's output and this component only asks
    // whether a name is in it. That split is the same one `model.ts` makes for
    // the rules editor: a condition named and answered in a pure function is
    // tested by calling it, while the same condition written here is testable
    // only by building a whole screen that reaches it. (When this was written
    // the latter was not possible at all — see `importModel.ts`. It is now, in
    // the `components` vitest project, and `NewTransactionsPanel.staged.svelte.test.ts`
    // uses it to check that the values this panel is HANDED are the right ones.)
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {actionBlocker, actionLabel, importAction, shows, validateCsvPath, type ImportSection} from "../importModel";
    import type {JournalTarget, StagedFile} from "../importTypes";
    import BalanceField from "./BalanceField.svelte";
    import CandidateList from "./CandidateList.svelte";
    import DestinationForm from "./DestinationForm.svelte";
    import PreviewTable from "./PreviewTable.svelte";

    let {
        sections,
        view,
        staged,
        error,
        journals,
        accountNames,
        selectedRulesId,
        csvPath,
        journalId,
        balance,
        balanceAccount,
        writeAssertion,
        busy,
        onRetry,
        onSelect,
        onCsvPath,
        onJournal,
        onBalance,
        onBalanceAccount,
        onWriteAssertion,
        onSubmit,
    }: {
        sections: readonly ImportSection[];
        view: import("$lib/stores/loadState").DataView;
        staged: StagedFile | null;
        error: Error | null;
        journals: readonly JournalTarget[];
        accountNames: string[];
        selectedRulesId: string | null;
        csvPath: string;
        journalId: string | null;
        balance: string;
        balanceAccount: string;
        writeAssertion: boolean;
        /** A dry run or a write is in flight, so the form is frozen and the button is not pressable. */
        busy: boolean;
        onRetry: () => void;
        onSelect: (id: string | null) => void;
        onCsvPath: (value: string) => void;
        onJournal: (value: string | null) => void;
        onBalance: (value: string) => void;
        onBalanceAccount: (value: string) => void;
        onWriteAssertion: (value: boolean) => void;
        onSubmit: () => void;
    } = $props();

    const action = $derived(importAction(selectedRulesId));
    const draft = $derived({csvPath, journalId, balance, balanceAccount});
    const blocker = $derived(actionBlocker(action, draft));
</script>

<AsyncSection {view} value={staged} {error} testid="imports-stage-error" label="that file" loadingLabel="Converting the file" {onRetry}>
    {#snippet children(file)}
        <div class="flex flex-col gap-3">
            {#if shows(sections, "preview")}
                <PreviewTable staged={file} />
            {/if}

            {#if shows(sections, "candidates")}
                <CandidateList candidates={file.candidates} selectedId={selectedRulesId} disabled={busy} {onSelect} />
            {/if}

            {#if shows(sections, "destinations")}
                <DestinationForm
                    {csvPath}
                    {journals}
                    {journalId}
                    needsJournal={action === "saveAndImport"}
                    problems={validateCsvPath(csvPath)}
                    disabled={busy}
                    {onCsvPath}
                    {onJournal}
                />
            {/if}

            {#if shows(sections, "balance")}
                <BalanceField
                    {balance}
                    {balanceAccount}
                    statement={file.statement}
                    {accountNames}
                    {writeAssertion}
                    disabled={busy}
                    {onBalance}
                    onAccount={onBalanceAccount}
                    {onWriteAssertion}
                />
            {/if}

            {#if shows(sections, "actions")}
                <div class="flex flex-wrap items-center gap-3" data-testid="imports-actions">
                    <button type="button" class="btn btn-primary" disabled={blocker !== null || busy} onclick={onSubmit} data-testid="imports-submit">
                        {#if busy}<span class="loading loading-spinner loading-xs"></span>{/if}
                        {actionLabel(action)}
                    </button>
                    <!-- A disabled button with no explanation is how a form
                         dead-ends. Every blocker names a field just above it. -->
                    {#if blocker !== null}
                        <span class="text-warning text-sm" role="status">{blocker}</span>
                    {:else if action === "saveAndImport"}
                        <span class="text-base-content/60 text-sm">Nothing is written until you have seen what it proposes.</span>
                    {/if}
                </div>
            {/if}
        </div>
    {/snippet}
</AsyncSection>
