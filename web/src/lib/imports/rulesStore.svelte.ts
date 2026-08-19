// The imports screen's data layer: the file index, one open document plus its
// CSV preview, and the save dispatcher.
//
// Two shapes, both borrowed rather than invented:
//
//   - `createResource` for the two reads, so the stale-response token and the
//     "payload and the question it answers are ONE value" invariant (FE-1) come
//     for free instead of being re-derived here.
//   - a `run()` dispatcher modelled on `stores/editing.svelte.ts` for the write:
//     one place that calls the client, classifies the failure, and decides what
//     a conflict does to the rest of the store.
//
// # The index doubles as the capability probe
//
// An engine without `/api/rules` 404s, which `getJson` turns into
// `NativeApiUnavailableError` — that is how the nav item knows to hide itself.
// `available` starts TRUE and only goes false on that specific error, so the
// item does not blink out of existence during every ordinary load; the same
// reasoning as `editing.probe`'s refusal to treat an unreachable probe as a "no"
// (FE-5g).
//
// # Why a save does not go through `journal.refresh()`
//
// A rules file describes how a FUTURE import will read a CSV. It invalidates no
// transaction, which is exactly why the engine keeps rules files out of
// `source_files`, the snapshot and the watcher. Refetching the journal after
// saving one would buy a full reparse and republish for nothing.

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi, NativeApiUnavailableError, type SaveRulesBody} from "$lib/api/native";
import {decodeRulesDoc, decodeRulesIndex, decodeRulesPreview} from "$lib/api/nativeDecode";
import {createResource} from "$lib/stores/resource.svelte";
import {settings} from "$lib/stores/settings.svelte";
import type {RulesDocument, RulesIndex, RulesPreview} from "./types";

/** An open document plus the preview of the data file it describes. */
export interface OpenRules {
    readonly doc: RulesDocument;
    /**
     * The preview, or null when the REQUEST failed.
     *
     * Distinct from `preview.available === false`, which is the engine saying
     * "nothing was read, and here is the typed reason". A failed request is our
     * problem; an unavailable preview is information the mapping panel shows.
     */
    readonly preview: RulesPreview | null;
}

export const rulesIndex = createResource<string, RulesIndex>(async (serverUrl) => decodeRulesIndex(await new LedgelineApi(serverUrl).listRules()));

/** The preview, or null when the request itself failed — never a reason to lose the document. */
async function loadPreview(api: LedgelineApi, id: string): Promise<RulesPreview | null> {
    try {
        return decodeRulesPreview(await api.previewRules(id));
    } catch {
        return null;
    }
}

export const openRules = createResource<{url: string; id: string}, OpenRules>(async (serverUrl, query) => {
    const api = new LedgelineApi(serverUrl);
    // Sequential, not `Promise.all`: the preview is decoration, and a document
    // that loaded fine must not be thrown away because its CSV could not be read.
    const doc = decodeRulesDoc(await api.getRules(query.id));
    return {doc, preview: await loadPreview(api, query.id)};
});

let available = $state(true);
let saving = $state(false);
/** Set by a 409: the file changed under us, so the open document is a stale base for any save. */
let conflict = $state(false);
/** The last index/document load key, so a page visit after a prefetch does not walk the tree twice. */
let indexKey: string | null = null;

/** A save either lands (and hands back the document the engine wrote) or fails with a classified reason. */
export type SaveOutcome = {ok: true; doc: RulesDocument} | {ok: false; failure: EditFailure};

export const rulesStore = {
    /** False once the engine has answered 404 for `/api/rules` — an older engine, so hide the nav item. */
    get available(): boolean {
        return available;
    },
    /** A save is in flight. */
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

    /**
     * Load the file index once per (server, reconnect), and never twice for the
     * same one.
     *
     * The layout calls this to decide whether the nav item exists, and the page
     * calls it to render the list. Deduping is what stops that from being two
     * directory walks — the engine re-scans on every request by design (a cached
     * set is a set that no longer describes the disk), so the walk is real work.
     */
    async ensureIndex(serverUrl: string, nonce: number): Promise<void> {
        const key = `${nonce}|${serverUrl}`;
        if (key === indexKey) return;
        indexKey = key;
        await this.reloadIndex(serverUrl);
    },

    /** Re-read the file index unconditionally (after a save, or a Retry). */
    async reloadIndex(serverUrl: string): Promise<void> {
        await rulesIndex.load(serverUrl, serverUrl);
        available = !(rulesIndex.error instanceof NativeApiUnavailableError);
    },

    /** Open one rules file (document + preview), discarding any conflict flag from the last one. */
    async open(serverUrl: string, id: string): Promise<void> {
        conflict = false;
        await openRules.load(serverUrl, {url: serverUrl, id});
    },

    /**
     * Re-read JUST the CSV preview, after a save changed what it would say.
     *
     * A save can move `skip` and `separator`, and the engine feeds both into the
     * preview — `skip` picks which record is the header row, `separator` picks
     * where the columns are. So the preview loaded with the document can end up
     * labelling the wrong columns with the wrong header the moment a save lands.
     * Unlike the parse warnings, the `PUT` response cannot fix this: `wire_doc`
     * describes the rules document and says nothing about the DATA file it
     * names, so a second request is the only way to learn the new answer.
     *
     * Handed BACK rather than written into `openRules`, because that resource
     * holds `{doc, preview}` as one value answering one question (FE-1) and has
     * no partial update. Reloading it to refresh a decoration would refetch the
     * document too and flip the whole editor to its loading state — and would
     * re-open the file underneath an edit. The caller owns where this lands,
     * exactly as it already owns `baseDoc`.
     *
     * Null means the REQUEST failed, the same distinction `OpenRules.preview`
     * draws, and never an excuse to disturb the open document.
     */
    async reloadPreview(serverUrl: string, id: string): Promise<RulesPreview | null> {
        return loadPreview(new LedgelineApi(serverUrl), id);
    },

    /**
     * Save a whole document.
     *
     * On success the engine answers with the document it actually wrote — a
     * fresh revision and a fresh set of item ids — so the caller re-seeds its
     * form from THAT rather than from what it sent. Item ids are a parse's
     * indices and are explicitly not stable across saves, so keeping the old
     * ones would make the next save address items that no longer exist.
     */
    async save(id: string, body: SaveRulesBody): Promise<SaveOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        saving = true;
        try {
            const doc = decodeRulesDoc(await new LedgelineApi(url).saveRules(id, body));
            conflict = false;
            // The listing carries each file's revision and summary counts, both
            // of which this write just changed.
            void this.reloadIndex(url);
            return {ok: true, doc};
        } catch (error) {
            const failure = classify(error);
            if (failure.kind === "conflict") conflict = true;
            // `available` is deliberately NOT cleared here. A 501 on this route
            // means "this server has no journal bound to an editor", which the
            // index already reports as `editable: false` — it is not the "no
            // /api/rules at all" fact the nav item hides on, and conflating them
            // would make one read-only server remove the whole screen.
            return {ok: false, failure};
        } finally {
            saving = false;
        }
    },
};
