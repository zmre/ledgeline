// Settings › hledger config's data layer — and the Reports page's source for
// the config's `--depth` defaults. The same two shapes `accountsStore` uses:
// `createResource` for the read, a dispatcher for the write.
//
// A save refreshes the journal feed as well as this resource: the engine
// republishes its snapshot on a config write (reports read the file's depth,
// and `--auto` changes which postings exist), so the generation moves and
// every open report must redraw.

import {classify, type EditFailure} from "#lib/api/editFailure.js";
import {LedgelineApi, NativeApiUnavailableError} from "#lib/api/native.js";
import {decodeHledgerConf} from "#lib/api/nativeDecode.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {createResource} from "#lib/stores/resource.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import type {HledgerConf, SaveHledgerConfBody} from "./hledgerConfTypes";

/** Keyed by whatever the caller wants a fresh read for (a reconnect, a journal generation). */
export const hledgerConf = createResource<string, HledgerConf>(async (serverUrl) => decodeHledgerConf(await new LedgelineApi(serverUrl).hledgerConf()));

let available = $state(true);
let saving = $state(false);
/** Set by a 409: the file changed on disk under the open form. */
let conflict = $state(false);
let loadedKey: string | null = null;

export type ConfSaveOutcome = {ok: true; conf: HledgerConf} | {ok: false; failure: EditFailure};

export const hledgerConfStore = {
    /** False once the engine has answered 404 for `/api/hledger-conf` — an older engine. */
    get available(): boolean {
        return available;
    },
    get saving(): boolean {
        return saving;
    },
    get conflict(): boolean {
        return conflict;
    },

    /** Load once per `key` (e.g. `nonce|generation`), never twice for the same one. */
    async ensure(serverUrl: string, key: string): Promise<void> {
        const full = `${key}|${serverUrl}`;
        if (full === loadedKey) return;
        loadedKey = full;
        await this.reload(serverUrl);
    },

    /** Re-read unconditionally (after a save, or a Retry). */
    async reload(serverUrl: string): Promise<void> {
        conflict = false;
        await hledgerConf.load(serverUrl, serverUrl);
        available = !(hledgerConf.error instanceof NativeApiUnavailableError);
    },

    /** Write the changes; on success the engine answers with the file as written, which the caller re-seeds from. */
    async save(body: SaveHledgerConfBody): Promise<ConfSaveOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        saving = true;
        try {
            const conf = decodeHledgerConf(await new LedgelineApi(url).saveHledgerConf(body));
            conflict = false;
            void this.reload(url);
            void journal.refresh();
            return {ok: true, conf};
        } catch (error) {
            const failure = classify(error);
            if (failure.kind === "conflict") conflict = true;
            return {ok: false, failure};
        } finally {
            saving = false;
        }
    },
};
