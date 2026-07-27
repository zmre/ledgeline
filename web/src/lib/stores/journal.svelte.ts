// Journal store (WP-03): Svelte 5 runes state holding the normalized journal,
// plus the filtered $derived views consumed by the journal route. `refresh()`
// is called on startup (once settings.serverUrl is set) and by WP-08's poller;
// state is swapped only after a successful normalize, so old data stays
// visible on error (the route shows an error toast from `status`/`error`).

import {HledgerApi} from "$lib/api/client";
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
let lastFingerprint = 0;

/**
 * Content-aware change fingerprint so polling refreshes don't churn every
 * $derived when nothing changed, while never discarding a REAL change — an
 * in-place edit to any transaction (recategorize, amount fix) must swap state,
 * not just appends. djb2 over each txn's index/date/status/haystack plus its
 * postings' EXACT amounts, then account names, price directives, and declared
 * account types (a `type:` edit must recompute the cash-flow report). Exported
 * for unit tests.
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
        mixStr(t.status);
        mixStr(t.haystack);
        for (const posting of t.postings) {
            mixStr(posting.account);
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
        h = (Math.imul(h, 33) + Number(BigInt.asIntN(32, p.price.qty.m))) >>> 0;
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

async function doRefresh(): Promise<void> {
    const baseUrl = settings.serverUrl;
    if (baseUrl === null) {
        status = "error";
        error = "No hledger-web server configured";
        return;
    }
    status = "loading";
    try {
        const api = new HledgerApi(baseUrl);
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
            new LedgelineApi(baseUrl).diagnostics().catch(() => null),
        ]);
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
        status = "error";
        error = cause instanceof Error ? cause.message : String(cause);
    }
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
    /** Full refetch of /transactions, /accountnames, /prices, /accounts. Concurrent calls share one in-flight request. */
    refresh(): Promise<void> {
        inFlight ??= doRefresh().finally(() => {
            inFlight = null;
        });
        return inFlight;
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
