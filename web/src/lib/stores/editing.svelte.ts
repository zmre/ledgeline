// Editing store: the write-path capability probe + the action dispatch that
// every add/edit/delete goes through. Actions call the native client, then —
// on success — refetch the journal (`journal.refresh()`) so the view reflects
// the change. Errors are classified into an `EditFailure` the callers surface:
// the popup shows validation messages inline; the journal page shows a global
// conflict banner (409 ⇒ the file changed on disk) and a transient toast for
// inline-edit failures.

import {
    ConflictError,
    LedgelineApi,
    NativeApiUnavailableError,
    NotFoundError,
    ValidationError,
    type AddTransactionBody,
    type PatchTransactionBody,
    type ReplaceTransactionBody,
} from "$lib/api/native";
import {ApiUnreachableError} from "$lib/api/client";
import {journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

export type EditFailureKind = "conflict" | "validation" | "notFound" | "unavailable" | "network" | "unknown";

export interface EditFailure {
    kind: EditFailureKind;
    message: string;
}

export type EditResult = {ok: true} | {ok: false; failure: EditFailure};

const OK: EditResult = {ok: true};

/** Map any thrown error onto the edit failure taxonomy (message is user-facing). */
function classify(error: unknown): EditFailure {
    if (error instanceof ConflictError) return {kind: "conflict", message: error.message};
    if (error instanceof ValidationError) return {kind: "validation", message: error.message};
    if (error instanceof NotFoundError) return {kind: "notFound", message: error.message};
    if (error instanceof NativeApiUnavailableError) return {kind: "unavailable", message: error.message};
    if (error instanceof ApiUnreachableError) return {kind: "network", message: error.message};
    return {kind: "unknown", message: error instanceof Error ? error.message : String(error)};
}

let canEdit = $state(false);
/** Non-null when the last probe could not REACH the server, so `canEdit` is stale rather than known. */
let probeError = $state<string | null>(null);
let busy = $state(false);
/** True after a 409: the file changed on disk — the page shows a refresh banner. */
let conflict = $state(false);
/** Transient failure from an INLINE edit (no modal to host the message) → a toast. */
let notice = $state<EditFailure | null>(null);

function client(): LedgelineApi | null {
    const url = settings.serverUrl;
    return url === null ? null : new LedgelineApi(url);
}

/**
 * Run one mutation, then refetch the journal on success. A 409 flips the global
 * `conflict` flag AND refetches (the server has already re-synced to disk), so
 * the user sees current data plus the "changed on disk" notice. `unavailable`
 * (501/non-native) turns editing off. Returns a typed result for the caller.
 */
async function run(action: (api: LedgelineApi) => Promise<unknown>): Promise<EditResult> {
    const api = client();
    if (api === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
    busy = true;
    try {
        await action(api);
        // `force`: a refresh already in flight issued its GETs BEFORE this
        // write, so joining it would confirm the edit against pre-write data
        // and then bank that as the current fingerprint — which suppresses the
        // swap for the correct data when it does arrive (FE-5e).
        await journal.refresh({force: true});
        conflict = false;
        return OK;
    } catch (error) {
        const failure = classify(error);
        if (failure.kind === "conflict") {
            conflict = true;
            await journal.refresh({force: true});
        } else if (failure.kind === "unavailable") {
            canEdit = false;
        }
        return {ok: false, failure};
    } finally {
        busy = false;
    }
}

export const editing = {
    /** True once the native engine's write route is confirmed present; gates every edit affordance. */
    get canEdit(): boolean {
        return canEdit;
    },
    /** A mutation (or its follow-up refetch) is in flight. */
    get busy(): boolean {
        return busy;
    },
    /** The journal changed on disk under an edit; the page shows a refresh banner until cleared. */
    get conflict(): boolean {
        return conflict;
    },
    clearConflict(): void {
        conflict = false;
    },
    /** The latest inline-edit failure, for a transient toast. */
    get notice(): EditFailure | null {
        return notice;
    },
    clearNotice(): void {
        notice = null;
    },
    /** Publish an inline-edit failure as a toast (the popup surfaces its own errors instead). */
    reportFailure(failure: EditFailure): void {
        notice = failure;
    },

    /** Why the last probe couldn't be answered, or null when it was. Non-null means `canEdit` is a GUESS. */
    get probeError(): string | null {
        return probeError;
    },

    /**
     * Probe write availability for the configured server (called when the URL is
     * set, and again after a refresh that follows a failed probe).
     *
     * A probe that comes back is authoritative: 404 means read-only, anything
     * else means the write routes are there. A probe that never comes back says
     * nothing at all, and treating the two alike is what let one dropped packet
     * remove every edit affordance — the Add button, the inline editors, the
     * popup — for the rest of the session, with no message and nothing to click
     * (FE-5g). So an unreachable probe keeps whatever was last known and records
     * why, leaving the retry to the caller.
     */
    async probe(): Promise<void> {
        const api = client();
        if (api === null) {
            canEdit = false;
            probeError = null;
            return;
        }
        try {
            canEdit = await api.probeEditing();
            probeError = null;
        } catch (error) {
            probeError = error instanceof Error ? error.message : String(error);
        }
    },

    /** ADD a whole transaction (popup, add mode). */
    add(body: AddTransactionBody): Promise<EditResult> {
        return run((api) => api.addTransaction(body));
    },
    /** REPLACE a whole transaction (popup, edit mode). */
    replace(index: number, body: ReplaceTransactionBody): Promise<EditResult> {
        return run((api) => api.replaceTransaction(index, body));
    },
    /** Surgical PATCH (inline description / account edits). */
    patch(index: number, patch: PatchTransactionBody): Promise<EditResult> {
        return run((api) => api.patchTransaction(index, patch));
    },
    /** DELETE a transaction (popup delete action). */
    remove(index: number): Promise<EditResult> {
        return run((api) => api.deleteTransaction(index));
    },
};
