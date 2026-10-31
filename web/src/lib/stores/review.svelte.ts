// The review RUN store (plan 32 D5, D5a, D12, D14): one pass through a review
// queue, card by card, with the network kept off the critical path.
//
//   - QUEUE — `/api/review/queue` returns every card's full transaction, so
//     stepping between cards never waits. Unknowns page at 100; the next page
//     loads as the cursor nears the end of what is held.
//   - SIMILAR — fetched once per card, keyed on its ORIGINAL description (the
//     engine's), for the card on screen and the next one, so the section is
//     there the moment a card is.
//   - SAVES — queued and sent strictly in order. `save` advances the cursor at
//     once (optimistic); the write happens behind it. A failure stops the run:
//     the cursor rewinds to the failed card, its draft intact, with the error,
//     and every save queued after it is cancelled (their drafts kept too).
//   - D5a — a batch's result updates the run at once: in Unknowns a queued
//     card the save categorised leaves the run ("9 fixed via similar"); in
//     Recents it stays, showing the applied change and which card made it.
//     Then the queue is re-validated against the new generation in the
//     background and merged, keeping the run's order, skips and current card.
//
// The run lives in module state like every store here (one overlay at a
// time); `resetReview` is the test seam.

/* eslint-disable svelte/prefer-svelte-reactivity -- the similar cache is an
   immutable snapshot replaced wholesale (`$state.raw`); `inflight` is plain
   bookkeeping no template or effect observes. */
