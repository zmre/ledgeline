// Settings › Automatic postings' data: one read of `GET /api/auto-rules`,
// keyed on the journal's generation as well as the server, so a journal edit
// or an `--auto` toggle (both republish the snapshot) re-reads the list and
// its generated-posting counts.

import {LedgelineApi} from "#lib/api/native.js";
import {createResource} from "#lib/stores/resource.svelte.js";
import {decodeAutoRules} from "./decode";
import type {AutoRulesListing} from "./types";

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
