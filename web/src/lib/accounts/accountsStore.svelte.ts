// The Account List Editor's data layer: the listing, the save dispatcher, and
// the "create accounts.journal" action.
//
// Same two shapes `aliasStore.svelte.ts` uses, for the same reason:
// `createResource` for the read, a `run()`-style dispatcher for the write. A
// save DOES refetch the listing, exactly as an alias save does — an account
// declaration lives IN the journal, so writing one changes a file the engine
// has parsed and every other file's ranking (which is "worth listing") can
// move too.

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi, NativeApiUnavailableError, type SaveAccountsBody} from "$lib/api/native";
import {decodeAccountFileResponse, decodeAccountListing, decodeCreatedAccountsFile} from "$lib/api/nativeDecode";
import {createResource} from "$lib/stores/resource.svelte";
import {settings} from "$lib/stores/settings.svelte";
import type {AccountFile, AccountListing, CreatedAccountsFile} from "./types";

export const accountListing = createResource<string, AccountListing>(async (serverUrl) =>
    decodeAccountListing(await new LedgelineApi(serverUrl).listAccounts())
);

let available = $state(true);
let saving = $state(false);
let creating = $state(false);
/** Set by a 409: the file moved under us, so the open form is a stale base for any save. */
let conflict = $state(false);
/** The last load key, so a page visit after a prefetch does not re-read the tree. */
let listingKey: string | null = null;

export type AccountSaveOutcome = {ok: true; file: AccountFile} | {ok: false; failure: EditFailure};
export type CreateFileOutcome = {ok: true; created: CreatedAccountsFile} | {ok: false; failure: EditFailure};

export const accountsStore = {
    /** False once the engine has answered 404 for `/api/accounts` — an older engine. */
    get available(): boolean {
        return available;
    },
    get saving(): boolean {
        return saving;
    },
    get creating(): boolean {
        return creating;
    },
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

    /** Re-read the listing unconditionally (after a save, a create, or a Retry). */
    async reload(serverUrl: string): Promise<void> {
        conflict = false;
        await accountListing.load(serverUrl, serverUrl);
        available = !(accountListing.error instanceof NativeApiUnavailableError);
    },

    /**
     * Save one file's account lines.
     *
     * On success the engine answers with the file it actually wrote, at a
     * fresh revision — the caller re-seeds its form from THAT, not from what
     * it sent, for the same reason `aliasStore.save` does: an index is a
     * parse ordinal and this module's `Declare` can add one at any moment.
     */
    async save(journalId: string, body: SaveAccountsBody): Promise<AccountSaveOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        saving = true;
        try {
            const file = decodeAccountFileResponse(await new LedgelineApi(url).saveAccounts(journalId, body));
            conflict = false;
            void this.reload(url);
            return {ok: true, file};
        } catch (error) {
            const failure = classify(error);
            if (failure.kind === "conflict") conflict = true;
            return {ok: false, failure};
        } finally {
            saving = false;
        }
    },

    /** Create `accounts.journal` and `include` it, then reload the listing. */
    async createFile(): Promise<CreateFileOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        creating = true;
        try {
            const created = decodeCreatedAccountsFile(await new LedgelineApi(url).createAccountsFile());
            void this.reload(url);
            return {ok: true, created};
        } catch (error) {
            return {ok: false, failure: classify(error)};
        } finally {
            creating = false;
        }
    },
};
