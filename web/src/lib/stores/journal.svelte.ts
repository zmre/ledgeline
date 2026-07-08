// Journal store (WP-03): Svelte 5 runes state holding the normalized journal,
// plus the filtered $derived views consumed by the journal route. `refresh()`
// is called on startup (once settings.serverUrl is set) and by WP-08's poller;
// state is swapped only after a successful normalize, so old data stays
// visible on error (the route shows an error toast from `status`/`error`).

import {HledgerApi} from "$lib/api/client";
import {normalizePrices, normalizeTransactions} from "$lib/api/normalize";
import type {MixedAmount} from "$lib/domain/money";
import type {PriceDirective, Transaction} from "$lib/domain/types";
import {filteredTotals, filterTxns, sortTxnsDesc} from "$lib/journal/rowModel";
import {filters} from "$lib/stores/filters.svelte";
import {settings} from "$lib/stores/settings.svelte";

type JournalStatus = "idle" | "loading" | "ready" | "error";

let txns = $state<Transaction[]>([]);
let accountNames = $state<string[]>([]);
let prices = $state<PriceDirective[]>([]);
let status = $state<JournalStatus>("idle");
let error = $state<string | null>(null);
let fetchedAt = $state<number | null>(null);

let inFlight: Promise<void> | null = null;
let lastFingerprint = 0;

/**
 * Content-aware change fingerprint so polling refreshes don't churn every
 * $derived when nothing changed, while never discarding a REAL change — an
 * in-place edit to any transaction (recategorize, amount fix) must swap state,
 * not just appends. djb2 over each txn's index/date/status/haystack (the
 * haystack covers description, comments, accounts, amounts, commodities) plus
 * account names and price directives. Exported for unit tests.
 */
export function contentFingerprint(list: readonly Transaction[], names: readonly string[], priceList: readonly PriceDirective[]): number {
    let h = 5381;
    const mixStr = (s: string): void => {
        for (let i = 0; i < s.length; i += 1) h = (Math.imul(h, 33) + s.charCodeAt(i)) >>> 0;
    };
    for (const t of list) {
        h = (Math.imul(h, 33) + t.index) >>> 0;
        mixStr(t.date);
        mixStr(t.status);
        mixStr(t.haystack);
    }
    for (const name of names) mixStr(name);
    for (const p of priceList) {
        mixStr(p.date);
        mixStr(p.commodity);
        mixStr(p.price.commodity);
        h = (Math.imul(h, 33) + Number(BigInt.asIntN(32, p.price.qty.m))) >>> 0;
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
        const [rawTxns, nextNames, rawPrices] = await Promise.all([api.transactions(), api.accountNames(), api.prices()]);
        const nextTxns = normalizeTransactions(rawTxns);
        const nextPrices = normalizePrices(rawPrices);
        const nextFingerprint = contentFingerprint(nextTxns, nextNames, nextPrices);
        if (fetchedAt === null || nextFingerprint !== lastFingerprint) {
            txns = nextTxns;
            accountNames = nextNames;
            prices = nextPrices;
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
    get prices(): PriceDirective[] {
        return prices;
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
    /** Full refetch of /transactions, /accountnames, /prices. Concurrent calls share one in-flight request. */
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
const totals = $derived.by(() => filteredTotals(filtered, filters.value.accounts));

/** Transactions matching the current filters, sorted for display (date desc, index desc). */
export function getFilteredTxns(): Transaction[] {
    return filteredSorted;
}

/** Sum of postings in the selected accounts (all postings when none selected) within the filtered txns. */
export function getFilteredTotals(): MixedAmount {
    return totals;
}
