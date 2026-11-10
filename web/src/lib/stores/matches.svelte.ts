// Find Matches (plan 35, Phase 3): the proposals behind the journal's badge,
// the overlay's run, and every match write.
//
// The proposals are read once per (server, generation, window, lookback):
// the journal toolbar keeps them in step with the journal for its badge, and
// the overlay shows the same list, so opening it costs no request. Every
// write goes through `editing.write` — the journal is re-read on success and
// a 409 raises the page's conflict banner, as for any other edit — and then
// the proposals are read again at the new generation. Writes are not
// optimistic: the card waits for its write, then the re-read moves it on.
//
// Skip (D9) lasts for the run: a skipped pair is remembered by its halves'
// fingerprints (indices move on every write; untouched text does not) until
// the overlay closes.
//
// An import's result screen opens a SCOPED run (D9 "Import"): `since`
// (`JOURNAL_ID:COUNT`, the transactions the commit appended) is part of the
// read's key while the run is open, so the overlay shows only the pairs with
// a new half; closing it drops the scope and reads the whole list again for
// the badge.
//
// Amendment A2: `openFor(index)` opens the same overlay FOCUSED on one
// transaction, read from `/api/matches/for/{index}` with its own window
// (the ▾ setting to start with, widenable in the view). Every write re-reads
// it too.

