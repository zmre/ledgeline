// Journal store (WP-03): Svelte 5 runes state holding the normalized journal,
// plus the filtered $derived views consumed by the journal route. `refresh()`
// is called on startup (once settings.serverUrl is set) and by WP-08's poller;
// state is swapped only after a successful normalize, so old data stays
// visible on error (the route shows an error toast from `status`/`error`).

import {HledgerApi, isNotModified, resetConditionalCache} from "$lib/api/client";
import {LedgelineApi} from "$lib/api/native";
import {normalizeAccounts, normalizeDiagnostics, normalizePrices, normalizeTransactions} from "$lib/api/normalize";
import type {Problem} from "$lib/checks/engine";
import type {Dec} from "$lib/domain/money";
import type {AccountDecl} from "$lib/domain/accountTypes";
import type {PriceDirective, Transaction} from "$lib/domain/types";
import {filterTxns, sortTxnsDesc} from "$lib/journal/rowModel";
import {filters} from "$lib/stores/filters.svelte";
import {settings} from "$lib/stores/settings.svelte";

type JournalStatus = "idle" | "loading" | "ready" | "error";

let txns = $state<Transaction[]>([]);
let accountNames = $state<string[]>([]);
let accountDecls = $state<AccountDecl[]>([]);
let prices = $state<PriceDirective[]>([]);
let diagnostics = $state<Problem[]>([]);
let engineChecked = $state(false);
let status = $state<JournalStatus>("idle");
let error = $state<string | null>(null);
let fetchedAt = $state<number | null>(null);

let inFlight: Promise<void> | null = null;
/** The server URL the in-flight round is asking; a reconnect elsewhere must not join it. */
let inFlightUrl: string | null = null;
/** Cancels the in-flight round when a newer one supersedes it. */
let inFlightAbort: AbortController | null = null;
/**
 * Monotonic round id. Only the newest round may touch state.
 *
 * Without it a round could still be answering after it had been superseded and
 * would write its (older) result anyway — most damagingly `lastFingerprint`,
 * which is the one variable that decides whether a LATER, correct result gets
 * swapped in at all.
 */
let roundToken = 0;
let lastFingerprint = 0;

/**
 * Content-aware change fingerprint so polling refreshes don't churn every
 * $derived when nothing changed, while never discarding a REAL change — an
 * in-place edit to any transaction (recategorize, amount fix) must swap state,
 * not just appends. djb2 over each txn's index/dates/status/haystack plus its
 * postings' account, status, date, type, balance assertion and EXACT amounts,
 * then account names, price directives, and declared account types (a `type:`
 * edit must recompute the cash-flow report). Exported for unit tests.
 *
 * The haystack alone is NOT enough, even though it mentions amounts: it is
 * built for searching, so its amounts run through `formatAmount`, which rounds
 * to `MAX_DISPLAY_DECIMALS` (2). Anything that changes only below that — a
 * sub-cent broker fee, a share count's third decimal — renders identically and
 * would hash the same, so the refresh after an edit would fetch the new data
 * and then discard it, leaving stale transactions on screen and stale problems
 * in the badge. Hashing `qty` exactly (mantissa AND scale, plus any cost
 * annotation) is what makes those edits visible.
 */
