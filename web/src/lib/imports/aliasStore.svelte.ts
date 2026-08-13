// The account-alias screen's data layer: the listing, and the save dispatcher.
//
// The same two shapes `rulesStore.svelte.ts` uses, and borrowed rather than
// invented for the same reason — `createResource` for the read (so FE-1's
// "payload and the question it answers are ONE value" and the stale-response
// token come for free) and a `run()`-style dispatcher for the write.
//
// # Why a save DOES refetch the journal, unlike a rules save
//
// A rules file describes how a future import will read a CSV; it invalidates no
// transaction, which is why saving one deliberately skips `journal.refresh()`.
// An alias is different in one respect that matters: it lives IN the journal, so
// writing one changes a file the engine has parsed and the watcher is watching.
// The engine re-opens its own editor after the write; this reloads the listing
// so the screen's revisions and scope verdicts describe the file that now
// exists.

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi, NativeApiUnavailableError, type SaveAliasesBody} from "$lib/api/native";
import {decodeAliasFileResponse, decodeAliasListing} from "$lib/api/nativeDecode";
import {createResource} from "$lib/stores/resource.svelte";
import {settings} from "$lib/stores/settings.svelte";
import type {AliasFile, AliasListing} from "./importTypes";

export const aliasListing = createResource<string, AliasListing>(async (serverUrl) => decodeAliasListing(await new LedgelineApi(serverUrl).listAliases()));

let available = $state(true);
let saving = $state(false);
/** Set by a 409: the file moved under us, so the open form is a stale base for any save. */
let conflict = $state(false);
/** The last load key, so a page visit after a prefetch does not re-read the tree. */
let listingKey: string | null = null;

/** A save either lands (handing back the file the engine wrote) or fails with a classified reason. */
export type AliasSaveOutcome = {ok: true; file: AliasFile} | {ok: false; failure: EditFailure};

export const aliasStore = {
    /** False once the engine has answered 404 for `/api/aliases` — an older engine. */
    get available(): boolean {
        return available;
    },
    get saving(): boolean {
        return saving;
    },
    /** The open file changed on disk under an edit; the page offers reload-and-discard until cleared. */
    get conflict(): boolean {
        return conflict;
    },
    clearConflict(): void {
        conflict = false;
    },

    /** Load the listing once per (server, reconnect), and never twice for the same one. */
    async ensureListing(serverUrl: string, nonce: number): Promise<void> {
        const key = `${nonce}|${serverUrl}`;
        if (key === listingKey) return;
        listingKey = key;
        await this.reload(serverUrl);
    },

    /** Re-read the listing unconditionally (after a save, or a Retry). */
    async reload(serverUrl: string): Promise<void> {
        conflict = false;
        await aliasListing.load(serverUrl, serverUrl);
        available = !(aliasListing.error instanceof NativeApiUnavailableError);
    },

    /**
     * Save one file's alias lines.
     *
     * On success the engine answers with the file it actually wrote, at a fresh
     * revision — so the caller re-seeds its form from THAT rather than from what
     * it sent. An alias index is a parse's ordinal and is explicitly not stable
     * across saves (a delete renumbers everything below it), so keeping the old
     * form would make the next save address a different line.
     */
    async save(journalId: string, body: SaveAliasesBody): Promise<AliasSaveOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        saving = true;
        try {
            const file = decodeAliasFileResponse(await new LedgelineApi(url).saveAliases(journalId, body));
            conflict = false;
            // The whole listing carries every file's revision and each alias's
            // scope verdict, and a write can change both.
            void this.reload(url);
            return {ok: true, file};
        } catch (error) {
            const failure = classify(error);
            if (failure.kind === "conflict") conflict = true;
            // `available` is deliberately NOT cleared here: a 501 on this route
            // means "no journal is bound to an editor", which the listing already
            // reports as `editable: false`. Conflating the two would make one
            // read-only server remove the whole screen.
            return {ok: false, failure};
        } finally {
            saving = false;
        }
    },
};
