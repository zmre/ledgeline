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

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi} from "$lib/api/native";
import {decodeQbCommitResult, decodeQbPreview, decodeSortResult} from "$lib/api/nativeDecode";
import {aliasListing, aliasStore} from "./aliasStore.svelte";
import {isInFlight} from "./importModel";
import type {QbCommitResult, QbPreview} from "./importTypes";
import {defaultAliasTargetFile, mappingEdits} from "./qbJournalModel";
import {createResource} from "$lib/stores/resource.svelte";
import {settings} from "$lib/stores/settings.svelte";

export const preview = createResource<string, QbPreview>(async (serverUrl, stageId) =>
    decodeQbPreview(await new LedgelineApi(serverUrl).qbJournalPreview(stageId))
);

/** Keyed on `stageId`, exactly like the request body — there is nothing else the request could vary by. */
export const commit = createResource<string, QbCommitResult>(async (serverUrl, stageId) =>
    decodeQbCommitResult(await new LedgelineApi(serverUrl).qbJournalCommit({stageId}))
);

/** Which stage `preview` was last asked to load — so `ensurePreview` fetches once per stage, not once per render. */
let previewKey: string | null = null;

/** One typed replacement per unmapped account, keyed by the QuickBooks account name (the alias PATTERN). */
let drafts = $state<Record<string, string>>({});
let mappingSaving = $state(false);
let mappingError = $state<string | null>(null);

/** `Import` was pressed, so a commit exists or is running. */
let commitRequested = $state(false);

/** The re-sort offer is per touched FILE (a multi-year import can touch more than one), unlike the CSV path's single target. */
let sortingJournalId = $state<string | null>(null);
let sortMoved = $state<Record<string, number>>({});
let sortErrors = $state<Record<string, string>>({});

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
    sortingJournalId = null;
    sortMoved = {};
    sortErrors = {};
}

export const qbJournalStore = {
    // --- preview ------------------------------------------------------------
    get preview(): QbPreview | null {
        return preview.value;
    },
    get previewView(): import("$lib/stores/loadState").DataView {
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
    get commitView(): import("$lib/stores/loadState").DataView {
        return commit.view;
    },
    get commitError(): Error | null {
        return commit.error;
    },
    /** The write is running right now — the one thing the Import button must not be pressed twice during. */
    get committing(): boolean {
        return isInFlight(commit.status);
    },

    async commitStage(serverUrl: string, stageId: string): Promise<void> {
        commitRequested = true;
        sortMoved = {};
        sortErrors = {};
        await commit.load(serverUrl, stageId);
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
            // A re-sort git did not take leaves the journal rewritten and
            // uncommitted — the same caveat `importStore.resort` reports.
            if (sorted.git !== null && !sorted.git.committed) {
                sortErrors = {
                    ...sortErrors,
                    [journalId]: `The journal was re-sorted, but git did not commit the change. Commit ${journalId} yourself — until you do, this import can no longer be undone with \`git revert\`.`,
                };
            }
        } catch (error) {
            sortErrors = {...sortErrors, [journalId]: failureMessage(error)};
        } finally {
            sortingJournalId = null;
        }
    },
};