export function contentFingerprint(
    list: readonly Transaction[],
    names: readonly string[],
    priceList: readonly PriceDirective[],
    decls: readonly AccountDecl[] = [],
    diags: readonly Problem[] = []
): number {
    let h = 5381;
    const mixStr = (s: string): void => {
        for (let i = 0; i < s.length; i += 1) h = (Math.imul(h, 33) + s.charCodeAt(i)) >>> 0;
    };
    /** Exact, unrounded: the full mantissa as text plus its scale. */
    const mixDec = (d: Dec): void => {
        mixStr(d.m.toString());
        mixStr(String(d.p));
    };
    for (const t of list) {
        h = (Math.imul(h, 33) + t.index) >>> 0;
        mixStr(t.date);
        // Secondary date: nothing else here mixes it, and it is absent from the
        // haystack, so adding/clearing one used to hash identically.
        mixStr(t.date2 ?? "");
        mixStr(t.status);
        mixStr(t.haystack);
        for (const posting of t.postings) {
            mixStr(posting.account);
            // Per-posting status and date, the posting type (`(a)` / `[a]`) and
            // the balance assertion are all invisible to the haystack, so an
            // edit that touches only one of them produced an unchanged
            // fingerprint — the refresh fetched the new journal and then threw
            // it away, permanently (see `lastFingerprint` in doRefresh).
            mixStr(posting.status);
            mixStr(posting.date ?? "");
            mixStr(posting.type ?? "regular");
            if (posting.balanceAssertion !== undefined) {
                const assertion = posting.balanceAssertion;
                mixStr(assertion.amount.commodity);
                mixDec(assertion.amount.qty);
                mixStr(`${assertion.inclusive ? "*" : ""}${assertion.total ? "=" : ""}`);
            }
            for (const amount of posting.amounts) {
                mixStr(amount.commodity);
                mixDec(amount.qty);
                if (amount.cost !== undefined) {
                    mixStr(amount.cost.commodity);
                    mixStr(amount.cost.per ? "@" : "@@");
                    mixDec(amount.cost.qty);
                }
            }
        }
    }
    for (const name of names) mixStr(name);
    for (const p of priceList) {
        mixStr(p.date);
        mixStr(p.commodity);
        mixStr(p.price.commodity);
        // `mixDec`, exactly like every other amount. The old line mixed
        // `Number(BigInt.asIntN(32, qty.m))` — mantissa only, truncated to 32
        // bits — so `P VTI $1.00` (m=100n,p=2) and `P VTI $100` (m=100n,p=0)
        // hashed the same, as did any two 8-dp mantissas 2^32 apart. Prices
        // drive holdings and every valuation report, and a skipped swap also
        // rewrites `lastFingerprint`, so a collision stranded a stale portfolio
        // on screen for good rather than for one poll.
        mixDec(p.price.qty);
    }
    for (const d of decls) {
        mixStr(d.name);
        mixStr(d.type ?? "");
    }
    // Engine diagnostics are hashed too: they are computed from state this
    // function only hashes APPROXIMATELY (a fixed balance assertion need not
    // change any amount we mix), so leaving them out would let a refresh fetch
    // resolved diagnostics and then discard them, stranding a red badge.
    for (const d of diags) {
        h = (Math.imul(h, 33) + d.txnIndex) >>> 0;
        mixStr(d.rule);
        mixStr(d.severity);
        mixStr(d.message);
    }
    return h;
}

/**
 * One fetch round.
 *
 * `token` is this round's id: every write below is guarded on it still being
 * the newest, so a round that has been superseded (by a reconnect, or by the
 * forced refetch after a write) resolves without touching state — including
 * `lastFingerprint`, whose whole job is to decide what the NEXT round is
 * allowed to swap in.
 *
 * `unconditional` forces a full refetch by first forgetting every recorded
 * ETag; `doRefresh` sets it for the single retry it does when a conditional
 * round comes back mixed (see below). That retry is a continuation of the same
 * round, so it keeps the same token and signal.
 */
