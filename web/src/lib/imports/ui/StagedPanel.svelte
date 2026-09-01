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
    import CreateRulesPanel from "./CreateRulesPanel.svelte";
    import DestinationForm from "./DestinationForm.svelte";
    import PreviewTable from "./PreviewTable.svelte";
    import type {RulesDraft} from "../types";
    import type {FormItem, RulesForm} from "../model";

    let {
        sections,
        view,
        staged,
        error,
        journals,
        accountNames,
        selectedRulesId,
        creating,
        createDraft,
        createForm,
        createId,
        createDrafting,
        createSaving,
        createError,
        createdId,
        onCreateOpen,
        onCreateClose,
        onCreateId,
        onCreateItems,
        onCreateSave,
        onCreateRetry,
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
        /** The Create-a-rules-file flow. All of it null/false until the button is pressed. */
        creating: boolean;
        createDraft: RulesDraft | null;
        createForm: RulesForm | null;
        createId: string;
        createDrafting: boolean;
        createSaving: boolean;
        createError: string | null;
        createdId: string | null;
        onCreateOpen: () => void;
        onCreateClose: () => void;
        onCreateId: (value: string) => void;
        onCreateItems: (items: FormItem[]) => void;
        onCreateSave: () => void;
        onCreateRetry: () => void;
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
                <CandidateList
                    candidates={file.candidates}
                    selectedId={selectedRulesId}
                    disabled={busy}
                    {creating}
                    {createdId}
                    {onSelect}
                    onCreate={onCreateOpen}
                />
            {/if}

            <!-- Outside `candidates`' own section: the panel is reached from
                 that list but is not part of it, and it stays open across the
                 re-stage its own save triggers. -->
            {#if creating}
                <CreateRulesPanel
                    draft={createDraft}
                    form={createForm}
                    id={createId}
                    drafting={createDrafting}
                    saving={createSaving}
                    error={createError}
                    {accountNames}
                    onId={onCreateId}
                    onItems={onCreateItems}
                    onSave={onCreateSave}
                    onRetry={onCreateRetry}
                    onCancel={onCreateClose}
                />
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
                        {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                        {actionLabel(action)}
                    </button>
                    <!-- A disabled button with no explanation is how a form
                         dead-ends. Every blocker names a field just above it. -->
                    {#if blocker !== null}
                        <span class="text-sm text-warning" role="status">{blocker}</span>
                    {:else if action === "saveAndImport"}
                        <span class="text-sm text-base-content/60">Nothing is written until you have seen what it proposes.</span>
                    {/if}
                </div>
            {/if}
        </div>
    {/snippet}
</AsyncSection>
