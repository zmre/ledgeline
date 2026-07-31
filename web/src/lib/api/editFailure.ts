// The write path's error taxonomy, in one place.
//
// Every mutating surface in the SPA — transaction add/edit/delete, and now the
// import-rules editor — has to answer the same four questions about a thrown
// error: is it a conflict (refetch and let the user re-apply), a validation
// refusal (show the server's sentence inline), a missing capability (turn the
// affordance off), or a network failure (retry)? Getting that mapping wrong is
// how a 409 becomes an anonymous red box, so it is written once here rather than
// re-derived per store.
//
// A pure module with no Svelte state, so it is unit-testable under node and can
// be shared by stores that must not import one another (`stores/editing.svelte`
// pulls in the whole journal feed; the rules editor has no business doing that).

import {ApiUnreachableError} from "./client";
import {ConflictError, NativeApiUnavailableError, NotFoundError, ValidationError} from "./native";

export type EditFailureKind = "conflict" | "validation" | "notFound" | "unavailable" | "network" | "unknown";

export interface EditFailure {
    kind: EditFailureKind;
    /** The server's own plain-text message where there is one — shown to the user verbatim. */
    message: string;
}

export type EditResult = {ok: true} | {ok: false; failure: EditFailure};

/** Map any thrown error onto the edit failure taxonomy (message is user-facing). */
export function classify(error: unknown): EditFailure {
    if (error instanceof ConflictError) return {kind: "conflict", message: error.message};
    if (error instanceof ValidationError) return {kind: "validation", message: error.message};
    if (error instanceof NotFoundError) return {kind: "notFound", message: error.message};
    if (error instanceof NativeApiUnavailableError) return {kind: "unavailable", message: error.message};
    if (error instanceof ApiUnreachableError) return {kind: "network", message: error.message};
    return {kind: "unknown", message: error instanceof Error ? error.message : String(error)};
}