async function doRefresh(token: number, unconditional: boolean, signal: AbortSignal): Promise<void> {
    const baseUrl = settings.serverUrl;
    if (baseUrl === null) {
        if (token !== roundToken) return;
        status = "error";
        error = "No hledger-web server configured";
        return;
    }
    if (unconditional) resetConditionalCache();
    if (token === roundToken) status = "loading";
    try {
        // `conditional`: this is the one caller that repeats these requests
        // forever (a 30-second poll) and can act on a 304 by keeping what it has.
        const api = new HledgerApi(baseUrl, undefined, {conditional: true, signal});
        // Diagnostics are advisory and Ledgeline-native: a plain hledger-web has
        // no such route. Failing it to `null` here (rather than letting it reject
        // the whole `Promise.all`) is what keeps the journal loading against any
        // engine — an unreachable or absent route means "no diagnostics", never a
        // failed load.
        const [rawTxns, nextNames, rawPrices, rawAccounts, rawDiagnostics] = await Promise.all([
            api.transactions(),
            api.accountNames(),
            api.prices(),
            api.accounts(),
            new LedgelineApi(baseUrl, {signal}).diagnostics().catch(() => null),
        ]);

        // Superseded while the network was answering: these payloads describe a
        // server, or a moment, the app has already moved on from. Falling
        // through would write `lastFingerprint` from them and make the newer
        // round's swap look like a no-op.
        if (token !== roundToken) return;

        // Every journal payload comes from one server-side snapshot carrying one
        // ETag, so an unchanged journal answers 304 on all three of the big
        // conditional routes and there is nothing to normalize, hash or swap —
        // the point of PERF-2. (`/accountnames` is small and unconditional; it
        // derives from the same snapshot, so these three vouch for it.)
        if (isNotModified(rawTxns) && isNotModified(rawPrices) && isNotModified(rawAccounts)) {
            // `fetchedAt === null` means we have ETags but no state to keep — a
            // round that recorded them and then failed to normalize. Refetch.
            if (fetchedAt === null) {
                await doRefresh(token, true, signal);
                return;
            }
            fetchedAt = Date.now();
            status = "ready";
            error = null;
            if (import.meta.env.DEV) console.debug("[journal] 304 — nothing refetched");
            return;
        }
        if (isNotModified(rawTxns) || isNotModified(rawPrices) || isNotModified(rawAccounts)) {
            // A journal swap landed mid-round, so some routes answered 304 against
            // the OLD snapshot while others returned the NEW one. The halves cannot
            // be combined; drop the tags and take one clean unconditional round.
            // Bounded: the retry sends no `If-None-Match`, so it cannot come back
            // mixed again.
            if (unconditional) throw new Error("the server answered 304 to an unconditional request");
            await doRefresh(token, true, signal);
            return;
        }

        const nextTxns = normalizeTransactions(rawTxns);
        const nextPrices = normalizePrices(rawPrices);
        const nextDecls = normalizeAccounts(rawAccounts);
        // Wire positions resolve against exactly the array the engine indexed, so
        // this must be the transactions from THIS same fetch round. Never throws.
        const nextDiagnostics = normalizeDiagnostics(rawDiagnostics, nextTxns);
        // `null` only when the route was absent/unreachable. An engine that
        // answered with zero diagnostics HAS checked, and the local unbalanced
        // rule must stand down for it.
        const nextEngineChecked = rawDiagnostics !== null;
        const nextFingerprint = contentFingerprint(nextTxns, nextNames, nextPrices, nextDecls, nextDiagnostics);
        if (fetchedAt === null || nextFingerprint !== lastFingerprint) {
            txns = nextTxns;
            accountNames = nextNames;
            prices = nextPrices;
            accountDecls = nextDecls;
            diagnostics = nextDiagnostics;
            engineChecked = nextEngineChecked;
            if (import.meta.env.DEV) console.debug(`[journal] state swapped (${nextTxns.length} txns)`);
        } else if (import.meta.env.DEV) {
            console.debug("[journal] poll unchanged — state swap skipped");
        }
        lastFingerprint = nextFingerprint;
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

/**
 * Begin a new round, superseding and cancelling any round already running.
 *
 * The abort is what makes a hung server recoverable: without it the previous
 * round holds its socket until the deadline, and the promise every caller is
 * waiting on with it.
 */
function startRound(): Promise<void> {
    const token = ++roundToken;
    inFlightAbort?.abort();
    const controller = new AbortController();
    inFlightAbort = controller;
    inFlightUrl = settings.serverUrl;
    const promise = doRefresh(token, false, controller.signal).finally(() => {
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
    /** Normalized, frozen transactions in journal order. */
    get txns(): Transaction[] {
        return txns;
    },
    get accountNames(): string[] {
        return accountNames;
    },
    /** Declared account types from /accounts (name + `type:` tag); [] until the first successful refresh. */
    get accountDecls(): AccountDecl[] {
        return accountDecls;
    },
    get prices(): PriceDirective[] {
        return prices;
    },
    /** Engine-computed diagnostics for the current journal; [] when the engine sends none (or is an older build). */
    get diagnostics(): Problem[] {
        return diagnostics;
    },
    /** Whether the engine answered `/api/diagnostics` — so [] means "clean", not
     * "unchecked". False against a plain hledger-web. */
    get engineChecked(): boolean {
        return engineChecked;
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
     * Full refetch of /transactions, /accountnames, /prices, /accounts.
     *
     * Concurrent calls share one in-flight round — but only when that round is
     * answering the same question. Two cases must never join one:
     *
     *  - `force`, for any caller that has just CHANGED something the round is
     *    reading. A round's GETs went out before the write did, so joining it
     *    resolves the write "ok" against pre-edit data and then records the
     *    pre-edit fingerprint, which suppresses the swap for the real result
     *    too. Visibly: toggle one row's status, then another's, and the second
     *    badge doesn't move for up to 30 seconds — and since the cycle button
     *    steps from the DISPLAYED status, clicking again advances the journal
     *    file two steps (FE-5e).
     *  - a different `serverUrl`. Reconnecting handed back the OLD server's
     *    promise, and every page guard had already latched, so nothing retried
     *    against the new one (FE-5d).
     */
    refresh(options?: {force?: boolean}): Promise<void> {
        if (inFlight !== null && options?.force !== true && inFlightUrl === settings.serverUrl) return inFlight;
        return startRound();
    },
};

/**
 * Live-update polling loop (WP-08). Refreshes every `intervalMs` (default 30s)
 * via `journal.refresh()` — which already dedups concurrent calls and skips the
 * state swap when the fingerprint is unchanged. Pauses while the document is
 * hidden; on becoming visible again it refreshes immediately and resumes. On
 * fetch errors `refresh()` keeps stale data and sets `status = "error"`, which
 * the layout surfaces as a red status dot with a reconnect affordance.
 * Returns a stop function.
 */
export function startPolling(intervalMs = 30_000): () => void {
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

// Filtered views (WP-03 contract): pure derivation logic lives in
// lib/journal/rowModel.ts; these wrappers just wire it to the runes graph.
const filtered = $derived.by(() => filterTxns(txns, filters.value));
const filteredSorted = $derived.by(() => sortTxnsDesc(filtered));

/** Transactions matching the current filters, sorted for display (date desc, index desc). */
export function getFilteredTxns(): Transaction[] {
    return filteredSorted;
}
