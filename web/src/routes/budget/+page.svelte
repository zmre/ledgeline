<script lang="ts">
    // The Budget tab: how you are doing, and what you said you would do.
    //
    // It was a report tab. It is its own top-level tab now because the second
    // half — the editor — is a write surface, and a write surface does not belong
    // behind a strip labelled "Reports". The bars above and the goals below are
    // the same subject read two ways, so they share a page and a refresh: a save
    // reloads both (`budgetStore.afterWrite`), because a screen whose top half
    // disagrees with its bottom half about the budget is worse than one that is
    // briefly blank.
    //
    // The controls, the URL mirroring and the export follow the reports page
    // verbatim — same `searchMirror`, same FE-1 gate on the export (the workbook
    // is named from the CURRENT controls and built from the HELD report, so those
    // two may only be combined when they answer the same question).
    import {onMount} from "svelte";
    import {
        activeBudgetPreset,
        budgetParamsToSearch,
        budgetPresetRange,
        budgetSpan,
        BUDGET_PRESETS,
        defaultBudgetParams,
        searchToBudgetParams,
        type BudgetParams,
        type BudgetPreset,
    } from "$lib/budget/params";
    import {budgetStore, REFERENCE_PERIODS, type BudgetReportQuery} from "$lib/budget/budgetStore.svelte";
    import {alreadyBudgeted, goalChange} from "$lib/budget/target";
    import type {BudgetChange} from "$lib/api/native";
    import type {BudgetGoal, BudgetPeriod, BudgetRule, GoalDraft, GoalSubmission} from "$lib/budget/types";
    import BudgetEditor from "$lib/budget/ui/BudgetEditor.svelte";
    import GoalModal from "$lib/budget/ui/GoalModal.svelte";
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import ErrorToast from "$lib/components/ErrorToast.svelte";
    import DepthSlider from "$lib/insights/DepthSlider.svelte";
    import {declaredTypes} from "$lib/domain/accountTypes";
    import {decToInput} from "$lib/api/editMapping";
    import {exportBudgetXlsx} from "$lib/export/xlsx";
    import BudgetSummary from "$lib/reports/ui/BudgetSummary.svelte";
    import ExportButton from "$lib/reports/ui/ExportButton.svelte";
    import {reportStyles} from "$lib/reports/ui/styles";
    import {dataView} from "$lib/stores/loadState";
    import {journal} from "$lib/stores/journal.svelte";
    import {loadJournalWhenReady, onServerReady} from "$lib/stores/serverWatch.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {searchMirror} from "$lib/url/searchSync";

    let params = $state<BudgetParams>(defaultBudgetParams());
    let restored = $state(false);

    onMount(() => {
        if (window.location.search !== "") Object.assign(params, searchToBudgetParams(window.location.search, defaultBudgetParams()));
        restored = true;
        return () => mirror.stop();
    });

    // Mirror params → URL, debounced, replaceState (no history entries, no loops).
    // Reading `params` before the `restored` guard is deliberate: the effect has
    // to depend on them even on the run where it declines to write.
    const mirror = searchMirror();
    $effect(() => {
        const search = budgetParamsToSearch(params);
        if (!restored) return;
        mirror.write(search);
    });

    // Styles + account names come from the journal wire feed; the report and the
    // goals are native. Never a gate — `formatTotals` falls back per commodity.
    loadJournalWhenReady();
    const styles = $derived(reportStyles(journal.txns));
    const declared = $derived(declaredTypes(journal.accountDecls));
    const maxDepth = $derived(journal.accountNames.reduce((max, name) => Math.max(max, name.split(":").length), 1));

    // --- The bars ------------------------------------------------------------

    // budgetSpan, not params: the bars cover whole months, so the query (and the
    // journal link under each bar) has to cover the same span or the rows won't
    // add up to the bar.
    const span = $derived(budgetSpan(params.from, params.to));
    const reportQuery = $derived<BudgetReportQuery>({from: params.from, end: span.to, count: span.count, depth: params.depth});
    // The nonce is in the key because a reconnect usually leaves the URL
    // identical (the engine restarted on the same port), and keying on the URL
    // alone meant a page never retried after one (FE-5d).
    const loadKey = $derived({url: settings.serverUrl, nonce: settings.serverNonce, query: reportQuery});
    $effect(() => {
        const {url, query} = loadKey;
        if (url !== null) void budgetStore.report.load(url, query);
    });

    const activePreset = $derived(activeBudgetPreset(params.from, params.to));
    function applyPreset(preset: BudgetPreset): void {
        const range = budgetPresetRange(preset);
        params.from = range.from;
        params.to = range.to;
    }

    const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
    function setDate(key: "from" | "to", value: string): void {
        if (ISO_DATE.test(value)) params[key] = value;
    }

    const held = $derived(budgetStore.report);
    const reportView = $derived(dataView(held.status, held.value !== null));

    /**
     * The report the export button may act on: the one being shown AND answering
     * exactly the query the controls currently describe. `exportInfo` builds the
     * filename and the title from `params`, so anything less lets the user
     * download a workbook labelled with a span it does not hold (FE-1).
     */
    const exportable = $derived.by(() => {
        const q = held.query;
        const shown = held.value;
        if (shown === null || q === null) return null;
        return q.from === reportQuery.from && q.end === reportQuery.end && q.count === reportQuery.count && q.depth === reportQuery.depth ? shown : null;
    });

    // --- The goals -----------------------------------------------------------

    onServerReady((url) => void budgetStore.ensureListing(url, settings.serverNonce));
    const listing = $derived(budgetStore.listing);
    const listingView = $derived(dataView(listing.status, listing.value !== null));

    let draft = $state<GoalDraft | null>(null);
    /** The engine's own sentence for the last refused write, shown in the modal or as a toast. */
    let writeError = $state<string | null>(null);

    const reference = $derived(budgetStore.reference);
    const referenceView = $derived(dataView(reference.status, reference.value !== null));

    /** Refetch the history strip for the account and period the modal is showing. */
    function loadReference(account: string, period: BudgetPeriod): void {
        const url = settings.serverUrl;
        if (url === null) return;
        void budgetStore.loadReference(url, {account, interval: period, count: REFERENCE_PERIODS});
    }

    function openAdd(journalId: string, rule: BudgetRule | null): void {
        writeError = null;
        draft = {
            goal: null,
            rule,
            journalId,
            // A new goal joins the rule it was added from; a brand-new rule opens
            // on monthly, which is what almost every budget line is.
            period: rule !== null && isOffered(rule.period) ? (rule.period as BudgetPeriod) : "monthly",
            account: "",
            amount: "",
        };
    }

    function openEdit(journalId: string, rule: BudgetRule, goal: BudgetGoal): void {
        writeError = null;
        draft = {
            goal,
            rule,
            journalId,
            period: isOffered(rule.period) ? (rule.period as BudgetPeriod) : "monthly",
            account: goal.account,
            // The magnitude, exactly as the engine offers it — never the signed
            // amount, so an income goal opens showing what the user typed.
            amount: goal.entry === null ? "" : decToInput(goal.entry.value),
        };
    }

    function isOffered(period: string): boolean {
        return period === "daily" || period === "weekly" || period === "monthly" || period === "quarterly" || period === "yearly";
    }

    /** The revision to quote for a save into `journalId`, or null when it is not listed. */
    function revisionFor(journalId: string): string | null {
        return listing.value?.files.find((file) => file.journalId === journalId)?.revision ?? null;
    }

    async function apply(journalId: string, change: BudgetChange): Promise<boolean> {
        const revision = revisionFor(journalId);
        if (revision === null) {
            writeError = "That budget file is no longer listed. Reload and try again.";
            return false;
        }
        const outcome = await budgetStore.save(journalId, revision, change, reportQuery);
        if (outcome.ok) {
            writeError = null;
            return true;
        }
        writeError = outcome.failure.message;
        return false;
    }

    async function submitGoal(submission: GoalSubmission): Promise<void> {
        const current = draft;
        if (current === null) return;
        // Which of the three shapes this is, and which file it goes to, is
        // `goalChange`'s to decide — the same module the per-period "+ Add"
        // button asks, so that a goal added from the top of the page joins the
        // rule that button would have added it to instead of opening a second
        // one beside it.
        const {journalId, change} = goalChange(listing.value, current, submission);
        if (await apply(journalId, change)) draft = null;
    }

    /**
     * The goal a Remove click has armed, awaiting confirmation.
     *
     * An inline two-step, like the transaction popup's delete — not
     * `window.confirm`, which this app uses nowhere and which cannot be styled,
     * tested or dismissed with Escape.
     */
    let removing = $state<{journalId: string; goal: BudgetGoal} | null>(null);

    async function removeGoal(journalId: string, goal: BudgetGoal): Promise<void> {
        removing = null;
        await apply(journalId, {kind: "remove", index: goal.index});
    }

    async function createFile(): Promise<void> {
        const outcome = await budgetStore.createFile(reportQuery);
        writeError = outcome.ok ? null : outcome.failure.message;
    }

    function retryListing(): void {
        const url = settings.serverUrl;
        if (url !== null) void budgetStore.reloadListing(url);
    }
