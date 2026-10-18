// The journal VIEW store (plan 26 B5): the filtered journal, fetched a window
// at a time from the engine instead of downloaded whole.
//
// One view at a time, keyed on (server, generation, filter):
//
//   - SUMMARY — `/api/journal/summary`: the filtered count, the footer's visible
//     net, the big numbers and the chart selectors.
//   - ROWS — `/api/journal/rows`, in BLOCKS of 200 in display order, fetched as
//     the table scrolls (`ensure`), with the neighbouring block prefetched. A
//     row not loaded yet reads as null and the table draws a skeleton for it.
//
// A FILTER change is a different list: everything is dropped. A GENERATION
// change (an edit, a file change) is the same list, re-read: the held blocks
// stay on screen, marked stale, until their fresh copies arrive — so an inline
// edit does not flash the table to skeletons.
//
// The store never holds more than MAX_BLOCKS blocks; the ones furthest from
// where the table last looked are evicted first. Scrolling a 200k-row journal
// end to end therefore costs a few thousand rows of memory, not 200k.

/* eslint-disable svelte/prefer-svelte-reactivity -- the block Map is an
   immutable snapshot replaced wholesale (`$state.raw`), so a SvelteMap would
   add per-key signals nothing reads; `pending` is plain bookkeeping that no
   template or effect observes. */
import {LedgelineApi, type JournalQueryFilter} from "#lib/api/native.js";
import {decodeJournalSummary, decodeRowPosition, decodeRowsPage} from "#lib/api/journalViewDecode.js";
import type {Transaction} from "#lib/domain/types.js";
import type {JournalRow, JournalSummary, RowPosition} from "#lib/journalView/types.js";
import {localToday} from "./filters.svelte";
import {journal} from "./journal.svelte";

/** Rows per block: one request's worth, and the granularity of the cache. */
export const BLOCK_SIZE = 200;
/** Most blocks held at once (10,000 rows). */
const MAX_BLOCKS = 50;
/** Rows per request when paging the whole view (the export). The engine caps a request at 1,000. */
const PAGE_SIZE = 1000;

/** The question the held view answers. */
export interface ViewQuery {
    serverUrl: string;
    generation: string;
    filter: JournalQueryFilter;
}

interface Block {
    generation: string;
    rows: readonly JournalRow[];
}

/** Filters are compared by value: `accounts` is a Set, so its sorted members stand in for it. */
export function filterKey(filter: JournalQueryFilter): string {
    return JSON.stringify([filter.from, filter.to, [...filter.accounts].sort(), filter.query]);
}

let query = $state.raw<ViewQuery | null>(null);
let key: string | null = null;
let summary = $state.raw<{key: string; value: JournalSummary} | null>(null);
/** The filtered count, from whichever answered first for this filter: the summary or a block. */
let total = $state<number | null>(null);
let blocks = $state.raw<ReadonlyMap<number, Block>>(new Map());
let error = $state<string | null>(null);

/** Bumped on every filter change; a response from an older epoch is dropped. */
let epoch = 0;
let summaryToken = 0;
/** `${epoch}:${generation}:${block}` for every block request in flight. */
const pending = new Set<string>();
/** The block the table last asked around — eviction keeps the ones nearest it. */
let focusBlock = 0;

function api(serverUrl: string): LedgelineApi {
    return new LedgelineApi(serverUrl);
}

function failed(cause: unknown): void {
    error = cause instanceof Error ? cause.message : String(cause);
}

async function loadSummary(target: ViewQuery, targetKey: string): Promise<void> {
    const token = ++summaryToken;
    try {
        const value = decodeJournalSummary(await api(target.serverUrl).journalSummary(target.filter));
        if (token !== summaryToken || targetKey !== key) return;
        summary = {key: targetKey, value};
        total = value.total;
        error = null;
    } catch (cause) {
        if (token === summaryToken) failed(cause);
    }
}

/** Keep the MAX_BLOCKS blocks nearest the focus, dropping the rest. */
function evict(map: Map<number, Block>): void {
    if (map.size <= MAX_BLOCKS) return;
    const byDistance = [...map.keys()].sort((a, b) => Math.abs(b - focusBlock) - Math.abs(a - focusBlock));
    for (const block of byDistance.slice(0, map.size - MAX_BLOCKS)) map.delete(block);
}

