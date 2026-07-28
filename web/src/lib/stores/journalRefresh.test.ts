// The journal store's refresh LIFECYCLE: who gets to join an in-flight round,
// who supersedes one, and what happens when a server accepts a connection and
// then says nothing (FE-5d / FE-5e / FE-5f).
//
// Driven through a stubbed `fetch`, because that is the only place these bugs
// are visible: each is about a promise being handed to the wrong caller, not
// about any value the store computes.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {REQUEST_TIMEOUT_MS} from "$lib/api/client";
import {journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

/** A promise with its settle functions exposed. */
function deferred<T>(): {promise: Promise<T>; resolve: (value: T) => void} {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((res) => {
        resolve = res;
    });
    return {promise, resolve};
}

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

/** One wire transaction, enough for normalizeTransactions. */
const wireTxn = (index: number): unknown => ({
    tindex: index,
    tdate: `2026-01-0${index}`,
    tstatus: "Cleared",
    tdescription: `txn ${index}`,
    tpostings: [{paccount: "expenses:food", pamount: []}],
});

/** A whole journal of `count` transactions, per route. */
const payloadFor = (url: string, count: number): unknown => {
    if (url.endsWith("/version")) return "1.52";
    if (url.endsWith("/transactions")) return Array.from({length: count}, (_, i) => wireTxn(i + 1));
    if (url.endsWith("/accountnames")) return ["expenses:food"];
    return [];
};

/** A response that never arrives but DOES reject when its request is aborted. */
const hang = (signal: AbortSignal | undefined): Promise<Response> =>
    new Promise((_, reject) => {
        const fail = (): void => reject(new DOMException("aborted", "AbortError"));
        if (signal === undefined) return;
        if (signal.aborted) fail();
        else signal.addEventListener("abort", fail, {once: true});
    });

type Handler = (url: string, signal: AbortSignal | undefined) => Promise<Response>;

let requested: string[] = [];
/** Answers everything immediately with a one-transaction journal. */
const ok =
    (count = 1): Handler =>
    (url) =>
        Promise.resolve(json(payloadFor(url, count)));
let handler: Handler = ok();

/** How many times /transactions has been requested so far. */
const txnHits = (): number => requested.filter((url) => url.endsWith("/transactions")).length;

/**
 * Answer /transactions differently on each attempt (index 0 = first round),
 * everything else immediately. `nth` is captured BEFORE the request is counted.
 */
function perAttempt(...answers: ((signal: AbortSignal | undefined) => Promise<Response>)[]): Handler {
    let attempt = -1;
    return (url, signal) => {
        if (!url.endsWith("/transactions")) return Promise.resolve(json(payloadFor(url, 0)));
        attempt += 1;
        const answer = answers[Math.min(attempt, answers.length - 1)];
        return answer(signal);
    };
}

beforeEach(async () => {
    handler = ok();
    requested = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        requested.push(url);
        return handler(url, init?.signal ?? undefined);
    });
    await settings.setServerUrl("http://engine-a");
    // Leave the store with no round in flight and a known good state, so each
    // test starts from the same place regardless of what ran before it.
    await journal.refresh({force: true});
    requested = [];
});

afterEach(async () => {
    vi.useRealTimers();
    handler = ok();
    // Drain: abort whatever a test left running so it cannot bleed into the next.
    await journal.refresh({force: true}).catch(() => undefined);
    vi.unstubAllGlobals();
});

describe("INTEGRATION journal.refresh in-flight sharing", () => {
    it("joins an in-flight round when nothing has changed (the dedup that makes polling cheap)", async () => {
        const gate = deferred<Response>();
        handler = perAttempt(() => gate.promise);

        const first = journal.refresh();
        const second = journal.refresh();
        expect(second).toBe(first);
        expect(txnHits()).toBe(1);

        gate.resolve(json(payloadFor("/transactions", 1)));
        await Promise.all([first, second]);
    });

    it("a FORCED refresh starts its own round instead of joining (FE-5e)", async () => {
        // `editing.run()` does `await write; await journal.refresh()`. A round
        // already in flight issued its GETs BEFORE the write, so joining it
        // resolved the edit "ok" against pre-edit data and then banked the
        // pre-edit fingerprint — which suppressed the swap for the real result
        // too. Toggle row A's status then row B's and B's badge sat unchanged
        // for up to 30 seconds.
        const gate = deferred<Response>();
        handler = perAttempt(
            () => gate.promise,
            () => Promise.resolve(json(payloadFor("/transactions", 2)))
        );

        const stale = journal.refresh();
        expect(txnHits()).toBe(1);

        const forced = journal.refresh({force: true});
        expect(forced).not.toBe(stale);
        expect(txnHits()).toBe(2);

        await forced;
        expect(journal.txns).toHaveLength(2);

        gate.resolve(json(payloadFor("/transactions", 1)));
        await stale;
    });

    it("does not hand a reconnect the OLD server's promise (FE-5d)", async () => {
        handler = (url, signal) =>
            url.startsWith("http://engine-a") && url.endsWith("/transactions") ? hang(signal) : Promise.resolve(json(payloadFor(url, 1)));

        const hung = journal.refresh();
        expect(txnHits()).toBe(1);

        // The user reconnects to a different address while that round is stuck.
        await settings.setServerUrl("http://engine-b");
        const reconnected = journal.refresh();
        expect(reconnected).not.toBe(hung);

        await Promise.all([hung, reconnected]);
        expect(requested).toContain("http://engine-b/transactions");
        expect(journal.status).toBe("ready");
    });

    it("discards a superseded round's answers, including its fingerprint", async () => {
        // The token. A round that answers after being superseded must not write
        // state — least of all `lastFingerprint`, which is what decides whether
        // the NEXT (correct) result is swapped in at all.
        const late = deferred<Response>();
        handler = perAttempt(
            // Deliberately ignores the abort signal: a fetch that had already
            // completed when the abort landed behaves exactly like this.
            () => late.promise,
            () => Promise.resolve(json(payloadFor("/transactions", 3)))
        );

        const superseded = journal.refresh();
        const winner = journal.refresh({force: true});
        await winner;
        expect(journal.txns).toHaveLength(3);

        late.resolve(json([wireTxn(1)]));
        await superseded;
        expect(journal.txns).toHaveLength(3);
    });
});

describe("INTEGRATION journal.refresh deadlines", () => {
    it("fails a request that never answers instead of pinning the store forever (FE-5f)", async () => {
        vi.useFakeTimers();
        handler = (_url, signal) => hang(signal);

        const pending = journal.refresh({force: true});
        await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 1);
        await pending;

        expect(journal.status).toBe("error");
        expect(journal.error).toMatch(/timed out/i);
    });

    it("releases the in-flight slot after a deadline, so the next Retry actually retries", async () => {
        vi.useFakeTimers();
        handler = (_url, signal) => hang(signal);
        const pending = journal.refresh({force: true});
        await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 1);
        await pending;

        // Before the deadline existed, the poller, the toolbar refresh button
        // and every error toast's Retry all got this same dead promise back.
        requested = [];
        handler = ok();
        await journal.refresh();
        expect(txnHits()).toBe(1);
        expect(journal.status).toBe("ready");
    });
});
