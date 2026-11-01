// The review run AFTER ACTIONS: saves are optimistic and serial (D14), their
// results reshape the run at once (D5a), a failure rewinds to the failed card
// with its edits intact, and a generation change re-validates the queue.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {deferred, queueItemWire, queuePageWire, reviewTxnWire, serveReview, txnFixture} from "#lib/testing/reviewFixture.js";
import type {ReviewForm} from "#lib/review/types.js";
import {resetReview, review} from "./review.svelte";

const TAG = {op: "setTxnTag", name: "reviewed", value: "2026-10-30"} as const;
const COFFEE = {op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"} as const;

const txn = (index: number) => txnFixture(index, `SQ *BLUE BOTTLE ${index} OAKLAND`);
const page = (kind: "unknowns" | "recents", indices: number[], generation = "g1") =>
    queuePageWire(
        kind,
        indices.map((index) => queueItemWire(txn(index))),
        {generation}
    );
/** The batch's 200 for `edits`: each edited transaction categorised, with a new fingerprint. */
const landed = (edits: {index: number}[], generation = "g2", marks: {unknown?: boolean; reviewed?: boolean} = {}) => ({
    generation,
    edited: edits.map(({index}) =>
        reviewTxnWire(txnFixture(index, `SQ *BLUE BOTTLE ${index} OAKLAND`, "expenses:food:coffee"), {
            unknown: marks.unknown ?? false,
            reviewed: marks.reviewed ?? false,
            fingerprint: `fp-${index}-1`,
        })
    ),
});

beforeEach(async () => {
    resetReview();
    await connectFakeEngine();
});
afterEach(() => vi.unstubAllGlobals());

describe("UNIT review saves", () => {
    it("advances at once, sends the card's fingerprint, and marks the card saved when the write lands", async () => {
        const hold = deferred<unknown>();
        const seen = serveReview({queue: () => page("unknowns", [1, 2, 3]), batch: () => hold.promise});
        await review.open("unknowns");
        const result = review.save([COFFEE, TAG]);
        // Optimistic: the next card is up before the engine has answered.
        expect(review.current?.item.txn.index).toBe(2);
        expect(review.saving).toBe(1);
        expect(review.cards[0]?.status).toBe("saving");
        const sent = seen.find((r) => r.url.pathname.endsWith("/batch"))?.body;
        expect(sent).toEqual({edits: [{index: 1, fingerprint: "fp-1-0", ops: [COFFEE, TAG]}]});
        hold.resolve(landed([{index: 1}], "g2", {reviewed: true}));
        expect(await result).toEqual({ok: true});
        expect(review.saving).toBe(0);
        expect(review.cards[0]).toMatchObject({status: "saved", draft: null});
        expect(review.cards[0]?.item.fingerprint).toBe("fp-1-1");
        expect(review.tally.saved).toBe(1);
    });

    it("D5a, Unknowns: queued cards a similar-apply categorised leave the run, and the cursor skips them", async () => {
        const hold = deferred<unknown>();
        const seen = serveReview({queue: () => page("unknowns", [1, 2, 3, 4]), batch: () => hold.promise});
        await review.open("unknowns");
        const similar = [2, 3].map((index) => ({txn: review.cards[index - 1]!.item, ops: [COFFEE]}));
        const result = review.save([COFFEE, TAG], similar);
        // The cards this batch is fixing are skipped by the optimistic advance.
        expect(review.current?.item.txn.index).toBe(4);
        expect(seen.find((r) => r.url.pathname.endsWith("/batch"))?.body).toMatchObject({
            edits: [{index: 1}, {index: 2, fingerprint: "fp-2-0", ops: [COFFEE]}, {index: 3}],
        });
        hold.resolve(landed([{index: 1}, {index: 2}, {index: 3}]));
        await result;
        expect(review.cards.map((card) => card.item.txn.index)).toEqual([1, 4]);
        expect(review.tally).toEqual({saved: 1, skipped: 0, fixed: 2});
        expect(review.current?.item.txn.index).toBe(4);
        expect(review.position).toBe(2);
        expect(review.count).toBe(2);
    });

    it("D5a, Recents: a touched queued card stays, showing the change and which card made it", async () => {
        serveReview({queue: () => page("recents", [1, 2, 3]), batch: (body) => landed(body.edits)});
        await review.open("recents");
        await review.save([COFFEE, TAG], [{txn: review.cards[2]!.item, ops: [COFFEE]}]);
        expect(review.cards.map((card) => card.item.txn.index)).toEqual([1, 2, 3]);
        expect(review.cards[2]).toMatchObject({status: "open", updatedFrom: 1});
        expect(review.cards[2]?.item.txn.postings[0]?.account).toBe("expenses:food:coffee");
        expect(review.cards[2]?.item.fingerprint).toBe("fp-3-1");
        // The key and suggestions the queue served are kept.
        expect(review.cards[2]?.item.key).toBe("payee");
    });

    it("sends saves one at a time, in order", async () => {
        const first = deferred<unknown>();
        const seen = serveReview({
            queue: () => page("unknowns", [1, 2, 3]),
            batch: (body) => (body.edits[0]?.index === 1 ? first.promise : landed(body.edits, "g3")),
        });
        await review.open("unknowns");
        const one = review.save([TAG]);
        const two = review.save([TAG]);
        expect(review.saving).toBe(2);
        await Promise.resolve();
        expect(seen.filter((r) => r.url.pathname.endsWith("/batch"))).toHaveLength(1);
        first.resolve(landed([{index: 1}]));
        await Promise.all([one, two]);
        const batches = seen.filter((r) => r.url.pathname.endsWith("/batch")).map((r) => (r.body as {edits: {index: number}[]}).edits[0]?.index);
        expect(batches).toEqual([1, 2]);
        expect(review.generation).toBe("g3");
    });

    it("a failed save rewinds to its card with its draft and the error, and cancels what was queued behind it", async () => {
        const hold = deferred<unknown>();
        serveReview({
            queue: () => page("unknowns", [1, 2, 3]),
            batch: () => hold.promise,
        });
        await review.open("unknowns");
        const draft: ReviewForm = {date: "2026-10-01", description: "Blue Bottle", comment: "", tags: [], postings: []};
        review.setDraft(draft);
        const one = review.save([TAG]);
        const second: ReviewForm = {...draft, description: "Second"};
        review.setDraft(second);
        const two = review.save([TAG]);
        expect(review.current?.item.txn.index).toBe(3);
        hold.resolve(new Response("transaction #1 changed since it was loaded", {status: 409}));
        expect(await one).toMatchObject({ok: false, failure: {kind: "conflict"}});
        expect(await two).toMatchObject({ok: false});
        expect(review.current?.item.txn.index).toBe(1);
        expect(review.current).toMatchObject({status: "open", draft});
        expect(review.cards[1]).toMatchObject({status: "open", draft: second});
        expect(review.failure).toMatchObject({index: 1, failure: {kind: "conflict", message: "transaction #1 changed since it was loaded"}});
        expect(review.saving).toBe(0);
    });

    it("refuses to save a card the engine served without a fingerprint", async () => {
        serveReview({queue: () => queuePageWire("unknowns", [queueItemWire(txn(1), {fingerprint: null})])});
        await review.open("unknowns");
        expect(await review.save([TAG])).toMatchObject({ok: false, failure: {kind: "unavailable"}});
        expect(review.cursor).toBe(0);
    });

    it("re-validates on a generation change: drops what left, refreshes what changed, appends what is new", async () => {
        let generation = "g1";
        serveReview({
            queue: () =>
                generation === "g1"
                    ? page("unknowns", [1, 2, 3, 4])
                    : queuePageWire("unknowns", [queueItemWire(txn(1)), queueItemWire(txn(3), {fingerprint: "fp-3-9"}), queueItemWire(txn(5))], {
                          generation: "g9",
                      }),
        });
        await review.open("unknowns");
        review.skip();
        review.setDraft({date: "2026-10-01", description: "typing", comment: "", tags: [], postings: []});
        review.go(2);
        review.setDraft({date: "2026-10-01", description: "stale", comment: "", tags: [], postings: []});
        review.go(1);
        generation = "g9";
        review.syncGeneration("g1");
        expect(review.generation).toBe("g1");
        review.syncGeneration("g9");
        await vi.waitFor(() => expect(review.generation).toBe("g9"));
        // 2 is on screen, so it stays; 4 left; 3 changed (draft dropped); 5 is new.
        expect(review.cards.map((card) => card.item.txn.index)).toEqual([1, 2, 3, 5]);
        expect(review.cards[0]?.status).toBe("skipped");
        expect(review.current?.item.txn.index).toBe(2);
        expect(review.current?.draft?.description).toBe("typing");
        expect(review.cards[2]).toMatchObject({draft: null});
        expect(review.cards[2]?.item.fingerprint).toBe("fp-3-9");
    });

    it("never grows a Recents run: the transaction a save lets into the top N is not appended", async () => {
        let after = false;
        serveReview({
            // Before: the top 3 are 1–3. After 1 is reviewed, 4 slides into the top 3.
            queue: () => (after ? page("recents", [2, 3, 4], "g2") : page("recents", [1, 2, 3])),
            batch: (body) => {
                after = true;
                return landed(body.edits, "g2", {reviewed: true});
            },
        });
        await review.open("recents", {limit: 3});
        await review.save([TAG]);
        await vi.waitFor(() => expect(review.generation).toBe("g2"));
        expect(review.cards.map((card) => card.item.txn.index)).toEqual([1, 2, 3]);
        expect(review.count).toBe(3);
    });

    it("re-validates in the background after a save", async () => {
        let after = false;
        const seen = serveReview({
            queue: () => (after ? page("unknowns", [2, 3], "g2") : page("unknowns", [1, 2, 3])),
            batch: (body) => {
                after = true;
                return landed(body.edits);
            },
        });
        await review.open("unknowns");
        await review.save([COFFEE, TAG]);
        await vi.waitFor(() => expect(seen.filter((r) => r.url.pathname.endsWith("/api/review/queue"))).toHaveLength(2));
        expect(review.cards.map((card) => card.item.txn.index)).toEqual([1, 2, 3]);
        expect(review.cards[0]?.status).toBe("saved");
    });
});
