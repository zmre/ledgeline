// One async report store, written once.
//
// `reports`, `insights`, `subscriptions` and `holdingsData` were four copies of
// the same 33-line quartet — `payload`/`status`/`error`/`seq`, three getters and
// a token/try/catch `load` — differing mostly by a type name. Four copies meant
// the two invariants below had to be re-derived (and were, at different times,
// got wrong) in each:
//
//   1. STALE RESPONSES. A monotonic token is taken before the await and
//      re-checked after it; a response whose token has been superseded is
//      dropped rather than written. Without it, params changing faster than the
//      network answers lets an older response land last and win.
//
//   2. THE PAYLOAD AND THE QUESTION IT ANSWERS ARE ONE VALUE (FE-1). Held
//      separately they drift: `bs`/`is` are both SectionedReport and `cf`/`nw`
//      are both PeriodReport, so a payload's own shape cannot say which request
//      it belongs to and NO type error is possible when they are mixed up. A
//      balance sheet that was already loaded stayed on screen — and in the
//      export — when the P&L tab's fetch failed, under the P&L's label. Storing
//      `{query, value}` as one assignment is what lets a surface refuse.
//
// Note what `load` deliberately does NOT do on failure: it leaves the previous
// `{query, value}` in place. That is not an oversight — it is what lets a
// surface keep rendering the last good answer while `status` says the newest
// request failed, and it is why the error branch may never be gated on the
// payload being null (see `dataView`).

import {dataView, type DataView, type LoadStatus} from "./loadState";

/** An async payload fetched for a query, plus everything a surface needs to render it. */
export interface Resource<Q, T> {
    /** The last successfully fetched payload, or null before the first success. */
    readonly value: T | null;
    /** The query `value` came from — compare against the live one before rendering or exporting it. */
    readonly query: Q | null;
    readonly status: LoadStatus;
    readonly error: Error | null;
    /**
     * Which branch to render, for the common case where any held payload is
     * worth showing. Surfaces whose held payload can answer a question the user
     * is no longer asking (the reports tabs) call `dataView` directly with their
     * own `matchesRequest`.
     */
    readonly view: DataView;
    /** Fetch for `query`; a response superseded by a newer load is discarded. */
    load(serverUrl: string, query: Q): Promise<void>;
}

/**
 * Build an async store around `fetcher`.
 *
 * Call once at module scope: the returned object owns module-level `$state` and
 * is the singleton the app shares, exactly as the four hand-written stores were.
 */
export function createResource<Q, T>(fetcher: (serverUrl: string, query: Q) => Promise<T>): Resource<Q, T> {
    // One value, never two — see (2) above.
    let loaded = $state<{query: Q; value: T} | null>(null);
    let status = $state<LoadStatus>("idle");
    let error = $state<Error | null>(null);
    let seq = 0;

    return {
        get value(): T | null {
            return loaded?.value ?? null;
        },
        get query(): Q | null {
            return loaded?.query ?? null;
        },
        get status(): LoadStatus {
            return status;
        },
        get error(): Error | null {
            return error;
        },
        get view(): DataView {
            return dataView(status, loaded !== null);
        },
        async load(serverUrl: string, query: Q): Promise<void> {
            const token = ++seq;
            status = "loading";
            try {
                const next = await fetcher(serverUrl, query);
                if (token !== seq) return;
                loaded = {query, value: next};
                status = "ready";
                error = null;
            } catch (cause) {
                if (token !== seq) return;
                status = "error";
                error = cause instanceof Error ? cause : new Error(String(cause));
            }
        },
    };
}
