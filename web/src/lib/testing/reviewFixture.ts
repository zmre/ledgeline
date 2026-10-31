// The transaction-review wire (plan 32), built from domain transactions the way
// `crates/ledgeline-server/src/review_api.rs` serves it — so a decoder or store
// test feeds the REAL code literal engine bytes rather than a mock.

import {vi} from "vitest";
import type {Transaction} from "#lib/domain/types.js";
import type {QueueKind} from "#lib/review/types.js";
import {FAKE_JOURNAL_INFO} from "./fakeEngine";
import {wireRow} from "./journalViewFixture";

export interface Marks {
    unknown?: boolean;
    reviewed?: boolean;
    fingerprint?: string | null;
}

/** A review transaction: the journal row, its markers, and a fingerprint (`fp-<index>-<n>` by default). */
export function reviewTxnWire(txn: Transaction, marks: Marks = {}): Record<string, unknown> {
    return {
        ...wireRow(txn),
        unknown: marks.unknown ?? true,
        reviewed: marks.reviewed ?? false,
        fingerprint: marks.fingerprint === undefined ? `fp-${txn.index}-0` : marks.fingerprint,
    };
}

/** A queued card: a review transaction plus its key and suggestions. */
export function queueItemWire(txn: Transaction, marks: Marks & {key?: string | null; suggestions?: unknown[]} = {}): Record<string, unknown> {
    return {...reviewTxnWire(txn, marks), key: marks.key === undefined ? "payee" : marks.key, suggestions: marks.suggestions ?? []};
}

/** A CSV dry run's staged transaction (plan 32 D23): a queue item plus its 0-based ordinal. `txn.index` should be `ordinal + 1`. */
export function stagedTxnWire(
    txn: Transaction,
    marks: Marks & {ordinal?: number; key?: string | null; suggestions?: unknown[]; overridden?: boolean} = {}
): Record<string, unknown> {
    return {
        ...queueItemWire(txn, {...marks, fingerprint: marks.fingerprint === undefined ? `gen-${txn.index}` : marks.fingerprint}),
        ordinal: marks.ordinal ?? txn.index - 1,
        overridden: marks.overridden ?? false,
    };
}

/** A QuickBooks preview's staged transaction (D30): keyed by Trans #, with its classification. */
export function qbStagedTxnWire(txn: Transaction, id: string, marks: Marks & {key?: string | null; classification?: string} = {}): Record<string, unknown> {
    return {
        ...queueItemWire(txn, {...marks, fingerprint: marks.fingerprint === undefined ? `qb-${id}` : marks.fingerprint}),
        id,
        classification: marks.classification ?? "new",
        overridden: false,
    };
}

/** One `/api/review/queue` page over `items` (already wire-shaped). */
export function queuePageWire(
    kind: QueueKind,
    items: Record<string, unknown>[],
    overrides: {generation?: string; total?: number; offset?: number; notEditable?: number} = {}
): Record<string, unknown> {
    return {
        generation: overrides.generation ?? "g1",
        kind,
        total: overrides.total ?? items.length,
        offset: overrides.offset ?? 0,
        notEditable: overrides.notEditable ?? 0,
        items,
    };
}

/** A `/api/review/similar/{index}` body. */
export function similarWire(
    index: number,
    candidates: Record<string, unknown>[],
    overrides: {generation?: string; inQueue?: number[]; precedent?: unknown; key?: string | null} = {}
): Record<string, unknown> {
    return {
        generation: overrides.generation ?? "g1",
        index,
        key: overrides.key === undefined ? "payee" : overrides.key,
        total: candidates.length,
        candidates,
        inQueue: overrides.inQueue ?? [],
        precedent: overrides.precedent ?? null,
    };
}

/** A minimal two-posting transaction `index` on `date`, to `account`. */
export function txnFixture(index: number, description: string, account = "expenses:unknown", date = "2026-10-01"): Transaction {
    return {
        index,
        date,
        status: "unmarked",
        description,
        code: "",
        comment: "",
        tags: [],
        postings: [
            {
                account,
                amounts: [{commodity: "$", qty: {m: 500n, p: 2}, style: {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null}}],
                status: "unmarked",
                comment: "",
                tags: [],
            },
            {account: "liabilities:visa", amounts: [], status: "unmarked", comment: "", tags: []},
        ],
    };
}

/** One request the fake review engine saw. */
export interface SeenRequest {
    method: string;
    url: URL;
    body: unknown;
}

/** What the fake review engine answers; a handler may return a `Response` (an error status) or a promise of one. */
export interface ReviewHandlers {
    queue: (url: URL) => unknown;
    similar?: (index: number, url: URL) => unknown;
    /** `/api/review/similar-key?key=` (a staged card's journal siblings). Defaults to an empty set. */
    similarKey?: (key: string, url: URL) => unknown;
    batch?: (body: {edits: {index: number; fingerprint: string; ops: unknown[]}[]}) => unknown;
}

/**
 * Stub `fetch` as an engine serving the review routes through `handlers`, plus
 * what a journal refresh asks for. Call after `connectFakeEngine()` (which
 * points `settings` at the fake engine), and pair with `vi.unstubAllGlobals()`.
 */
export function serveReview(handlers: ReviewHandlers): SeenRequest[] {
    const seen: SeenRequest[] = [];
    const answer = async (value: unknown): Promise<Response> => {
        const resolved = await value;
        return resolved instanceof Response ? resolved : new Response(JSON.stringify(resolved), {status: 200});
    };
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
        seen.push({method, url, body});
        const path = url.pathname;
        if (path.endsWith("/version")) return answer("1.52");
        if (path.endsWith("/api/journal")) return answer(FAKE_JOURNAL_INFO);
        if (path.endsWith("/api/diagnostics")) return answer({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/review/queue")) return answer(handlers.queue(url));
        if (path.endsWith("/api/review/similar-key")) {
            const key = url.searchParams.get("key") ?? "";
            return answer(handlers.similarKey?.(key, url) ?? {...similarWire(0, []), key});
        }
        const similar = /\/api\/review\/similar\/(\d+)$/.exec(path);
        if (similar !== null) {
            const index = Number(similar[1]);
            return answer(handlers.similar?.(index, url) ?? similarWire(index, []));
        }
        if (path.endsWith("/api/transactions/batch") && handlers.batch !== undefined) {
            return answer(handlers.batch(body as Parameters<NonNullable<ReviewHandlers["batch"]>>[0]));
        }
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    return seen;
}

/** A promise settled from outside, to hold a response until a test lets it go. */
export function deferred<T>(): {promise: Promise<T>; resolve: (value: T) => void} {
    let resolve: (value: T) => void = () => {};
    const promise = new Promise<T>((settle) => (resolve = settle));
    return {promise, resolve};
}
