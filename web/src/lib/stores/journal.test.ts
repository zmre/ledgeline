// The journal METADATA store: which ledger is on screen, its account tables,
// and the generation every journal-derived view keys on. One `/api/journal`
// request per round (plan 26 B5 — the journal itself is no longer downloaded).

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {journalInfoBody} from "#lib/testing/fakeEngine.js";
import {journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

/** A promise with its settle functions exposed. */
function deferred<T>(): {promise: Promise<T>; resolve: (value: T) => void} {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((res) => {
        resolve = res;
    });
    return {promise, resolve};
}

describe("INTEGRATION journal metadata rounds", () => {
    /** What each `/api/journal` round answers (the last repeats); `null` is a 404 — not the engine. */
    let answers: (Record<string, unknown> | null)[] = [];
    let hits = 0;
    /** Optionally hold a round's answer back, by round number. */
    let gate: ((round: number) => Promise<Response> | null) | null = null;

    beforeEach(async () => {
        answers = [
            journalInfoBody({
                title: "Acme Books",
                file: "2026.journal",
                generation: "g-1",
                txnCount: 3,
                accounts: {"expenses:food": {type: "expense", typeDeclared: false, term: null, termGuessed: false}},
            }),
        ];
        hits = 0;
        gate = null;
        vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
            const url = String(input);
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (!url.endsWith("/api/journal")) return Promise.resolve(new Response("unexpected", {status: 500}));
            const round = hits;
            hits += 1;
            const held = gate?.(round) ?? null;
            if (held !== null) return held;
            const body = answers[Math.min(round, answers.length - 1)];
            return Promise.resolve(body === null ? new Response("no such route", {status: 404}) : json(body));
        });
        await settings.setServerUrl("http://engine-a");
        await journal.refresh({force: true});
    });

    afterEach(async () => {
        gate = null;
        await journal.refresh({force: true}).catch(() => undefined);
        vi.unstubAllGlobals();
    });

    it("exposes the engine's title, file, generation and account tables after a round", () => {
        expect(journal.title).toBe("Acme Books");
        expect(journal.file).toBe("2026.journal");
        expect(journal.generation).toBe("g-1");
        expect(journal.txnCount).toBe(3);
        expect(journal.accountNames).toEqual(["expenses:food"]);
        expect(journal.accountClasses.get("expenses:food")?.type).toBe("expense");
        expect(journal.status).toBe("ready");
    });

    it("sorts account names, even the integer-like ones a JS object puts first", async () => {
        const accounts = Object.fromEntries(["zeta", "1000", "alpha"].map((name) => [name, {type: null, typeDeclared: false, term: null, termGuessed: false}]));
        answers = [journalInfoBody({generation: "g-2", accounts})];
        hits = 0;
        await journal.refresh({force: true});
        expect(journal.accountNames).toEqual(["1000", "alpha", "zeta"]);
    });

    it("leaves the account tables alone when the generation is unchanged", async () => {
        const before = journal.accountClasses;
        await journal.refresh({force: true});
        expect(journal.accountClasses).toBe(before);
    });

    it("relabels when the SAME url starts serving a different journal (desktop File→Open)", async () => {
        answers = [journalInfoBody({title: "Ledger Two", generation: "g-9"})];
        hits = 0;
        await journal.refresh({force: true});
        expect(journal.title).toBe("Ledger Two");
        expect(journal.generation).toBe("g-9");
    });

    it("fails the round when the server has no /api/journal — the SPA requires the engine", async () => {
        answers = [null];
        hits = 0;
        await journal.refresh({force: true});
        expect(journal.status).toBe("error");
        expect(journal.error).not.toBeNull();
        // Like any failed round, the last good metadata stays on screen.
        expect(journal.title).toBe("Acme Books");
        expect(journal.generation).toBe("g-1");
    });

    it("does not let a superseded round relabel the screen", async () => {
        const late = deferred<Response>();
        answers = [journalInfoBody({title: "Old Books", generation: "g-old"}), journalInfoBody({title: "New Books", generation: "g-new"})];
        hits = 0;
        // Round 0 answers late, deliberately ignoring the abort — a fetch that
        // had already completed when the abort landed behaves exactly so.
        gate = (round) => (round === 0 ? late.promise : null);
        const superseded = journal.refresh({force: true});
        const winner = journal.refresh({force: true});
        await winner;
        expect(journal.title).toBe("New Books");
        late.resolve(json(answers[0]));
        await superseded;
        expect(journal.title).toBe("New Books");
        expect(journal.generation).toBe("g-new");
    });
});
