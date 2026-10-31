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

import {LedgelineApi} from "#lib/api/native.js";
import {decodePayees, decodeTags} from "#lib/api/reviewDecode.js";
import {settings} from "#lib/stores/settings.svelte.js";
import type {PayeeUse, QueueKind, TagUse} from "../types";

let kind = $state<QueueKind | null>(null);
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
        kind = next;
    },
    /** Close; `lastReviewed` (a tindex) is where the journal's cursor should land. */
    close(lastReviewed: number | null): void {
        kind = null;
        if (lastReviewed !== null) returnTo = {txnIndex: lastReviewed, nonce: ++nonce};
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
    returnTo = null;
    payees = [];
    tags = [];
    vocabGeneration = null;
    vocabToken += 1;
}
