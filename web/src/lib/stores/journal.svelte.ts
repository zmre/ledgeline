// Journal METADATA store (WP-03, reshaped by plan 26 B5): which journal is
// open, its account names and classes, its styles, and the snapshot
// GENERATION everything else keys on.
//
// The journal itself is no longer downloaded. Rows, totals and charts come
// from the engine's windowed routes for exactly the slice on screen
// (`stores/journalView.svelte.ts`); this store is one small `/api/journal`
// request per round, and a changed `generation` is the signal that every
// journal-derived view is stale (an edit, a file change, File→Open).
//
// `refresh()` is called on startup, by the 5-second poller (every screen, from the root layout), after every edit
// (`force`), and by the app-wide refresh. State is swapped only after a
// successful round, so the old metadata stays visible on error (the route shows
// an error toast from `status`/`error`).

import {LedgelineApi} from "#lib/api/native.js";
import {decodeJournalInfo} from "#lib/api/nativeDecode.js";
import type {CommodityStyles} from "#lib/api/normalize.js";
import {NO_ACCOUNT_CLASSES, type AccountClasses} from "#lib/domain/accountClass.js";
import {settings} from "#lib/stores/settings.svelte.js";

type JournalStatus = "idle" | "loading" | "ready" | "error";

// `$state.raw`: these are replaced wholesale on a generation change and never
// mutated in place, and the account tables can be large.
let accountNames = $state.raw<string[]>([]);
let reportStyles = $state.raw<CommodityStyles>(new Map());
let accountClasses = $state.raw<AccountClasses>(NO_ACCOUNT_CLASSES);
let dominantCommodity = $state<string>("$");
/** The engine's name for the journal on screen, and its main file — the app bar's label. */
let title = $state<string | null>(null);
let file = $state<string | null>(null);
let txnCount = $state(0);
let generation = $state<string | null>(null);
/** Bumped each time the generation moves from one known snapshot to another. See `journal.changes`. */
let changes = $state(0);
let status = $state<JournalStatus>("idle");
let error = $state<string | null>(null);
let fetchedAt = $state<number | null>(null);

let inFlight: Promise<void> | null = null;
/** The server URL the in-flight round is asking; a reconnect elsewhere must not join it. */
let inFlightUrl: string | null = null;
/** Cancels the in-flight round when a newer one supersedes it. */
let inFlightAbort: AbortController | null = null;
/**
 * Monotonic round id. Only the newest round may touch state: a superseded
 * round that answers late must never overwrite a newer one's metadata.
 */
let roundToken = 0;

/**
 * One fetch round. Every write is guarded on `token` still being the newest.
 *
 * WHY PER ROUND, and not once per connection: the desktop File→Open action
 * rebinds the engine to a DIFFERENT journal at the SAME URL, and nothing about
 * the connection changes when it does. The title and the generation travel
 * with the round that re-reads everything else.
 */
async function doRefresh(token: number, signal: AbortSignal): Promise<void> {
    const baseUrl = settings.serverUrl;
    if (baseUrl === null) {
        if (token !== roundToken) return;
        status = "error";
        error = "No hledger-web server configured";
        return;
    }
    if (token === roundToken) status = "loading";
    try {
        // The SPA requires the Ledgeline engine (plan 26): there is no
        // plain-hledger-web fallback to keep alive.
        const info = decodeJournalInfo(await new LedgelineApi(baseUrl, {signal}).journalInfo());
        if (token !== roundToken) return;
        title = info.title;
        file = info.file;
        // A generation is unique per snapshot, so an unchanged one means
        // nothing journal-derived changed: skip the swap and leave every
        // `$derived` downstream untouched.
        if (info.generation !== generation) {
            // `/api/journal` lists every account `/accountnames` does, with its
            // class — one request instead of two. Sorted again here because a
            // JS object puts integer-like keys (an account named `1000`) first.
            accountNames = [...info.accounts.keys()].sort();
            accountClasses = info.accounts;
            reportStyles = info.reportStyles;
            dominantCommodity = info.dominantCommodity;
            txnCount = info.txnCount;
            if (generation !== null) changes += 1;
            generation = info.generation;
            if (import.meta.env.DEV) console.debug(`[journal] generation ${info.generation} (${info.txnCount} txns)`);
        }
        fetchedAt = Date.now();
        status = "ready";
        error = null;
    } catch (cause) {
        // A superseded round's failure is not the user's problem — most of the
        // time it IS the supersession (we aborted it ourselves).
        if (token !== roundToken) return;
        status = "error";
        error = cause instanceof Error ? cause.message : String(cause);
    }
}