</script>

<svelte:head><title>Ledgeline — Budget</title></svelte:head>

<div class="flex flex-col gap-6">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-lg font-semibold">Budget</h1>
        {#if exportable !== null}
            {@const current = exportable}
            <ExportButton
                run={() =>
                    exportBudgetXlsx(
                        current,
                        {title: "Budget", params: `${params.from} to ${params.to}, depth ${params.depth}`},
                        `budget-${params.from}-to-${params.to}.xlsx`,
                        declared
                    )}
            />
        {/if}
    </div>

    <div class="flex flex-wrap items-end gap-x-4 gap-y-2 rounded-box bg-base-200 px-3 py-2">
        <div class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Period</span>
            <div class="join" role="group" aria-label="Budget period">
                {#each BUDGET_PRESETS as preset (preset.id)}
                    <button
                        type="button"
                        class="btn join-item btn-sm {activePreset === preset.id ? 'btn-active btn-primary' : ''}"
                        aria-pressed={activePreset === preset.id}
                        onclick={() => applyPreset(preset.id)}
                    >
                        {preset.label}
                    </button>
                {/each}
            </div>
        </div>
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">From</span>
            <input type="date" class="input w-40 input-sm" value={params.from} onchange={(e) => setDate("from", e.currentTarget.value)} aria-label="From" />
        </label>
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">To</span>
            <input type="date" class="input w-40 input-sm" value={params.to} onchange={(e) => setDate("to", e.currentTarget.value)} aria-label="To" />
        </label>
        <!-- Keyed on maxDepth for the reason the reports bar keys it: the slider
             can mount while accounts are still loading (max=1), and the browser
             clamps the input's value to that max without updating bound state. -->
        {#key maxDepth}
            <DepthSlider bind:depth={params.depth} max={maxDepth} />
        {/key}
    </div>

    <AsyncSection
        view={reportView}
        value={held.value}
        error={held.error}
        testid="budget-error"
        label="the budget"
        loadingLabel="Loading budget"
        onRetry={() => void budgetStore.report.load(settings.serverUrl ?? "", reportQuery)}
    >
        {#snippet children(current)}
            <BudgetSummary report={current} {styles} {declared} from={span.from} to={span.to} />
        {/snippet}
    </AsyncSection>

    <div class="divider my-0"></div>

    {#if budgetStore.available}
        <AsyncSection
            view={listingView}
            value={listing.value}
            error={listing.error}
            testid="budget-lines-error"
            label="your budget goals"
            loadingLabel="Loading budget goals"
            onRetry={retryListing}
        >
            {#snippet children(current)}
                <BudgetEditor
                    listing={current}
                    {styles}
                    busy={budgetStore.saving}
                    onAdd={openAdd}
                    onEdit={openEdit}
                    {removing}
                    onRemove={(journalId, goal) => (removing = {journalId, goal})}
                    onRemoveCancel={() => (removing = null)}
                    onRemoveConfirm={(journalId, goal) => void removeGoal(journalId, goal)}
                    onCreateFile={() => void createFile()}
                />
            {/snippet}
        </AsyncSection>
    {/if}

    {#if budgetStore.conflict}
        <div class="alert py-2 text-sm alert-warning" role="alert" data-testid="budget-conflict">
            <span class="grow">Your journal changed on disk, so nothing was written. Reload to pick up the new version.</span>
            <button type="button" class="btn btn-sm" onclick={retryListing}>Reload</button>
        </div>
    {/if}
</div>

{#if draft !== null}
    {@const current = draft}
    <!-- Keyed on which goal is being edited, so opening a second one gets fresh
         fields. The modal seeds its inputs from the draft ONCE (typing must not
         be undone by the parent's copy), and "once" only means what it should if
         the component is remounted per goal. -->
    {#key `${current.journalId}#${current.goal?.index ?? "new"}#${current.rule?.block ?? "norule"}`}
        <GoalModal
            draft={current}
            accountNames={journal.accountNames}
            reference={reference.value}
            {referenceView}
            {styles}
            saving={budgetStore.saving}
            error={writeError}
            isBudgeted={(account, period) => alreadyBudgeted(listing.value, current, period, account)}
            onAccountChange={loadReference}
            onSubmit={(submission) => void submitGoal(submission)}
            onCancel={() => {
                draft = null;
                writeError = null;
            }}
        />
    {/key}
{/if}

<!-- A write that failed with no modal open (a removal, or the create-file
     button) still has to say so. -->
{#if draft === null}
    <ErrorToast message={writeError} onRetry={() => (writeError = null)} />
{/if}