import {decodeLinkAll, decodeMatches, decodeMatchWrite, decodeTransactionMatches} from "#lib/api/matchDecode.js";
import {LedgelineApi, type MatchGroupBody, type MatchPairBody, type WireMatchHalf} from "#lib/api/native.js";
import {decodeReviewItem} from "#lib/api/reviewDecode.js";
import type {EditFailure} from "#lib/api/editFailure.js";
import type {TransferRow} from "#lib/journalView/types.js";
import {MATCH_TAG, NOMATCH_TAG, pairKey, postingsTagged, wireHalf} from "#lib/matching/model.js";
import type {LinkAllResult, MatchesPage, MatchHalf, MatchLink, MatchProposal, MatchWriteResult, TransactionMatches} from "#lib/matching/types.js";
import {editing} from "./editing.svelte";
import {localToday} from "./filters.svelte";
import {journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

/** What a match write came to: done, or why not. A conflict also raised the page's banner. */
export type MatchOutcome<T = MatchWriteResult> = {ok: true; value: T} | {ok: false; failure: EditFailure};

let page = $state.raw<MatchesPage | null>(null);
let pageKey: string | null = null;
let error = $state<string | null>(null);
let loading = $state(false);
let inflight: {key: string; promise: Promise<void>} | null = null;

let open = $state(false);
/** The open run's `since` scope (an import's new transactions), or null for the whole journal. */
let scope = $state<string | null>(null);
/** Pair keys (`pairKey`) skipped this run. */
let skipped = $state.raw<readonly string[]>([]);
let busy = $state(false);

/** The focused view (A2): the transaction asked about, and what the engine offered it. */
export interface MatchFocus {
    index: number;
    /** Null until the first answer. */
    data: TransactionMatches | null;
    error: string | null;
}
let focus = $state.raw<MatchFocus | null>(null);
/** The focused view's window (days either side for `nearby`), 1–60. */
let focusWindow = $state(10);
/** Bumped per focused read, so a stale answer is dropped. */
let focusSeq = 0;
const FOCUS_WINDOW_RANGE = [1, 60] as const;

function keyOf(serverUrl: string, generation: string): string {
    return `${serverUrl}|${generation}|${settings.matchWindow}|${settings.matchLookback}|${scope ?? ""}`;
}

/** `GET /api/matches` with the per-browser settings (D8), scoped by `since` when given. */
async function fetchPage(serverUrl: string, since: string | null): Promise<MatchesPage> {
    return decodeMatches(
        await new LedgelineApi(serverUrl).matches({
            today: localToday(),
            window: settings.matchWindow,
            lookback: settings.matchLookback,
            since: since ?? undefined,
        })
    );
}

/** Forget the list read: the next `sync` reads afresh (the scope changed). */
function dropPage(): void {
    page = null;
    pageKey = null;
    inflight = null;
    loading = false;
}

async function load(serverUrl: string, key: string): Promise<void> {
    loading = true;
    try {
        const next = await fetchPage(serverUrl, scope);
        if (inflight?.key !== key) return;
        page = next;
        pageKey = key;
        error = null;
    } catch (cause) {
        if (inflight?.key !== key) return;
        // Keep the last list: the badge is an aid, and a blip must not empty it.
        error = cause instanceof Error ? cause.message : String(cause);
        pageKey = key;
    } finally {
        if (inflight?.key === key) {
            inflight = null;
            loading = false;
        }
    }
}

/** Re-read the proposals at the journal's current generation (after a write). */
function reread(): Promise<void> {
    const serverUrl = settings.serverUrl;
    const generation = journal.generation;
    if (serverUrl === null || generation === null) return Promise.resolve();
    return matches.sync(serverUrl, generation);
}

/** `GET /api/matches/for/{index}` with the focused view's window. */
async function fetchFor(api: LedgelineApi, index: number, window: number): Promise<TransactionMatches> {
    return decodeTransactionMatches(await api.matchesFor(index, {today: localToday(), window}));
}

/** (Re-)read the focused view. A newer read, or closing it, drops this one's answer. */
async function loadFocus(): Promise<void> {
    const serverUrl = settings.serverUrl;
    const asked = focus;
    if (serverUrl === null || asked === null) return;
    focusSeq += 1;
    const seq = focusSeq;
    try {
        const data = await fetchFor(new LedgelineApi(serverUrl), asked.index, focusWindow);
        if (seq !== focusSeq || focus?.index !== asked.index) return;
        focus = {index: asked.index, data, error: null};
    } catch (cause) {
        if (seq !== focusSeq || focus === null || focus.index !== asked.index) return;
        focus = {index: asked.index, data: focus.data, error: cause instanceof Error ? cause.message : String(cause)};
    }
}

/** Run a match write through the journal's write path, then re-read the proposals (and the focused view). */
async function write<T>(action: (api: LedgelineApi) => Promise<T>): Promise<MatchOutcome<T>> {
    busy = true;
    try {
        const result = await editing.write(action);
        await Promise.all([reread(), focus === null ? Promise.resolve() : loadFocus()]);
        return result.ok ? {ok: true, value: result.value} : result;
    } finally {
        busy = false;
    }
}

/**
 * The fingerprint a journal row's transaction is at now, and the written
 * position of its posting carrying `name:value`. Journal rows carry no
 * fingerprint, so an unlink or a forget from the table asks the review item
 * route for it first, inside the same write.
 */
async function taggedHalf(api: LedgelineApi, index: number, name: string, value: string): Promise<WireMatchHalf> {
    const {item} = decodeReviewItem(await api.reviewItem(index, localToday()));
    const posting = postingsTagged(item.txn, name, value)[0];
    if (posting === undefined || item.fingerprint === null) throw new Error(`Transaction #${index} no longer carries ${name}:${value}.`);
    return {index, posting, fingerprint: item.fingerprint};
}

export const matches = {
    /** The proposals for the current journal, or null before the first answer. */
    get page(): MatchesPage | null {
        return page;
    },
    /** Why the last read failed, or null. */
    get error(): string | null {
        return error;
    },
    get loading(): boolean {
        return loading;
    },
    /** How many pairs are proposed: the toolbar's badge. */
    get count(): number {
        return page?.proposals.length ?? 0;
    },
    /** Whether a clearing account is declared (D2); null until the engine has said. */
    get clearing(): boolean | null {
        return page?.clearing ?? null;
    },
    /** How many more pairs there would be with no lookback (A1). */
    get older(): number {
        return page?.older ?? 0;
    },
    /** Show all (A1): the lookback to All time, and read again. */
    showAll(): Promise<void> {
        settings.matchLookback = 0;
        return reread();
    },

    /** Read the proposals unless they are already for this (server, generation, settings). Resolves when they are. */
    sync(serverUrl: string, generation: string): Promise<void> {
        const key = keyOf(serverUrl, generation);
        if (inflight?.key === key) return inflight.promise;
        if (pageKey === key) return Promise.resolve();
        const promise = load(serverUrl, key);
        inflight = {key, promise};
        return promise;
    },

    // -- the overlay's run --------------------------------------------------

    /** Whether the overlay is open. */
    get open(): boolean {
        return open;
    },
    /** The open run's `since` scope, or null for the whole journal. */
    get scope(): string | null {
        return scope;
    },
    /**
     * Open the overlay. `since` (`JOURNAL_ID:COUNT`) scopes the run to pairs
     * with a half among an import's new transactions; the list is read again
     * for it (the badge's list is for the whole journal).
     */
    openRun(since: string | null = null): void {
        skipped = [];
        focus = null;
        if (since !== scope) {
            scope = since;
            dropPage();
            void reread();
        }
        open = true;
    },
    close(): void {
        open = false;
        skipped = [];
        focus = null;
        focusSeq += 1;
        if (scope !== null) {
            scope = null;
            dropPage();
            void reread();
        }
    },
    /**
     * How many pairs a scoped run would show, read on its own: the import
     * result's Find Matches is offered only when there are some. It leaves the
     * badge's list alone.
     */
    async scopedCount(serverUrl: string, since: string): Promise<number> {
        return (await fetchPage(serverUrl, since)).proposals.length;
    },
    // -- the focused view (A2) ----------------------------------------------

    /**
     * Open the overlay focused on transaction `index`: its unlinked clearing
     * postings' exact partners, combinations and nearby postings, and its
     * links. The window starts at the ▾ setting.
     */
    openFor(index: number): void {
        if (scope !== null) {
            scope = null;
            dropPage();
            void reread();
        }
        skipped = [];
        focusWindow = settings.matchWindow;
        focus = {index, data: null, error: null};
        open = true;
        void loadFocus();
    },
    /** The focused view, or null for the whole-journal run. */
    get focus(): MatchFocus | null {
        return focus;
    },
    get focusWindow(): number {
        return focusWindow;
    },
    /** Widen (or narrow) the focused view's window, 1–60 days, and read it again. */
    setFocusWindow(days: number): Promise<void> {
        const [min, max] = FOCUS_WINDOW_RANGE;
        if (!Number.isFinite(days)) return Promise.resolve();
        const next = Math.min(max, Math.max(min, Math.round(days)));
        if (next === focusWindow) return Promise.resolve();
        focusWindow = next;
        return loadFocus();
    },
    /** Read the focused view again (after a failure). */
    reloadFocus(): Promise<void> {
        return loadFocus();
    },

    /** A write is in flight. */
    get busy(): boolean {
        return busy;
    },
    /** The proposals still to show this run: everything not skipped, earliest first. */
    get queue(): MatchProposal[] {
        return (page?.proposals ?? []).filter((proposal) => !skipped.includes(pairKey(proposal.a, proposal.b)));
    },
    /** Skip (D9): this run only. */
    skip(proposal: MatchProposal): void {
        skipped = [...skipped, pairKey(proposal.a, proposal.b)];
    },

    /** ✓ Link (D1): a pair, or a group (A3: a combination, or postings picked by hand), with any description edits in the same write. */
    link(body: MatchPairBody | MatchGroupBody): Promise<MatchOutcome> {
        return write(async (api) => decodeMatchWrite(await api.linkMatch(body)));
    },
    /** ✕ Not a match (D12), with any description edits in the same write. */
    reject(body: MatchPairBody): Promise<MatchOutcome> {
        return write(async (api) => decodeMatchWrite(await api.rejectMatch(body)));
    },
    /** Ignore this side (D1): `match:ignore` on one half. */
    ignore(half: WireMatchHalf): Promise<MatchOutcome> {
        return write(async (api) => decodeMatchWrite(await api.ignoreMatchHalf(half)));
    },
    /** Link all clear matches (D9): one request; the engine writes them in batch-sized saves. */
    linkAll(pairs: readonly {a: MatchHalf; b: MatchHalf}[]): Promise<MatchOutcome<LinkAllResult>> {
        const body = {pairs: pairs.map(({a, b}) => ({a: wireHalf(a), b: wireHalf(b)}))};
        return write(async (api) => decodeLinkAll(await api.linkAllMatches(body)));
    },
    /** Unlink a collapsed transfer row (D5): `match:ID` off both halves. */
    unlink(transfer: TransferRow): Promise<MatchOutcome> {
        return write(async (api) => {
            const halves = await Promise.all(transfer.halves.map((half) => taggedHalf(api, half.txn.index, MATCH_TAG, transfer.matchId)));
            return decodeMatchWrite(await api.unlinkMatch({matchId: transfer.matchId, halves}));
        });
    },
    /** Unlink (D5, A3) a link the engine described (`/for`): `match:ID` off every member, each named. */
    unlinkLink(link: MatchLink): Promise<MatchOutcome> {
        return write(async (api) => decodeMatchWrite(await api.unlinkMatch({matchId: link.matchId, halves: link.members.map((half) => wireHalf(half))})));
    },
    /**
     * Unlink from somewhere that holds only the transaction (the editor, the
     * Problems drawer): ask `/for/{index}` for its links, then unlink `matchId`
     * — or, with no id, every BROKEN link it carries — naming every member.
     */
    unlinkFrom(index: number, matchId: string | null): Promise<MatchOutcome> {
        return write(async (api) => {
            const {links} = await fetchFor(api, index, settings.matchWindow);
            const chosen = links.filter((link) => (matchId === null ? link.broken : link.matchId === matchId));
            if (chosen.length === 0) throw new Error(`Transaction #${index} has no ${matchId === null ? "broken link" : `link ${matchId}`} any more.`);
            let last: MatchWriteResult | null = null;
            for (const link of chosen) {
                last = decodeMatchWrite(await api.unlinkMatch({matchId: link.matchId, halves: link.members.map((half) => wireHalf(half))}));
            }
            if (last === null) throw new Error("Nothing was unlinked.");
            return last;
        });
    },
    /** Forget rejection (D12 Undo): `nomatch:VALUE` off every posting carrying it, named from transaction `index`. */
    forget(index: number, value: string): Promise<MatchOutcome> {
        return write(async (api) => {
            const half = await taggedHalf(api, index, NOMATCH_TAG, value);
            return decodeMatchWrite(await api.forgetRejection({value, halves: [half]}));
        });
    },
};

/** Test seam: module-level runes state is shared across a test file. */
export function resetMatches(): void {
    page = null;
    pageKey = null;
    error = null;
    loading = false;
    inflight = null;
    open = false;
    scope = null;
    skipped = [];
    busy = false;
    focus = null;
    focusWindow = 10;
    focusSeq += 1;
}