async function loadBlock(target: ViewQuery, block: number): Promise<void> {
    const ticket = `${epoch}:${target.generation}:${block}`;
    if (pending.has(ticket)) return;
    pending.add(ticket);
    const startedEpoch = epoch;
    try {
        const page = decodeRowsPage(
            await api(target.serverUrl).journalRows(target.filter, {offset: block * BLOCK_SIZE, limit: BLOCK_SIZE, today: localToday()})
        );
        if (startedEpoch !== epoch) return;
        const next = new Map(blocks);
        next.set(block, {generation: page.generation, rows: page.rows});
        evict(next);
        blocks = next;
        total = page.total;
        error = null;
        // The engine moved on (an edit elsewhere, a file change) before the
        // metadata poll noticed: catch the metadata up, which re-keys the view.
        if (page.generation !== target.generation) void journal.refresh();
    } catch (cause) {
        if (startedEpoch === epoch) failed(cause);
    } finally {
        pending.delete(ticket);
    }
}

export const journalView = {
    /** The (server, generation, filter) the view is showing, or null before the first `sync`. */
    get query(): ViewQuery | null {
        return query;
    },
    /** The summary for the CURRENT filter (possibly from the previous generation while a fresh one loads), or null. */
    get summary(): JournalSummary | null {
        return summary !== null && summary.key === key ? summary.value : null;
    },
    /** How many rows the filter keeps, or null until the engine has said. */
    get total(): number | null {
        return total;
    },
    /** The last request failure, cleared by the next success. */
    get error(): string | null {
        return error;
    },
    /** The row at `position` in display order, or null while its block loads. */
    rowAt(position: number): JournalRow | null {
        const block = blocks.get(Math.floor(position / BLOCK_SIZE));
        return block?.rows[position % BLOCK_SIZE] ?? null;
    },

    /**
     * Point the view at (server, generation, filter). Cheap to call on every
     * reactive tick: nothing happens unless one of the three moved.
     */
    sync(serverUrl: string, generation: string, filter: JournalQueryFilter): void {
        const nextKey = filterKey(filter);
        const sameList = query !== null && query.serverUrl === serverUrl && key === nextKey;
        if (sameList && query?.generation === generation) return;
        const target: ViewQuery = {serverUrl, generation, filter};
        if (!sameList) {
            // A different list: nothing held describes it.
            epoch += 1;
            key = nextKey;
            blocks = new Map();
            total = null;
            focusBlock = 0;
        }
        query = target;
        void loadSummary(target, nextKey);
        // The first block alongside the summary: it is what the table shows
        // first, and it answers `total` too. A generation change re-reads the
        // blocks the table asks for next, around where it is looking.
        if (!sameList) void loadBlock(target, 0);
    },

    /**
     * Make rows [start, end) available: fetch every block they touch that is
     * missing or stale, plus one block either side so a scroll lands on rows
     * that are already there.
     */
    ensure(start: number, end: number): void {
        const target = query;
        if (target === null || end <= start) return;
        const first = Math.floor(start / BLOCK_SIZE);
        const last = Math.floor((end - 1) / BLOCK_SIZE);
        focusBlock = Math.floor((first + last) / 2);
        const limit = total === null ? last + 1 : Math.ceil(total / BLOCK_SIZE);
        for (let block = Math.max(0, first - 1); block <= Math.min(last + 1, limit - 1); block += 1) {
            if (blocks.get(block)?.generation !== target.generation) void loadBlock(target, block);
        }
    },

    /** Where the transaction with tindex `index` sits in the current view (null position: filtered out). */
    async locate(index: number): Promise<RowPosition | null> {
        const target = query;
        if (target === null) return null;
        return decodeRowPosition(await api(target.serverUrl).journalPosition(target.filter, index));
    },

    /**
     * Every row of the current view, oldest first — the journal export. Paged
     * at the engine's per-request cap rather than through the block cache, so
     * exporting does not evict what the table is showing.
     */
    async allRows(): Promise<Transaction[]> {
        const target = query;
        if (target === null) return [];
        const out: Transaction[] = [];
        for (let offset = 0; ; offset += PAGE_SIZE) {
            const page = decodeRowsPage(await api(target.serverUrl).journalRows(target.filter, {offset, limit: PAGE_SIZE, order: "asc", today: localToday()}));
            out.push(...page.rows.map((row) => row.txn));
            if (page.rows.length < PAGE_SIZE || out.length >= page.total) return out;
        }
    },
};

/** Test seam: forget everything (vitest runs every test against one module instance). */
export function resetJournalView(): void {
    epoch += 1;
    summaryToken += 1;
    query = null;
    key = null;
    summary = null;
    total = null;
    blocks = new Map();
    error = null;
    focusBlock = 0;
    pending.clear();
}