/** Begin a new round, superseding and cancelling any round already running. */
function startRound(): Promise<void> {
    const token = ++roundToken;
    inFlightAbort?.abort();
    const controller = new AbortController();
    inFlightAbort = controller;
    inFlightUrl = settings.serverUrl;
    const promise = doRefresh(token, controller.signal).finally(() => {
        // A newer round owns these now; clearing them would strand it.
        if (token !== roundToken) return;
        inFlight = null;
        inFlightUrl = null;
        inFlightAbort = null;
    });
    inFlight = promise;
    return promise;
}

export const journal = {
    /** Every account the journal knows (posted-to, declared, implied ancestors), sorted. */
    get accountNames(): string[] {
        return accountNames;
    },
    /** Every account the journal knows, as the ENGINE classified it (type + term); empty until the first successful refresh. */
    get accountClasses(): AccountClasses {
        return accountClasses;
    },
    /** The style report totals format with, per posted commodity (engine-computed; see `JournalInfo.reportStyles`). */
    get reportStyles(): CommodityStyles {
        return reportStyles;
    },
    /** The journal's most-posted commodity — the ONE "which commodity" rule; "$" until loaded. */
    get dominantCommodity(): string {
        return dominantCommodity;
    },
    /** How many transactions the journal holds. */
    get txnCount(): number {
        return txnCount;
    },
    /**
     * The engine snapshot the metadata came from, or null before the first
     * successful round. Compared for equality only: a change means every
     * journal-derived view (rows, totals, problems, balances) is stale.
     */
    get generation(): string | null {
        return generation;
    },
    /**
     * How many times the journal has changed under this page: an edit, a file
     * changed on disk, File→Open. Starts at 0 and does NOT move on the first
     * successful round, which only reveals what was already there.
     *
     * For views fetched by their own query (the reports, holdings, budget,
     * projections) to put in their load key, so a change refetches what is on
     * screen instead of waiting for the user to leave and come back. Keying on
     * `generation` itself would fetch everything twice at startup, once before
     * the first round and once when the generation first appears, and gating
     * on it would hold a report hostage to the metadata round (FE-5c).
     */
    get changes(): number {
        return changes;
    },
    /**
     * What the engine calls the journal on screen (its first-line comment, else
     * its folder), for the app bar. Null when the journal has neither, and
     * before the first successful round.
     */
    get title(): string | null {
        return title;
    },
    /** The bare filename of the main journal file (never a path); null on the same terms as `title`. */
    get file(): string | null {
        return file;
    },
    get status(): JournalStatus {
        return status;
    },
    get error(): string | null {
        return error;
    },
    get fetchedAt(): number | null {
        return fetchedAt;
    },
    /**
     * Re-read `/api/journal`.
     *
     * Concurrent calls share one in-flight round — but only when that round is
     * answering the same question. Two cases must never join one:
     *
     *  - `force`, for any caller that has just CHANGED something. A round whose
     *    request went out before the write would report the pre-edit generation,
     *    and the views keyed on it would never refetch (FE-5e).
     *  - a different `serverUrl`: reconnecting must not hand back the OLD
     *    server's promise (FE-5d).
     */
    refresh(options?: {force?: boolean}): Promise<void> {
        if (inFlight !== null && options?.force !== true && inFlightUrl === settings.serverUrl) return inFlight;
        return startRound();
    },
};

/**
 * Live-update polling loop (WP-08). Refreshes every `intervalMs` (default 5s)
 * via `journal.refresh()`, which dedups concurrent calls; a changed generation
 * is what makes the views refetch. Pauses while the document is hidden; on
 * becoming visible again it refreshes immediately and resumes. Returns a stop
 * function.
 */
export function startPolling(intervalMs = 5_000): () => void {
    let timer: ReturnType<typeof setInterval> | null = null;
    const start = (): void => {
        timer ??= setInterval(() => void journal.refresh(), intervalMs);
    };
    const stop = (): void => {
        if (timer !== null) {
            clearInterval(timer);
            timer = null;
        }
    };
    const onVisibilityChange = (): void => {
        if (document.visibilityState === "hidden") {
            stop();
        } else {
            void journal.refresh();
            start();
        }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    start();
    return () => {
        stop();
        document.removeEventListener("visibilitychange", onVisibilityChange);
    };
}
