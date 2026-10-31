// The QuickBooks Online Journal import panel's data layer (WP-17 Phase C).
//
// Shaped like `importStore.svelte.ts`: `createResource` for what the screen
// WAITS on (the preview, the commit), a plain dispatcher for what it PRESSES
// (a mapping save, a re-sort). Every decision lives in `qbJournalModel.ts`;
// this file wires, sequences and holds.
//
// # Reuses the EXISTING alias-editing store, on purpose
//
// The plan is explicit: unmapped accounts are resolved through the alias
// wire the Account Aliases tab already uses (`PUT /api/aliases/{*journalId}`,
// via `aliasStore.save`), not a second way to write an alias line. So this
// file imports `aliasStore`/`aliasListing` from `aliasStore.svelte.ts` rather
// than duplicating a listing fetch or a save call — a save here and a save
// made from the Account Aliases tab are the exact same request, sharing the
// exact same conflict/revision handling.

import {classify, type EditFailure} from "#lib/api/editFailure.js";
import {LedgelineApi, type QbJournalCommitBody} from "#lib/api/native.js";
import {decodeQbCommitResult, decodeQbPreview, decodeSortResult} from "#lib/api/nativeDecode.js";
import {aliasListing, aliasStore} from "./aliasStore.svelte";
import {isInFlight} from "./importModel";
import type {QbCommitResult, QbPreview} from "./importTypes";
import type {ReplayReport, StagedOverride, StagedRef, StagedSink, StagedTxn} from "./reviewTypes";
import {createStagedEdits} from "./stagedEdits.svelte";
import {defaultAliasTargetFile, mappingEdits} from "./qbJournalModel";
import {createResource} from "#lib/stores/resource.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";

/**
 * How long a QuickBooks Journal COMMIT may run before the client gives up on
 * it, instead of `client.ts`'s shared `REQUEST_TIMEOUT_MS` (30s) every other
 * route uses.
 *
 * That 30s figure exists to catch a connection that has gone truly dead — see
 * its own docs — not as a performance budget, and a bulk write of several
 * thousand transactions in one request is a different shape of operation
 * than anything else in this app makes: a real 10,500-transaction import (the
 * one that motivated WP-17's bulk-insert rewrite) still takes single-digit
 * seconds end to end against a release build, but a slower machine, a
 * several-times-larger export, or a debug build (measurably an order of
 * magnitude slower for this CPU-bound path — confirmed directly against this
 * commit route) can all plausibly need more than 30s for a request that is
 * genuinely still working, not stuck. Ten minutes is generous headroom for
 * all of those without being unbounded — this route still fails closed
 * against an actually-dead connection, just on a timescale that matches what
 * it is actually being asked to do.
 */
export const QB_JOURNAL_COMMIT_TIMEOUT_MS = 10 * 60 * 1000;

export const preview = createResource<string, QbPreview>(async (serverUrl, stageId) =>
    decodeQbPreview(await new LedgelineApi(serverUrl).qbJournalPreview(stageId))
);

/** Keyed on the request body itself: the stage, and whether changed rows are to be replaced. */
export const commit = createResource<QbJournalCommitBody, QbCommitResult>(async (serverUrl, body) =>
    decodeQbCommitResult(await new LedgelineApi(serverUrl, {timeoutMs: QB_JOURNAL_COMMIT_TIMEOUT_MS}).qbJournalCommit(body))
);

/** Which stage `preview` was last asked to load — so `ensurePreview` fetches once per stage, not once per render. */
let previewKey: string | null = null;

/** One typed replacement per unmapped account, keyed by the QuickBooks account name (the alias PATTERN). */
let drafts = $state<Record<string, string>>({});
let mappingSaving = $state(false);
let mappingError = $state<string | null>(null);

/** `Import` was pressed, so a commit exists or is running. */
let commitRequested = $state(false);

/**
 * Replace rows the journal already holds differently with the export's version.
 * Off by default, and back off for every new stage: the usual reason a row
 * differs is that its accounts were categorized by hand after the import.
 */
