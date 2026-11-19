// Whole transactions behind a settlement card's one-line sides (plan 45 Phase
// 3b): fetched lazily, on first expand, from the existing
// `GET /api/transactions/{index}`, so the list payload never grows.
//
// Cached by (index, generation): a new generation — any write, an outside
// edit — drops everything, because indices and contents may both have moved.
// A failed fetch is remembered as failed (the card says so with a Retry), not
// retried behind the user's back.
//
// `showAll` is the panel-wide "Show details": session state shared by the
// Invoices tab and the /settlements page, never a saved preference.

import {decodeRow} from "#lib/api/journalViewDecode.js";
import {LedgelineApi} from "#lib/api/native.js";
import type {Transaction} from "#lib/domain/types.js";
import {localToday} from "#lib/stores/filters.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";

export type TxnDetail = {state: "loading"} | {state: "ready"; txn: Transaction} | {state: "failed"; error: string};

/* eslint-disable svelte/prefer-svelte-reactivity -- the Map is an immutable
   snapshot replaced wholesale (`$state.raw`), as journalView's blocks are. */
let entries = $state.raw<ReadonlyMap<number, TxnDetail>>(new Map());
/** The generation `entries` describes. */
let cachedAt: string | null = null;
let showAll = $state(false);

function set(index: number, detail: TxnDetail): void {
    const next = new Map(entries);
    next.set(index, detail);
    entries = next;
}

/** Drop the cache when the journal has moved on since it was filled. */
function current(): void {
    const generation = journal.generation;
    if (generation !== cachedAt) {
        cachedAt = generation;
        entries = new Map();
    }
}

async function fetchOne(index: number): Promise<void> {
    const serverUrl = settings.serverUrl;
    if (serverUrl === null) return;
    const generation = cachedAt;
    set(index, {state: "loading"});
    try {
        const row = decodeRow(await new LedgelineApi(serverUrl).transaction(index, localToday()), `transaction ${index}`);
        if (generation !== cachedAt) return;
        set(index, {state: "ready", txn: row.txn});
    } catch (cause) {
        if (generation !== cachedAt) return;
        set(index, {state: "failed", error: cause instanceof Error ? cause.message : String(cause)});
    }
}

export const txnDetails = {
    /** What is known about transaction `index` at the current generation, or null (never asked). */
    get(index: number): TxnDetail | null {
        // Read `entries` first so a template that calls this re-runs when it changes.
        const held = entries.get(index) ?? null;
        return cachedAt === journal.generation ? held : null;
    },
    /** Make sure transaction `index` is fetched (once per generation). */
    request(index: number): void {
        current();
        if (entries.has(index)) return;
        void fetchOne(index);
    },
    /** Fetch transaction `index` again after a failure. */
    retry(index: number): void {
        current();
        void fetchOne(index);
    },
    /** The panel-wide "Show details" (session only). */
    get showAll(): boolean {
        return showAll;
    },
    set showAll(on: boolean) {
        showAll = on;
    },
};

/** Test seam: module-level runes state is shared across a test file. */
export function resetTxnDetails(): void {
    entries = new Map();
    cachedAt = null;
    showAll = false;
}
