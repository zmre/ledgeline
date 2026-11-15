// The bank sync store over a stubbed `fetch`: the real `LedgelineApi`, the
// real decoders, literal wire JSON. What is pinned here is what the header and
// the Accounts tab lean on — an engine without sync is "unsupported" and never
// an error, a running sync polls fast, and a 422 refusal arrives as the
// engine's own problem.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {conAuthProblem, emptyStatus, fixtureRun, fixtureStatus, revokedProblem, tokenUsedProblem} from "./fixtures";
import {POLL_RUNNING_MS, syncStore} from "./syncStore.svelte";
import type {WireSyncStatus} from "./types";

const json = (body: unknown, status = 200): Response => new Response(JSON.stringify(body), {status, headers: {"Content-Type": "application/json"}});

interface Call {
    method: string;
    url: string;
    body: unknown;
}

/** A fetch that answers `/api/sync/status` with whatever `current` holds, and records every call. */
function engine(handlers: Record<string, (body: unknown) => Response> = {}): {calls: Call[]; setStatus: (s: WireSyncStatus | null) => void} {
    let current: WireSyncStatus | null = fixtureStatus();
    const calls: Call[] = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
        calls.push({method, url, body});
        const path = url.slice(FAKE_ENGINE.length);
        const handler = handlers[`${method} ${path}`];
        if (handler !== undefined) return Promise.resolve(handler(body));
        if (path === "/api/sync/status") return Promise.resolve(current === null ? new Response("not found", {status: 404}) : json(current));
        return Promise.resolve(new Response("not found", {status: 404}));
    });
    return {calls, setStatus: (s) => (current = s)};
}

let stop: (() => void) | null = null;

beforeEach(() => {
    syncStore.reset();
});

afterEach(() => {
    stop?.();
    stop = null;
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

describe("UNIT syncStore", () => {
    it("re-reads a shown manual-downloads list after linking, which leaves the linked account out", async () => {
        const manualCalls = (calls: Call[]) => calls.filter((c) => c.url.endsWith("/api/sync/manual")).length;
        const {calls} = engine({
            "GET /api/sync/manual": () => json({accounts: []}),
            "POST /api/sync/link": () => json(fixtureRun()),
        });
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status).not.toBeNull());
        await syncStore.loadManual();
        expect(manualCalls(calls)).toBe(1);
        await syncStore.link([]);
        expect(manualCalls(calls)).toBe(2);
    });

    it("reads the status and derives the dot and the problems", async () => {
        engine();
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status).not.toBeNull());
        expect(syncStore.unsupported).toBe(false);
        expect(syncStore.dot).toBe("attention");
        expect(syncStore.problems.map((p) => p.problem.code)).toEqual(["held", "con-auth"]);
        expect(syncStore.maxSeverity).toBe("warning");
    });

    it("treats an engine without sync as unsupported, and hides the dot", async () => {
        const {calls, setStatus} = engine();
        setStatus(null);
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.unsupported).toBe(true));
        expect(syncStore.status).toBeNull();
        expect(syncStore.pollError).toBeNull();
        expect(syncStore.dot).toBe("hidden");
        expect(syncStore.problems).toEqual([]);
        await syncStore.refresh();
        expect(calls).toHaveLength(1);
    });

    it("hides the dot with nothing linked, and goes red for a broken connection", async () => {
        const {setStatus} = engine();
        setStatus(emptyStatus());
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status).not.toBeNull());
        expect(syncStore.dot).toBe("hidden");

        const broken = fixtureStatus();
        broken.connections[0] = {...broken.connections[0], state: "broken", problems: [revokedProblem()]};
        setStatus(broken);
        await syncStore.refresh();
        expect(syncStore.dot).toBe("broken");
    });

    it("polls every 2 s while a run is going, and stops when it ends", async () => {
        const {calls, setStatus} = engine();
        setStatus({...fixtureStatus(), running: true});
        vi.useFakeTimers();
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status?.running).toBe(true));
        expect(syncStore.dot).toBe("syncing");
        const before = calls.length;
        await vi.advanceTimersByTimeAsync(POLL_RUNNING_MS);
        expect(calls.length).toBe(before + 1);
        setStatus(fixtureStatus());
        await vi.advanceTimersByTimeAsync(POLL_RUNNING_MS);
        expect(syncStore.running).toBe(false);
        const settled = calls.length;
        await vi.advanceTimersByTimeAsync(POLL_RUNNING_MS * 5);
        expect(calls.length).toBe(settled);
    });

    it("keeps the last status when a poll fails, and says why", async () => {
        engine();
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status).not.toBeNull());
        vi.stubGlobal("fetch", () => Promise.reject(new TypeError("offline")));
        await syncStore.refresh();
        expect(syncStore.status).not.toBeNull();
        expect(syncStore.pollError).toMatch(/Cannot reach/);
    });

    it("runs Sync now for one account and keeps the run for the toast", async () => {
        const {calls} = engine({"POST /api/sync/now": () => json(fixtureRun())});
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status).not.toBeNull());
        const outcome = await syncStore.syncNow({accountKey: "a1b2c3d4e5"});
        expect(outcome.ok).toBe(true);
        expect(syncStore.lastRun?.accounts).toHaveLength(3);
        expect(calls.find((c) => c.method === "POST")?.body).toEqual({accountKey: "a1b2c3d4e5"});
        expect(syncStore.syncing).toBe(false);
        syncStore.dismissRun();
        expect(syncStore.lastRun).toBeNull();
    });

    it("hands back the engine's problem for a refused claim", async () => {
        engine({"POST /api/sync/connect": () => json({problem: tokenUsedProblem()}, 422)});
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status).not.toBeNull());
        const outcome = await syncStore.connect("dG9rZW4=");
        expect(outcome.ok).toBe(false);
        expect(!outcome.ok && outcome.failure.problem?.code).toBe("token-used");
        expect(!outcome.ok && outcome.failure.message).toMatch(/^This token was already used/);
    });

    it("takes the status a write answers with", async () => {
        const after = fixtureStatus();
        after.accounts[1].problems = [];
        after.accounts[0].problems = [conAuthProblem()];
        const {calls} = engine({"PATCH /api/sync/accounts/a1b2c3d4e5": () => json(after)});
        stop = syncStore.start(FAKE_ENGINE);
        await vi.waitFor(() => expect(syncStore.status).not.toBeNull());
        const outcome = await syncStore.patchAccount("a1b2c3d4e5", {balanceSign: -1});
        expect(outcome.ok).toBe(true);
        expect(calls.at(-1)?.body).toEqual({balanceSign: -1});
        expect(syncStore.problems.map((p) => p.where)).toEqual(["First Tech · Checking ••1234"]);
    });

    it("hands an intent to the Accounts tab once", () => {
        syncStore.request({type: "connect"});
        expect(syncStore.takeIntent()).toEqual({type: "connect"});
        expect(syncStore.takeIntent()).toBeNull();
    });
});
