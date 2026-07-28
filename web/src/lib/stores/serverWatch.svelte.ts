// "Run this once the server is reachable, and again if the user reconnects."
//
// All three pages need the journal feed for something small but visible — the
// commodity display styles, the account list for the scope/filter choosers, the
// max depth for the slider — and all three had hand-written the same latch
// effect to fetch it. Two of the copies latched on the URL ALONE:
//
//     let attemptedUrl: string | null = null;
//     $effect(() => {
//         const url = settings.serverUrl;
//         if (url !== null && url !== attemptedUrl) { attemptedUrl = url; void journal.refresh(); }
//     });
//
// which is the FE-5d bug the journal route had already fixed and the other two
// had not: a reconnect normally leaves the URL identical (the engine restarted
// on the same port), so `url !== attemptedUrl` is false and the journal is
// never re-read. The reports and holdings pages already keyed their OWN report
// fetch on `serverNonce` for exactly this reason — only this side-effect was
// left behind. Keying on the nonce here fixes both by construction.

import {journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

/**
 * Call `run` once a server URL is configured, and again after each reconnect.
 *
 * Must be called during component initialization (it declares an `$effect`).
 * The latch is a plain non-reactive `let`, so writing it cannot re-trigger the
 * effect that reads it.
 */
export function onServerReady(run: (serverUrl: string) => void): void {
    let attempted: string | null = null;
    $effect(() => {
        const url = settings.serverUrl;
        // The nonce, not just the URL — see the note above.
        const key = `${settings.serverNonce}|${url}`;
        if (url !== null && key !== attempted) {
            attempted = key;
            run(url);
        }
    });
}

/** The common case: (re)load the journal feed as soon as the server is reachable. */
export function loadJournalWhenReady(): void {
    onServerReady(() => void journal.refresh());
}
