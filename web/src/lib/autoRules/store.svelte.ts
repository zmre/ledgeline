// Settings › Automatic postings' data: one read of `GET /api/auto-rules`,
// keyed on the journal's generation as well as the server, so a journal edit
// or an `--auto` toggle (both republish the snapshot) re-reads the list and
// its generated-posting counts. And the editor's one write, `saveAutoRule`.

import {classify, type EditFailure} from "#lib/api/editFailure.js";
import {LedgelineApi} from "#lib/api/native.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {createResource} from "#lib/stores/resource.svelte.js";
import {decodeAutoRules} from "./decode";
import type {AutoRulesListing, SaveAutoRuleBody} from "./types";

export const autoRules = createResource<string, AutoRulesListing>(async (serverUrl) => decodeAutoRules(await new LedgelineApi(serverUrl).listAutoRules()));

let loadedKey: string | null = null;

/** Load the listing for (server, generation), at most once per pair. */
export function ensureAutoRules(serverUrl: string, generation: string | null): void {
    const key = `${generation ?? ""}|${serverUrl}`;
    if (key === loadedKey) return;
    loadedKey = key;
    void autoRules.load(serverUrl, key);
}

/** Re-read unconditionally (Retry). */
export function reloadAutoRules(serverUrl: string, generation: string | null): void {
    loadedKey = null;
    ensureAutoRules(serverUrl, generation);
}

export type SaveOutcome = {ok: true} | {ok: false; failure: EditFailure};

/**
 * Make one change to the journal's rules. On success the journal is
 * refreshed — every view now has different generated postings — and its new
 * generation re-reads the listing. A 409 re-reads it too, so the editor shows
 * the file as it now is.
 */
export async function saveAutoRule(serverUrl: string, body: SaveAutoRuleBody): Promise<SaveOutcome> {
    try {
        // Decoded only to prove the shape; the generation-keyed reload is what
        // the panel renders.
        decodeAutoRules(await new LedgelineApi(serverUrl).saveAutoRule(body));
        void journal.refresh({force: true});
        reloadAutoRules(serverUrl, journal.generation);
        return {ok: true};
    } catch (error) {
        const failure = classify(error);
        if (failure.kind === "conflict") reloadAutoRules(serverUrl, journal.generation);
        return {ok: false, failure};
    }
}