import {classify, type EditFailure, type EditResult} from "#lib/api/editFailure.js";
import {LedgelineApi, type WireBatchEdit, type WireEditOp} from "#lib/api/native.js";
import {decodeBatchResult, decodeQueuePage, decodeSimilarSet} from "#lib/api/reviewDecode.js";
import type {QueueItem, QueueKind, ReviewForm, ReviewTxn, SimilarSet} from "#lib/review/types.js";
import {localToday} from "./filters.svelte";
import {journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

/** Where a card stands in the run. */
export type CardStatus = "open" | "saving" | "saved" | "skipped";

/** One card of the run. */
export interface ReviewCard {
    /** The transaction as it now reads (refreshed by a save or a revalidation). */
    item: QueueItem;
    status: CardStatus;
    /** Unsaved edits, kept while moving between cards and across a failed save. */
    draft: ReviewForm | null;
    /** D5a (Recents): the index of the card whose save changed this one, for "updated from card N". */
    updatedFrom: number | null;
}

/** The queue settings a run was opened with (D19). */
export interface RunOptions {
    /** Recents' cap (default 50); Unknowns' page size (default 100). */
    limit?: number;
    /** The lookback window in days; 0 = no limit (default 365). */
    maxDays?: number;
}

/** A save that stopped the run. */
export interface SaveFailure {
    /** The failed card's transaction index. */
    index: number;
    failure: EditFailure;
}

interface Run {
    serverUrl: string;
    kind: QueueKind;
    today: string;
    limit: number | undefined;
    maxDays: number | undefined;
    /** The generation the held cards were last reconciled with. */
    generation: string;
    /** The queue's size when it was last fetched. */
    total: number;
    notEditable: number;
    /** How many queue items have been fetched (the next page's offset). */
    fetched: number;
}

interface PendingSave {
    card: number;
    edits: WireBatchEdit[];
    resolve: (result: EditResult) => void;
}

/** Load the next Unknowns page once the cursor is this close to the end of what is held. */
const PAGE_AHEAD = 10;

let run = $state.raw<Run | null>(null);
let cards = $state.raw<readonly ReviewCard[]>([]);
let cursor = $state(0);
let loading = $state(false);
let error = $state<string | null>(null);
let failure = $state.raw<SaveFailure | null>(null);
/** Cards that left the run because a similar-apply categorised them (D5a). */
let fixed = $state(0);
let similarCache = $state.raw<ReadonlyMap<number, SimilarSet>>(new Map());
let saving = $state(0);

/** Bumped on every open/reset; a response from an older epoch is dropped. */
let epoch = 0;
let revalidation = 0;
const inflight = new Map<number, Promise<SimilarSet | null>>();
let queue: PendingSave[] = [];
let draining: Promise<void> | null = null;
let paging = false;

function api(serverUrl: string): LedgelineApi {
    return new LedgelineApi(serverUrl);
}

const indexOf = (card: ReviewCard): number => card.item.txn.index;

/** A fresh review transaction over a held queue item, keeping the item's key and suggestions. */
function refreshed(item: QueueItem, txn: ReviewTxn): QueueItem {
    return Object.freeze({...item, ...txn});
}

function cardOf(item: QueueItem): ReviewCard {
    return {item, status: "open", draft: null, updatedFrom: null};
}

/** Replace `cards`, keeping the cursor on the same card (or, if it left, on whatever took its place). */
function replaceCards(next: readonly ReviewCard[]): void {
    const previous = cards;
    const current = previous[cursor];
    cards = next;
    if (current === undefined) {
        cursor = Math.min(cursor, next.length);
        return;
    }
    const at = next.findIndex((card) => indexOf(card) === indexOf(current));
    if (at >= 0) {
        cursor = at;
        return;
    }
    // It left: land on the first surviving card that was after it, which is
    // as many places in as there are survivors from before it.
    const before = new Set(previous.slice(0, cursor).map(indexOf));
    cursor = next.filter((card) => before.has(indexOf(card))).length;
}

/** The queued indices still to be worked through, for `?queue=`. */
function openIndices(): number[] {
    return cards.filter((card) => card.status === "open" || card.status === "skipped").map(indexOf);
}

async function fetchSimilar(index: number): Promise<SimilarSet | null> {
    const target = run;
    if (target === null) return null;
    const cached = similarCache.get(index);
    if (cached !== undefined && cached.generation === target.generation) return cached;
    const pending = inflight.get(index);
    if (pending !== undefined) return pending;
    const started = epoch;
    const request = (async (): Promise<SimilarSet | null> => {
        try {
            const set = decodeSimilarSet(await api(target.serverUrl).reviewSimilar(index, target.today, openIndices()));
            if (started !== epoch) return null;
            const next = new Map(similarCache);
            next.set(index, set);
            similarCache = next;
            return set;
        } catch {
            // Similar is an aid, never a blocker: a card without it is still a card.
            return null;
        } finally {
            inflight.delete(index);
        }
    })();
    inflight.set(index, request);
    return request;
}

/** D12: the card on screen and the next one have their similar sets on the way. */
function prefetch(): void {
    for (const card of [cards[cursor], cards[cursor + 1]]) {
        if (card !== undefined && card.status !== "saved") void fetchSimilar(indexOf(card));
    }
    void pageAhead();
}

/** Unknowns are paged: fetch the next page when the cursor nears the end of the held cards. */
async function pageAhead(): Promise<void> {
    const target = run;
    if (target === null || paging || target.fetched >= target.total || cursor < cards.length - PAGE_AHEAD) return;
    paging = true;
    const started = epoch;
    try {
        const page = decodeQueuePage(
            await api(target.serverUrl).reviewQueue(target.kind, {today: target.today, limit: target.limit, maxDays: target.maxDays, offset: target.fetched})
        );
        if (started !== epoch || run === null) return;
        const held = new Set(cards.map(indexOf));
        replaceCards([...cards, ...page.items.filter((item) => !held.has(item.txn.index)).map(cardOf)]);
        run = {...run, fetched: run.fetched + page.items.length, total: page.total, notEditable: page.notEditable};
    } catch (cause) {
        if (started === epoch) error = cause instanceof Error ? cause.message : String(cause);
    } finally {
        paging = false;
    }
}

/** The next card after `from` a pending batch is not about to change, for the optimistic advance. */
function nextFree(from: number, busy: ReadonlySet<number>): number {
    let at = from + 1;
    // In Unknowns, a queued card this save is applying to will most likely
    // leave the run when it lands (D5a): do not land on it.
    while (run?.kind === "unknowns" && at < cards.length && busy.has(indexOf(cards[at] as ReviewCard))) at += 1;
    return at;
}

/** D5a: fold a batch's result into the run. */
function applyResult(card: number, edited: readonly ReviewTxn[], generation: string): void {
    const byIndex = new Map(edited.map((txn) => [txn.txn.index, txn]));
    const kind = run?.kind;
    let removed = 0;
    const next: ReviewCard[] = [];
    for (const held of cards) {
        const fresh = byIndex.get(indexOf(held));
        if (fresh === undefined) {
            next.push(held);
        } else if (indexOf(held) === card) {
            next.push({...held, item: refreshed(held.item, fresh), status: "saved", draft: null});
        } else if (kind === "unknowns" && !fresh.unknown && (held.status === "open" || held.status === "skipped")) {
            removed += 1;
        } else {
            next.push({...held, item: refreshed(held.item, fresh), updatedFrom: card});
        }
    }
    fixed += removed;
    replaceCards(next);
    if (run !== null) run = {...run, generation};
    similarCache = new Map();
    inflight.clear();
}

/** A save failed: rewind to its card, keep every draft, cancel what was queued behind it. */
function stop(card: number, cause: EditFailure): void {
    const cancelled = queue;
    queue = [];
    const reopened = new Set([card, ...cancelled.map((save) => save.card)]);
    cards = cards.map((held) => (reopened.has(indexOf(held)) && held.status === "saving" ? {...held, status: "open"} : held));
    const at = cards.findIndex((held) => indexOf(held) === card);
    if (at >= 0) cursor = at;
    failure = {index: card, failure: cause};
    for (const save of cancelled) save.resolve({ok: false, failure: {kind: "unknown", message: "Not saved: an earlier save failed."}});
}

async function drain(): Promise<void> {
    while (queue.length > 0) {
        const save = queue[0] as PendingSave;
        const target = run;
        if (target === null) return;
        const started = epoch;
        try {
            const result = decodeBatchResult(await api(target.serverUrl).batchTransactions({edits: save.edits}));
            // The run was closed or reopened while this was in flight: the
            // write landed, but there is no run left to fold it into.
            if (started !== epoch) return;
            queue.shift();
            saving = queue.length;
            applyResult(save.card, result.edited, result.generation);
            save.resolve({ok: true});
            prefetch();
        } catch (cause) {
            if (started !== epoch) return;
            queue.shift();
            const classified = classify(cause);
            stop(save.card, classified);
            saving = 0;
            save.resolve({ok: false, failure: classified});
            if (classified.kind === "conflict") void revalidate();
            void journal.refresh({force: true});
            return;
        }
    }
    saving = 0;
    // D14: the journal page catches up as after any edit; the overlay never waits for it.
    void journal.refresh({force: true});
    void revalidate();
}

function startDrain(): void {
    if (draining !== null) return;
    draining = drain().finally(() => {
        draining = null;
        if (queue.length > 0) startDrain();
    });
}

/**
 * D5a's background re-validation: re-read the queue (as many items as are
 * held) at the current generation and merge it into the run.
 *
 * - The run's order, its skips and its saved cards stay.
 * - A held card the queue no longer lists (another tab fixed it, an external
 *   edit) is dropped — unless it is the card on screen, which is never pulled
 *   out from under the user.
 * - A held card whose text changed underneath is refreshed, and its draft
 *   dropped (it was made against text that no longer exists), except on the
 *   card on screen, which keeps it.
 * - Queue items the run has not seen are appended.
 */
async function revalidate(): Promise<void> {
    const target = run;
    if (target === null || queue.length > 0) return;
    const token = ++revalidation;
    const started = epoch;
    try {
        const page = decodeQueuePage(
            await api(target.serverUrl).reviewQueue(target.kind, {
                today: target.today,
                limit: Math.max(target.fetched, target.limit ?? 0) || undefined,
                maxDays: target.maxDays,
            })
        );
        if (token !== revalidation || started !== epoch || run === null || queue.length > 0) return;
        const fresh = new Map(page.items.map((item) => [item.txn.index, item]));
        const current = cards[cursor];
        const next: ReviewCard[] = [];
        for (const held of cards) {
            const item = fresh.get(indexOf(held));
            const onScreen = held === current;
            if (held.status === "saved") next.push(held);
            else if (item === undefined) {
                if (onScreen) next.push(held);
            } else if (item.fingerprint !== held.item.fingerprint) {
                next.push({...held, item, draft: onScreen ? held.draft : null});
            } else next.push({...held, item});
        }
        const held = new Set(cards.map(indexOf));
        for (const item of page.items) if (!held.has(item.txn.index)) next.push(cardOf(item));
        replaceCards(next);
        run = {...run, generation: page.generation, total: page.total, notEditable: page.notEditable, fetched: Math.max(run.fetched, page.items.length)};
        prefetch();
    } catch {
        // A failed re-validation leaves the run as it was; the next save or
        // generation change tries again.
    }
}

export const review = {
    /** The queue the run is working through, or null when no run is open. */
    get kind(): QueueKind | null {
        return run?.kind ?? null;
    },
    /** The generation the run was last reconciled with. */
    get generation(): string | null {
        return run?.generation ?? null;
    },
    /** Every card of the run, in run order (saved and skipped ones included, so ← can return to them). */
    get cards(): readonly ReviewCard[] {
        return cards;
    },
    /** The card on screen, or null past the last one (the run's summary). */
    get current(): ReviewCard | null {
        return cards[cursor] ?? null;
    },
    get cursor(): number {
        return cursor;
    },
    /** The header's "N of M" (1-based), which D5a's removals recompute. */
    get position(): number {
        return Math.min(cursor + 1, cards.length);
    },
    get count(): number {
        return cards.length;
    },
    /** The queue's own size when last fetched (more than `count` while Unknowns are still paging). */
    get total(): number {
        return run?.total ?? 0;
    },
    /** How many qualifying transactions live in files that cannot be written. */
    get notEditable(): number {
        return run?.notEditable ?? 0;
    },
    get loading(): boolean {
        return loading;
    },
    /** The last queue-load failure. */
    get error(): string | null {
        return error;
    },
    /** How many saves are queued or in flight (the header's "saving…" dot). */
    get saving(): number {
        return saving;
    },
    /** The save that stopped the run, until `clearFailure` or the next save. */
    get failure(): SaveFailure | null {
        return failure;
    },
    clearFailure(): void {
        failure = null;
    },
    /** The run's summary: cards reviewed (saved), skipped, and fixed via similar (D5a). */
    get tally(): {saved: number; skipped: number; fixed: number} {
        return {
            saved: cards.filter((card) => card.status === "saved").length,
            skipped: cards.filter((card) => card.status === "skipped").length,
            fixed,
        };
    },

    /** Open a run over `kind` as of the browser's today. Resolves once the first page is in. */
    async open(kind: QueueKind, options: RunOptions = {}): Promise<void> {
        const serverUrl = settings.serverUrl;
        resetReview();
        if (serverUrl === null) {
            error = "No server is configured.";
            return;
        }
        const started = epoch;
        const today = localToday();
        loading = true;
        try {
            const page = decodeQueuePage(await api(serverUrl).reviewQueue(kind, {today, limit: options.limit, maxDays: options.maxDays}));
            if (started !== epoch) return;
            run = {
                serverUrl,
                kind,
                today,
                limit: options.limit,
                maxDays: options.maxDays,
                generation: page.generation,
                total: page.total,
                notEditable: page.notEditable,
                fetched: page.items.length,
            };
            cards = page.items.map(cardOf);
            cursor = 0;
            error = null;
            prefetch();
        } catch (cause) {
            if (started === epoch) error = cause instanceof Error ? cause.message : String(cause);
        } finally {
            if (started === epoch) loading = false;
        }
    },

    /** Close the run (the overlay's ✕): forget it. Saves already sent still land. */
    close(): void {
        resetReview();
    },

    /** Move to card `at` (clamped; `count` is the summary). */
    go(at: number): void {
        cursor = Math.max(0, Math.min(at, cards.length));
        prefetch();
    },
    /** Skip (D5): this run only; the card comes back next time. */
    skip(): void {
        const card = cards[cursor];
        if (card === undefined) return;
        if (card.status === "open") cards = cards.map((held, at) => (at === cursor ? {...held, status: "skipped"} : held));
        review.go(cursor + 1);
    },
    /** ← : the previous card, whatever happened to it. */
    back(): void {
        if (cursor > 0) review.go(cursor - 1);
    },
    /** Keep (or, with null, discard) the card on screen's unsaved edits. */
    setDraft(draft: ReviewForm | null): void {
        if (cards[cursor] === undefined) return;
        cards = cards.map((held, at) => (at === cursor ? {...held, draft} : held));
    },

    /** `index`'s similar set when it has arrived (reactive), else null. */
    similar(index: number): SimilarSet | null {
        const set = similarCache.get(index);
        return set !== undefined && set.generation === run?.generation ? set : null;
    },
    /** `index`'s similar set, fetched if it is not held. Null when it cannot be had. */
    loadSimilar(index: number): Promise<SimilarSet | null> {
        return fetchSimilar(index);
    },

    /**
     * Save the card on screen with `ops`, plus `similar` — each a candidate from
     * its similar set with the ops to apply to it (empty ones are left out) —
     * as one batch, and advance at once. The returned promise settles when
     * THIS save lands or fails; the UI need not wait for it.
     */
    save(ops: readonly WireEditOp[], similar: readonly {txn: ReviewTxn; ops: readonly WireEditOp[]}[] = []): Promise<EditResult> {
        const card = cards[cursor];
        if (card === undefined || run === null) return Promise.resolve({ok: false, failure: {kind: "unknown", message: "There is no card to save."}});
        const entries = [{txn: card.item, ops}, ...similar.filter((entry) => entry.ops.length > 0)];
        const unprintable = entries.find((entry) => entry.txn.fingerprint === null);
        if (unprintable !== undefined) {
            return Promise.resolve({ok: false, failure: {kind: "unavailable", message: "Editing is not enabled on this server."}});
        }
        const edits: WireBatchEdit[] = entries.map((entry) => ({
            index: entry.txn.txn.index,
            fingerprint: entry.txn.fingerprint as string,
            ops: [...entry.ops],
        }));
        failure = null;
        cards = cards.map((held, at) => (at === cursor ? {...held, status: "saving"} : held));
        const busy = new Set(edits.slice(1).map((edit) => edit.index));
        const result = new Promise<EditResult>((resolve) => queue.push({card: indexOf(card), edits, resolve}));
        saving = queue.length;
        review.go(nextFree(cursor, busy));
        startDrain();
        return result;
    },

    /** `i` (Q1): mark the card on screen `reviewed:ignore` and advance. */
    ignore(): Promise<EditResult> {
        return review.save([{op: "setTxnTag", name: "reviewed", value: "ignore"}]);
    },

    /**
     * Tell the run the journal's generation (the root's poller has it). A
     * change made elsewhere — another tab, an external editor — re-validates
     * the queue in the background; the run's own saves already did.
     */
    syncGeneration(generation: string | null): void {
        if (run === null || generation === null || generation === run.generation || queue.length > 0) return;
        void revalidate();
    },

    /** Test seam: resolves once every queued save has landed or failed. */
    async settled(): Promise<void> {
        while (draining !== null) await draining;
    },
};

/** Test seam: forget the run (vitest runs every test against one module instance). */
export function resetReview(): void {
    epoch += 1;
    revalidation += 1;
    run = null;
    cards = [];
    cursor = 0;
    loading = false;
    error = null;
    failure = null;
    fixed = 0;
    similarCache = new Map();
    saving = 0;
    inflight.clear();
    for (const save of queue) save.resolve({ok: false, failure: {kind: "unknown", message: "The review was closed."}});
    queue = [];
    paging = false;
}