let updateChanged = $state(false);

/** The re-sort offer is per touched FILE (a multi-year import can touch more than one), unlike the CSV path's single target. */
let sortingJournalId = $state<string | null>(null);
let sortMoved = $state<Record<string, number>>({});
let sortErrors = $state<Record<string, string>>({});

/**
 * The review overlay's edits (plan 32 D30): overrides to the built
 * transactions, keyed by Trans #, and queued edits to similar journal
 * transactions. Both ride on the commit; the preview takes none, so it always
 * shows what the export alone would write.
 */
const edits = createStagedEdits();
let lastReplay = $state.raw<ReplayReport | null>(null);

function failureMessage(error: unknown): string {
    const failure: EditFailure = classify(error);
    return failure.message;
}

/** Forget everything downstream of the previous stage — a new stage invalidates all of it. */
function resetForNewStage(): void {
    drafts = {};
    mappingSaving = false;
    mappingError = null;
    commitRequested = false;
    updateChanged = false;
    sortingJournalId = null;
    sortMoved = {};
    sortErrors = {};
    edits.clear();
    lastReplay = null;
}

export const qbJournalStore = {
    // --- preview ------------------------------------------------------------
    get preview(): QbPreview | null {
        return preview.value;
    },
    get previewView(): import("#lib/stores/loadState.js").DataView {
        return preview.view;
    },
    get previewError(): Error | null {
        return preview.error;
    },

    /** Load the preview for `stageId`, once per stage — the panel's own mount hook. */
    async ensurePreview(serverUrl: string, stageId: string): Promise<void> {
        if (previewKey === stageId) return;
        previewKey = stageId;
        resetForNewStage();
        await preview.load(serverUrl, stageId);
    },

    /** Re-fetch the SAME stage's preview unconditionally — after a mapping save, or a Retry. */
    async refreshPreview(serverUrl: string, stageId: string): Promise<void> {
        await preview.load(serverUrl, stageId);
        // A mapping save rebuilds every transaction it touched, so the held
        // overrides are replayed against the new build (D25's rules, by Trans #).
        const fresh = preview.value;
        if (fresh !== null && preview.query === stageId && edits.overrideCount > 0) lastReplay = edits.replay(fresh.txns);
    },

    // --- review edits (plan 32 D30) -------------------------------------------
    /** The built transactions of the current preview, as review cards. Empty while any account is unmapped. */
    get stagedTxns(): readonly StagedTxn[] {
        return preview.value?.txns ?? [];
    },
    get overrides(): readonly StagedOverride[] {
        return edits.overrides;
    },
    get overrideCount(): number {
        return edits.overrideCount;
    },
    get journalEditCount(): number {
        return edits.journalEditCount;
    },
    get flaggedOverrides(): readonly StagedOverride[] {
        return edits.flagged;
    },
    get lastReplay(): ReplayReport | null {
        return lastReplay;
    },
    /** The review store's staged sink: saves become overrides and queued journal edits, not writes. */
    reviewSink: {stage: (save) => edits.stage(save)} satisfies StagedSink as StagedSink,
    /** Let go of held overrides (the panel's "Discard" on flagged edits). The preview takes none, so nothing re-runs. */
    discardOverrides(refs: readonly StagedRef[]): void {
        for (const ref of refs) edits.discard(ref);
    },

    // --- resolving unmapped accounts -----------------------------------------
    draftFor(account: string): string {
        return drafts[account] ?? "";
    },
    setDraft(account: string, value: string): void {
        drafts = {...drafts, [account]: value};
        mappingError = null;
    },
    get mappingSaving(): boolean {
        return mappingSaving;
    },
    get mappingError(): string | null {
        return mappingError;
    },

    /**
     * Submit every valid, non-blank typed mapping as one batch of `append`
     * edits through the existing alias wire, then re-fetch the preview so the
     * accounts that resolved drop off `unmappedAccounts` — the same proof
     * Phase B's own test uses (`preview_reports_id_matches_once_every_
     * account_is_mapped`).
     *
     * The target file is the first WRITABLE one `GET /api/aliases` lists
     * (`defaultAliasTargetFile`) — the same file the Account Aliases tab
     * itself defaults its selection to.
     */
    async saveMappings(serverUrl: string, stageId: string, unmapped: readonly string[]): Promise<boolean> {
        mappingError = null;
        const edits = mappingEdits(unmapped, drafts);
        if (edits.length === 0) {
            mappingError = "Type an account for at least one row before mapping.";
            return false;
        }
        await aliasStore.ensureListing(serverUrl, settings.serverNonce);
        const listing = aliasListing.value;
        const target = listing === null ? null : defaultAliasTargetFile(listing.files);
        if (target === null) {
            mappingError = "There is no journal file here Ledgeline can add an alias to.";
            return false;
        }
        mappingSaving = true;
        const result = await aliasStore.save(target.journalId, {revision: target.revision, edits});
        mappingSaving = false;
        if (!result.ok) {
            mappingError = result.failure.message;
            return false;
        }
        // The rows that resolved are gone from the next preview; anything
        // still unmapped (blank, or refused by `mappingProblems`) is offered
        // again with an empty field rather than a stale, possibly-invalid one.
        drafts = {};
        await this.refreshPreview(serverUrl, stageId);
        return true;
    },

    // --- commit ---------------------------------------------------------------
    get commitRequested(): boolean {
        return commitRequested;
    },
    get commitResult(): QbCommitResult | null {
        return commit.value;
    },
    get commitView(): import("#lib/stores/loadState.js").DataView {
        return commit.view;
    },
    get commitError(): Error | null {
        return commit.error;
    },
    /** The write is running right now — the one thing the Import button must not be pressed twice during. */
    get committing(): boolean {
        return isInFlight(commit.status);
    },

    get updateChanged(): boolean {
        return updateChanged;
    },
    setUpdateChanged(value: boolean): void {
        updateChanged = value;
    },

    async commitStage(serverUrl: string, stageId: string): Promise<void> {
        commitRequested = true;
        sortMoved = {};
        sortErrors = {};
        const overrides = edits.qbOverrides();
        const journalEdits = edits.journalEdits;
        const body: QbJournalCommitBody = {
            stageId,
            updateChanged,
            // Only when there are some, so a commit without review is the body it always was.
            ...(overrides.length === 0 ? {} : {overrides}),
            ...(journalEdits.length === 0 ? {} : {journalEdits: [...journalEdits]}),
        };
        await commit.load(serverUrl, body);
        if (commit.error === null && commit.query === body) {
            edits.clear();
        } else if (commit.error !== null && classify(commit.error).kind === "conflict" && overrides.length > 0) {
            // An override went stale: rebuild the preview and replay against it.
            await this.refreshPreview(serverUrl, stageId);
        }
    },

    // --- the post-commit re-sort, per touched file ------------------------------
    get sortingJournalId(): string | null {
        return sortingJournalId;
    },
    sortMovedFor(journalId: string): number | null {
        return sortMoved[journalId] ?? null;
    },
    sortErrorFor(journalId: string): string | null {
        return sortErrors[journalId] ?? null;
    },

    /** Apply the confirmed re-sort for one out-of-order file — the same `POST /api/import/sort` route the CSV commit flow already offers. */
    async resortFile(serverUrl: string, journalId: string): Promise<void> {
        sortingJournalId = journalId;
        try {
            const sorted = decodeSortResult(await new LedgelineApi(serverUrl).importSort(journalId));
            sortMoved = {...sortMoved, [journalId]: sorted.moved};
            if (journalId in sortErrors) {
                const rest = {...sortErrors};
                delete rest[journalId];
                sortErrors = rest;
            }
        } catch (error) {
            sortErrors = {...sortErrors, [journalId]: failureMessage(error)};
        } finally {
            sortingJournalId = null;
        }
    },
};
