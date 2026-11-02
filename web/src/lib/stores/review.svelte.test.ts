// The review run AT REST: opening a queue, moving through it, and what is
// fetched ahead of the user (D12, D14). Saves are `review.save.svelte.test.ts`
// — the store is a singleton, so the two are kept in separate files.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {queueItemWire, queuePageWire, serveReview, txnFixture} from "#lib/testing/reviewFixture.js";
import {resetReview, review} from "./review.svelte";

const items = (count: number, from = 1): Record<string, unknown>[] =>
    Array.from({length: count}, (_, i) => queueItemWire(txnFixture(from + i, `Payee ${from + i}`)));

beforeEach(async () => {
    resetReview();
    await connectFakeEngine();
});
afterEach(() => vi.unstubAllGlobals());

describe("UNIT review run", () => {
    it("opens on the first card with the whole page in hand, and prefetches similar for it and the next", async () => {
        const seen = serveReview({queue: () => queuePageWire("recents", items(3), {notEditable: 2})});
        await review.open("recents", {limit: 50, maxDays: 365});
        expect(review.kind).toBe("recents");
        expect(review.count).toBe(3);
        expect(review.position).toBe(1);
        expect(review.notEditable).toBe(2);
        expect(review.current?.item.txn.index).toBe(1);
        const queueCall = seen.find((r) => r.url.pathname.endsWith("/api/review/queue"));
        expect(queueCall?.url.searchParams.get("kind")).toBe("recents");
        expect(queueCall?.url.searchParams.get("limit")).toBe("50");
        expect(queueCall?.url.searchParams.get("maxDays")).toBe("365");
        expect(queueCall?.url.searchParams.get("today")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
        await vi.waitFor(() => expect(review.similar(2)).not.toBeNull());
        const similar = seen.filter((r) => r.url.pathname.includes("/api/review/similar/"));
        expect(similar.map((r) => r.url.pathname.split("/").pop())).toEqual(["1", "2"]);
        // The run tells the engine which cards are queued, for `inQueue`.
        expect(similar[0]?.url.searchParams.get("queue")).toBe("1,2,3");
    });

    it("skips (this run only) and goes back, and prefetches as it moves", async () => {
        const seen = serveReview({queue: () => queuePageWire("unknowns", items(4))});
        await review.open("unknowns");
        review.skip();
        expect(review.cursor).toBe(1);
        expect(review.cards[0]?.status).toBe("skipped");
        expect(review.tally.skipped).toBe(1);
        await vi.waitFor(() => expect(review.similar(3)).not.toBeNull());
        review.back();
        expect(review.current?.item.txn.index).toBe(1);
        review.go(99);
        expect(review.current).toBeNull();
        expect(review.position).toBe(4);
        // Each similar set is fetched once, however often its card is shown.
        const similar = seen.filter((r) => r.url.pathname.includes("/api/review/similar/")).map((r) => r.url.pathname.split("/").pop());
        expect(new Set(similar).size).toBe(similar.length);
    });

    it("keeps a card's draft while moving away from it and back", async () => {
        serveReview({queue: () => queuePageWire("unknowns", items(2))});
        await review.open("unknowns");
        const draft = {date: "2026-10-01", date2: "", status: "unmarked" as const, code: "", description: "Renamed", comment: "", tags: [], postings: []};
        review.setDraft(draft);
        review.skip();
        review.back();
        expect(review.current?.draft).toEqual(draft);
    });

    it("pages Unknowns as the cursor nears the end of what is held", async () => {
        const seen = serveReview({
            queue: (url) => {
                const offset = Number(url.searchParams.get("offset") ?? 0);
                return queuePageWire("unknowns", items(offset === 0 ? 100 : 20, offset + 1), {total: 120, offset});
            },
        });
        await review.open("unknowns");
        expect(review.count).toBe(100);
        expect(review.total).toBe(120);
        review.go(95);
        await vi.waitFor(() => expect(review.count).toBe(120));
        const pages = seen.filter((r) => r.url.pathname.endsWith("/api/review/queue")).map((r) => r.url.searchParams.get("offset"));
        expect(pages).toEqual([null, "100"]);
        expect(review.current?.item.txn.index).toBe(96);
    });

    it("reports a queue that cannot be loaded", async () => {
        serveReview({queue: () => new Response("kind must be unknowns or recents", {status: 400})});
        await review.open("unknowns");
        expect(review.error).toContain("kind must be");
        expect(review.count).toBe(0);
        expect(review.loading).toBe(false);
    });
});
