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
// A run has a SOURCE (plan 32 Phase 4). "journal" is all of the above.
// "staged" runs over an import's transactions before they are written
// (`openStaged`): the cards are handed in, nothing is paged or revalidated,
// and a save goes to the import store's sink as overrides (for the card and
// its staged siblings) and queued journal edits (for similar transactions
// already in the journal, D24) — never to the network. Similar is fetched by
// the card's similarity key, since a staged card has no journal index.
//
// A run may also be SINGLE (plan 32 D31): one transaction, opened from the
// Journal to edit it (`openEdit`) or to add a new one (`openAdd`). There is
// no queue: nothing pages or revalidates, the similar set is fetched for the
// one card, and a save is not optimistic. It goes through `editing` (a batch,
// an add or a delete), which refreshes the journal and raises the page's
// conflict banner on a 409, as every other journal edit does.
//
// The run lives in module state like every store here (one overlay at a
// time); `resetReview` is the test seam.

/* eslint-disable svelte/prefer-svelte-reactivity -- the similar cache is an
   immutable snapshot replaced wholesale (`$state.raw`); `inflight` is plain
   bookkeeping no template or effect observes. */
import {classify, type EditFailure, type EditResult} from "#lib/api/editFailure.js";
import {LedgelineApi, type AddTransactionBody, type WireBatchEdit, type WireEditOp} from "#lib/api/native.js";
import {
    decodeBatchResult,
    decodeDescriptionSuggestions,
    decodeKeySimilarSet,
    decodeQueuePage,
    decodeReviewItem,
    decodeSimilarSet,
} from "#lib/api/reviewDecode.js";
import type {StagedSink, StagedTxn} from "#lib/imports/reviewTypes.js";
import type {QueueItem, QueueKind, ReviewForm, ReviewTxn, SimilarSet} from "#lib/review/types.js";
import {editing} from "./editing.svelte";
import {localToday} from "./filters.svelte";
import {journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

/** What a staged run covers: the import's unknowns, or every transaction it would write (D26's "Review All"). */
export type StagedKind = "unknowns" | "all";

/** A single-transaction run (D31): editing one transaction, or adding one. */
export type SingleMode = "edit" | "add";

/** Where a run's cards come from and where its saves go. */
export type ReviewSource = "journal" | "staged";

/** One similar transaction a save is applied to: a journal one (→ a journal edit) or, in a staged run, a staged sibling (→ an override). */
export interface SimilarEntry {
    txn: ReviewTxn | StagedTxn;
    ops: readonly WireEditOp[];
}

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
    source: ReviewSource;
    kind: QueueKind | StagedKind | SingleMode;
    /** A single-transaction run (D31), or null for a queue or an import's run. */
    single: SingleMode | null;
    /** A staged run's save target; null for a journal run. */
    sink: StagedSink | null;
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
/** Bumped per add-mode suggestion request; only the latest one lands. */
let suggestion = 0;
let revalidation = 0;
const inflight = new Map<number, Promise<SimilarSet | null>>();
let queue: PendingSave[] = [];
let draining: Promise<void> | null = null;
let paging = false;

function api(serverUrl: string): LedgelineApi {
    return new LedgelineApi(serverUrl);
}

const indexOf = (card: ReviewCard): number => card.item.txn.index;

function isStaged(txn: ReviewTxn | StagedTxn): txn is StagedTxn {
    return "ref" in txn;
}

/** A save's ops that recategorise: an account moved, or a full re-render. What takes a staged sibling out of an Unknowns run. */
function categorises(ops: readonly WireEditOp[]): boolean {
    return ops.some((op) => op.op === "setPostingAccount" || op.op === "replace");
}

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
    // A staged run reconciles with no generation: its similar sets are kept for the run.
    if (cached !== undefined && (target.source === "staged" || cached.generation === target.generation)) return cached;
    const pending = inflight.get(index);
    if (pending !== undefined) return pending;
    const started = epoch;
    // A staged card has no journal index: its journal siblings are found by the
    // similarity key of what the bank sent (D24).
    const key = target.source === "staged" ? (cards.find((card) => indexOf(card) === index)?.item.key ?? null) : null;
    if (target.source === "staged" && key === null) return null;
    const request = (async (): Promise<SimilarSet | null> => {
        try {
            const set =
                target.source === "staged"
                    ? decodeKeySimilarSet(await api(target.serverUrl).reviewSimilarKey(key as string, target.today))
                    : decodeSimilarSet(await api(target.serverUrl).reviewSimilar(index, target.today, openIndices()));
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
    // An add card is no transaction yet: it has nothing similar to it.
    if (run?.single === "add") return;
    for (const card of [cards[cursor], cards[cursor + 1]]) {
        if (card !== undefined && card.status !== "saved") void fetchSimilar(indexOf(card));
    }
    void pageAhead();
}

/** Unknowns are paged: fetch the next page when the cursor nears the end of the held cards. */
async function pageAhead(): Promise<void> {
    const target = run;
    if (
        target === null ||
        target.source === "staged" ||
        target.single !== null ||
        paging ||
        target.fetched >= target.total ||
        cursor < cards.length - PAGE_AHEAD
    )
        return;
    paging = true;
    const started = epoch;
    try {
        const page = decodeQueuePage(
            await api(target.serverUrl).reviewQueue(target.kind as QueueKind, {
                today: target.today,
                limit: target.limit,
                maxDays: target.maxDays,
                offset: target.fetched,
            })
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
 * - Queue items the run has not seen are appended — in Unknowns only.
 *   Recents is "the N most recent unreviewed", fixed when the run opens:
 *   each save drops a card out of the server's top N and lets the next one
 *   in, so appending made a 50-card run grow by one per save (Patrick saw
 *   "60 of 90"). Unknowns has no count limit, so its newcomers really are
 *   more of the same queue.
 */
async function revalidate(): Promise<void> {
    const target = run;
    if (target === null || target.source === "staged" || target.single !== null || queue.length > 0) return;
    const token = ++revalidation;
    const started = epoch;
    try {
        const page = decodeQueuePage(
            await api(target.serverUrl).reviewQueue(target.kind as QueueKind, {
                today: target.today,
                // Recents re-reads past its own size, so newer arrivals pushing
                // a held card out of the top N don't read as "it left".
                limit:
                    target.kind === "recents"
                        ? Math.max(target.fetched, target.limit ?? 0) + cards.length
                        : Math.max(target.fetched, target.limit ?? 0) || undefined,
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
        if (target.kind !== "recents") for (const item of page.items) if (!held.has(item.txn.index)) next.push(cardOf(item));
        replaceCards(next);
        run = {...run, generation: page.generation, total: page.total, notEditable: page.notEditable, fetched: Math.max(run.fetched, page.items.length)};
        prefetch();
    } catch {
        // A failed re-validation leaves the run as it was; the next save or
        // generation change tries again.
    }
}

export const review = {
    /** The queue the run is working through (or the single run's mode), or null when no run is open. */
    get kind(): QueueKind | StagedKind | SingleMode | null {
        return run?.kind ?? null;
    },
    /** A single-transaction run's mode (D31), or null. */
    get single(): SingleMode | null {
        return run?.single ?? null;
    },
    /** Where the run's cards come from: the journal's queues, or an import's staged transactions. */
    get source(): ReviewSource | null {
        return run?.source ?? null;
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
                source: "journal",
                kind,
                single: null,
                sink: null,
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

    /**
     * Open a run over an import's staged transactions (plan 32 D23): `unknowns`
     * keeps those with an uncategorised posting, `all` keeps every one. Saves go
     * to `sink`, synchronously; nothing is written until the import is.
     * `generation` is null for the run's whole life — there is no queue to
     * reconcile with.
     */
    openStaged(kind: StagedKind, items: readonly StagedTxn[], sink: StagedSink): void {
        const serverUrl = settings.serverUrl;
        resetReview();
        if (serverUrl === null) {
            error = "No server is configured.";
            return;
        }
        const held = kind === "unknowns" ? items.filter((item) => item.unknown) : [...items];
        run = {
            serverUrl,
            source: "staged",
            kind,
            single: null,
            sink,
            today: localToday(),
            limit: undefined,
            maxDays: undefined,
            generation: "",
            total: held.length,
            notEditable: 0,
            fetched: held.length,
        };
        cards = held.map(cardOf);
        cursor = 0;
        error = null;
        prefetch();
    },

    /**
     * The OTHER staged transactions in this run sharing `index`'s similarity
     * key (D24's "In this import" group), in run order. Empty in a journal run.
     */
    stagedSimilar(index: number): StagedTxn[] {
        if (run?.source !== "staged") return [];
        const key = cards.find((card) => indexOf(card) === index)?.item.key ?? null;
        if (key === null) return [];
        return cards.filter((card) => indexOf(card) !== index && card.item.key === key && card.status !== "saved").map((card) => card.item as StagedTxn);
    },

    /**
     * Open a single-transaction run on journal transaction `index` (D31): one
     * fetch for the card (its row, fingerprint, key and suggestions), then
     * its similar set behind it, so the card is editable before that lands.
     */
    async openEdit(index: number): Promise<void> {
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
            const {generation, item} = decodeReviewItem(await api(serverUrl).reviewItem(index, today));
            if (started !== epoch) return;
            run = singleRun(serverUrl, "edit", today, generation);
            cards = [cardOf(item)];
            cursor = 0;
            error = null;
            prefetch();
        } catch (cause) {
            if (started === epoch) error = cause instanceof Error ? cause.message : String(cause);
        } finally {
            if (started === epoch) loading = false;
        }
    },

    /**
     * Open a single-transaction run for a NEW transaction (D36). Its card is
     * `blank`, a transaction that is in no file: index -1, no fingerprint, no
     * key and no suggestions until `suggestFor` is asked.
     */
    openAdd(blank: QueueItem): void {
        const serverUrl = settings.serverUrl;
        resetReview();
        if (serverUrl === null) {
            error = "No server is configured.";
            return;
        }
        run = singleRun(serverUrl, "add", localToday(), "");
        cards = [cardOf(blank)];
        cursor = 0;
        error = null;
    },

    /**
     * D36: the suggestions for `description` (a payee just picked), put on
     * the add card. A failure leaves the card as it was: suggestions are an
     * aid. Only the latest request lands.
     */
    async suggestFor(description: string): Promise<void> {
        const target = run;
        if (target === null || target.single !== "add") return;
        const token = ++suggestion;
        const started = epoch;
        try {
            const found = decodeDescriptionSuggestions(await api(target.serverUrl).reviewSuggest(description, target.today));
            if (token !== suggestion || started !== epoch) return;
            cards = cards.map((held) => ({...held, item: Object.freeze({...held.item, key: found.key, suggestions: found.suggestions})}));
        } catch {
            // Keep the card as it is.
        }
    },

    /**
     * Save a single run's card: `ops` on it, plus `similar` (D38), as one
     * batch through `editing`. Not optimistic: the card waits, marked
     * "saving". Answers the transaction's index as it now reads. A failure
     * other than a conflict is the run's `failure`; a conflict is the page's
     * banner.
     */
    async saveSingle(ops: readonly WireEditOp[], similar: readonly SimilarEntry[] = []): Promise<SingleOutcome> {
        const card = cards[cursor];
        if (card === undefined || run?.single !== "edit") return notSaved("There is no transaction to save.");
        const entries = [{txn: card.item, ops}, ...similar.filter((entry) => entry.ops.length > 0)];
        if (entries.some((entry) => entry.txn.fingerprint === null)) return notSaved("Editing is not enabled on this server.", "unavailable");
        const edits: WireBatchEdit[] = entries.map((entry) => ({
            index: entry.txn.txn.index,
            fingerprint: entry.txn.fingerprint as string,
            ops: [...entry.ops],
        }));
        return settleSingle(card, async () => {
            const result = await editing.batch(edits);
            return result.ok ? {ok: true, index: result.value.edited[0]?.txn.index ?? indexOf(card)} : result;
        });
    },

    /** Add the single run's new transaction (D36): one POST, its reviewed stamp, if any, in `body`'s comment. Answers its index. */
    addSingle(body: AddTransactionBody): Promise<SingleOutcome> {
        const card = cards[cursor];
        if (card === undefined || run?.single !== "add") return Promise.resolve(notSaved("There is no transaction to add."));
        return settleSingle(card, async () => {
            const result = await editing.add(body);
            return result.ok ? {ok: true, index: result.value} : result;
        });
    },

    /** Delete the single run's transaction (D37). */
    deleteSingle(): Promise<SingleOutcome> {
        const card = cards[cursor];
        if (card === undefined || run?.single !== "edit") return Promise.resolve(notSaved("There is no transaction to delete."));
        return settleSingle(card, async () => {
            const result = await editing.remove(indexOf(card));
            return result.ok ? {ok: true, index: null} : result;
        });
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
        return set !== undefined && (run?.source === "staged" || set.generation === run?.generation) ? set : null;
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
    save(ops: readonly WireEditOp[], similar: readonly SimilarEntry[] = []): Promise<EditResult> {
        const card = cards[cursor];
        if (card === undefined || run === null) return Promise.resolve({ok: false, failure: {kind: "unknown", message: "There is no card to save."}});
        if (run.source === "staged") return Promise.resolve(saveStaged(card, ops, similar));
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
        // A staged run's cards come from the import, not the journal's queue.
        // A single run's one card is checked by its save's fingerprint.
        if (run?.source === "staged" || (run?.single ?? null) !== null) return;
        if (run === null || generation === null || generation === run.generation || queue.length > 0) return;
        void revalidate();
    },

    /** Test seam: resolves once every queued save has landed or failed. */
    async settled(): Promise<void> {
        while (draining !== null) await draining;
    },
};

/**
 * A staged run's save (D23/D24): the card and its staged siblings become
 * overrides, similar journal transactions become queued journal edits, all
 * handed to the import's sink at once. Nothing goes to the network, so it
 * settles before it returns.
 *
 * The card is marked saved and KEEPS its draft — the overlay sets it before
 * saving — so ← shows the edit: the row itself only changes at the next dry
 * run. In Unknowns a staged sibling this save recategorised leaves the run
 * (D5a's "fixed via similar"); otherwise it shows which card changed it.
 */
function saveStaged(card: ReviewCard, ops: readonly WireEditOp[], similar: readonly SimilarEntry[]): EditResult {
    const target = run;
    if (target === null || target.sink === null) return {ok: false, failure: {kind: "unknown", message: "There is no import to save into."}};
    const applied = similar.filter((entry) => entry.ops.length > 0);
    const siblings = applied.filter((entry): entry is {txn: StagedTxn; ops: readonly WireEditOp[]} => isStaged(entry.txn));
    const journalEdits = applied.filter((entry) => !isStaged(entry.txn));
    if (journalEdits.some((entry) => entry.txn.fingerprint === null)) {
        return {ok: false, failure: {kind: "unavailable", message: "Editing is not enabled on this server."}};
    }
    const result = target.sink.stage({
        overrides: [{item: card.item as StagedTxn, ops}, ...siblings.map((entry) => ({item: entry.txn, ops: entry.ops}))],
        journalEdits: journalEdits.map((entry) => ({index: entry.txn.txn.index, fingerprint: entry.txn.fingerprint as string, ops: [...entry.ops]})),
    });
    if (!result.ok) {
        failure = {index: indexOf(card), failure: result.failure};
        return result;
    }
    failure = null;
    const changed = new Map(siblings.map((entry) => [entry.txn.txn.index, entry.ops]));
    const self = indexOf(card);
    let removed = 0;
    const next: ReviewCard[] = [];
    for (const held of cards) {
        const siblingOps = changed.get(indexOf(held));
        if (indexOf(held) === self) next.push({...held, status: "saved"});
        else if (siblingOps === undefined) next.push(held);
        else if (target.kind === "unknowns" && categorises(siblingOps) && (held.status === "open" || held.status === "skipped")) removed += 1;
        else next.push({...held, updatedFrom: self});
    }
    fixed += removed;
    replaceCards(next);
    review.go(cursor + 1);
    return {ok: true};
}

/** What a single run's write did: the transaction's index afterwards (null once deleted), or why it failed. */
export type SingleOutcome = {ok: true; index: number | null} | {ok: false; failure: EditFailure};

function notSaved(message: string, kind: EditFailure["kind"] = "unknown"): SingleOutcome {
    return {ok: false, failure: {kind, message}};
}

function singleRun(serverUrl: string, mode: SingleMode, today: string, generation: string): Run {
    return {
        serverUrl,
        source: "journal",
        kind: mode,
        single: mode,
        sink: null,
        today,
        limit: undefined,
        maxDays: undefined,
        generation,
        total: 1,
        notEditable: 0,
        fetched: 1,
    };
}

/** Mark `card` saving while `write` runs, then saved, or open again with the failure (a conflict is the page's banner, not the card's). */
async function settleSingle(card: ReviewCard, write: () => Promise<SingleOutcome>): Promise<SingleOutcome> {
    const started = epoch;
    const self = indexOf(card);
    const mark = (status: CardStatus): void => {
        cards = cards.map((held) => (indexOf(held) === self ? {...held, status} : held));
    };
    failure = null;
    mark("saving");
    saving = 1;
    const result = await write();
    if (started !== epoch) return result;
    saving = 0;
    mark(result.ok ? "saved" : "open");
    if (!result.ok && result.failure.kind !== "conflict") failure = {index: self, failure: result.failure};
    return result;
}

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
    suggestion += 1;
}
