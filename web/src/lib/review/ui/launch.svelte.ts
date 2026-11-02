// Which review overlay is open, and where the journal should land when it
// closes (plan 32 D17, Phase 3 "Focus").
//
// The overlay is mounted by the Journal page and opened from the table's
// toolbar (buttons, `u`/`r`), which are different components — so the open
// state is a tiny module store, like `txnModal`. Closing hands the journal a
// one-shot `returnTo` request (the problems drawer's `focusRequest` shape):
// the table consumes it, scrolls to that transaction and puts its cursor
// there.
//
// The payee and tag vocabularies (D15) live here too: loaded once per open,
// and again when the journal's generation moves.
//
// An IMPORT opens a staged run the same way (plan 32 Phase 4): the New
// Transactions panel mounts the overlay while `staged` is set, handing it the
// import's transactions, the sink its saves go to, and — for a CSV import
// only — the rule builder's view of the rules file. Closing one calls its
// `onClose` instead of steering the journal's cursor.

import {LedgelineApi} from "#lib/api/native.js";
import {decodePayees, decodeTags} from "#lib/api/reviewDecode.js";
import type {RuleSource, StagedSink, StagedTxn} from "#lib/imports/reviewTypes.js";
import type {StagedKind} from "#lib/stores/review.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import type {PayeeUse, QueueKind, TagUse} from "../types";

/** What an import hands the overlay to review its transactions before they are written. */
export interface StagedLaunch {
    readonly kind: StagedKind;
    readonly items: readonly StagedTxn[];
    readonly sink: StagedSink;
    /** "Remember as a rule" (D25): a CSV import's rules file. Null for QuickBooks (D30), which has no rule to write. */
    readonly rules: RuleSource | null;
    /** The import's write button, which is what applies the run's edits (D26): "Write changes", or "Import" for QuickBooks. */
    readonly writeLabel?: string;
    /** The run closed; `saved` says whether any card was saved in it. */
    readonly onClose?: (saved: boolean) => void;
}

let kind = $state<QueueKind | null>(null);
let staged = $state.raw<StagedLaunch | null>(null);
let returnTo = $state<{txnIndex: number; nonce: number} | null>(null);
let nonce = 0;

let payees = $state.raw<readonly PayeeUse[]>([]);
let tags = $state.raw<readonly TagUse[]>([]);
let vocabGeneration: string | null = null;
let vocabToken = 0;

export const reviewLaunch = {
    /** The queue the overlay is showing, or null when it is closed. */
    get kind(): QueueKind | null {
        return kind;
    },
    open(next: QueueKind): void {
        staged = null;
        kind = next;
    },
    /** The staged run the overlay is showing, or null. */
    get staged(): StagedLaunch | null {
        return staged;
    },
    /** Open a review run over an import's transactions (D23, D30). */
    openStaged(request: StagedLaunch): void {
        kind = null;
        staged = request;
    },
    /**
     * Close. For a journal run `lastReviewed` (a tindex) is where the journal's
     * cursor should land; for a staged run it is the last staged card saved,
     * and the import is told whether there was one.
     */
    close(lastReviewed: number | null): void {
        const closing = staged;
        kind = null;
        staged = null;
        if (closing !== null) closing.onClose?.(lastReviewed !== null);
        else if (lastReviewed !== null) returnTo = {txnIndex: lastReviewed, nonce: ++nonce};
    },
    get returnTo(): {txnIndex: number; nonce: number} | null {
        return returnTo;
    },
    clearReturn(): void {
        returnTo = null;
    },

    /** Description completions (D15), most used first. */
    get payees(): readonly PayeeUse[] {
        return payees;
    },
    /** Tag completions (D15), most used first. */
    get tags(): readonly TagUse[] {
        return tags;
    },
    /** Load both vocabularies unless they are already for `generation`. Failures leave the old lists (completion is an aid). */
    async loadVocabulary(generation: string | null): Promise<void> {
        const serverUrl = settings.serverUrl;
        if (serverUrl === null || (generation !== null && generation === vocabGeneration)) return;
        const token = ++vocabToken;
        const api = new LedgelineApi(serverUrl);
        const [payeeList, tagList] = await Promise.allSettled([api.payees(), api.tags()]);
        if (token !== vocabToken) return;
        if (payeeList.status === "fulfilled") {
            try {
                payees = decodePayees(payeeList.value).payees;
            } catch {
                // Keep the old list.
            }
        }
        if (tagList.status === "fulfilled") {
            try {
                tags = decodeTags(tagList.value).tags;
            } catch {
                // Keep the old list.
            }
        }
        vocabGeneration = generation;
    },
};

/** Test seam. */
export function resetReviewLaunch(): void {
    kind = null;
    staged = null;
    returnTo = null;
    payees = [];
    tags = [];
    vocabGeneration = null;
    vocabToken += 1;
}
