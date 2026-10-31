// A STAGED review run (plan 32 Phase 4): the overlay over an import's
// transactions before they are written. Cards are handed in, similar is found
// by key, and a save goes to the import's sink — never to the network.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {decodeCsvStagedTxn} from "#lib/api/importReviewDecode.js";
import type {StagedSave, StagedTxn} from "#lib/imports/reviewTypes.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {queuePageWire, reviewTxnWire, serveReview, similarWire, stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {decodeReviewTxn} from "#lib/api/reviewDecode.js";
import {resetReview, review} from "./review.svelte";

function staged(ordinal: number, description: string, marks: {unknown?: boolean; key?: string | null} = {}): StagedTxn {
    const account = marks.unknown === false ? "expenses:home" : "expenses:unknown";
    return decodeCsvStagedTxn(
        stagedTxnWire(txnFixture(ordinal + 1, description, account), {ordinal, unknown: marks.unknown ?? true, key: marks.key}),
        "staged"
    );
}

/** A sink that records every save and answers `result`. */
function recordingSink(result: {ok: true} | {ok: false; failure: {kind: "validation"; message: string}} = {ok: true}) {
    const saves: StagedSave[] = [];
    return {saves, sink: {stage: (save: StagedSave) => (saves.push(save), result)}};
}

const ITEMS = (): StagedTxn[] => [
    staged(0, "HARDWARE STORE 1", {key: "hardware store"}),
    staged(1, "ACME PAYROLL", {unknown: false, key: "acme payroll"}),
    staged(2, "HARDWARE STORE 2", {key: "hardware store"}),
    staged(3, "PEETS", {key: "peets"}),
];

const SUPPLIES = {op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"} as const;
const REVIEWED = {op: "setTxnTag", name: "reviewed", value: "2026-10-30"} as const;

beforeEach(async () => {
    resetReview();
    await connectFakeEngine();
});
afterEach(() => vi.unstubAllGlobals());

describe("UNIT review run — staged", () => {
    it("opens over the import's unknowns, or all of it, without asking for a queue", async () => {
        const seen = serveReview({queue: () => queuePageWire("unknowns", [])});
        review.openStaged("unknowns", ITEMS(), recordingSink().sink);
        expect(review.source).toBe("staged");
        expect(review.kind).toBe("unknowns");
        expect(review.cards.map((card) => card.item.txn.index)).toEqual([1, 3, 4]);
        review.openStaged("all", ITEMS(), recordingSink().sink);
        expect(review.count).toBe(4);
        expect(review.total).toBe(4);
        await vi.waitFor(() => expect(seen.some((r) => r.url.pathname.endsWith("/api/review/similar-key"))).toBe(true));
        expect(seen.some((r) => r.url.pathname.endsWith("/api/review/queue"))).toBe(false);
    });

    it("finds a card's journal siblings by its similarity key", async () => {
        const candidate = reviewTxnWire(txnFixture(40, "HARDWARE STORE 9"));
        const seen = serveReview({queue: () => queuePageWire("unknowns", []), similarKey: (key) => ({...similarWire(0, [candidate]), key})});
        review.openStaged("unknowns", ITEMS(), recordingSink().sink);
        await vi.waitFor(() => expect(review.similar(1)).not.toBeNull());
        expect(review.similar(1)?.candidates.map((txn) => txn.txn.index)).toEqual([40]);
        const call = seen.find((r) => r.url.pathname.endsWith("/api/review/similar-key"));
        expect(call?.url.searchParams.get("key")).toBe("hardware store");
        expect(call?.url.searchParams.get("today")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    });

    it("has no similar set for a card with no key", async () => {
        serveReview({queue: () => queuePageWire("unknowns", [])});
        review.openStaged("all", [staged(0, "12345", {key: null})], recordingSink().sink);
        expect(await review.loadSimilar(1)).toBeNull();
    });

    it("lists the staged siblings that share the card's key", () => {
        serveReview({queue: () => queuePageWire("unknowns", [])});
        review.openStaged("all", ITEMS(), recordingSink().sink);
        expect(review.stagedSimilar(1).map((txn) => txn.ref)).toEqual([{kind: "csv", ordinal: 2}]);
        expect(review.stagedSimilar(4)).toEqual([]);
    });

    it("saves into the sink: the card and its staged sibling as overrides, journal siblings as journal edits", async () => {
        const seen = serveReview({queue: () => queuePageWire("unknowns", [])});
        const {saves, sink} = recordingSink();
        const items = ITEMS();
        review.openStaged("unknowns", items, sink);
        const draft = {date: "2026-10-01", description: "HARDWARE STORE 1", comment: "", tags: [], postings: []};
        review.setDraft(draft);
        const journal = decodeReviewTxn(reviewTxnWire(txnFixture(40, "HARDWARE STORE 9")));
        const result = await review.save(
            [SUPPLIES, REVIEWED],
            [
                {txn: items[2] as StagedTxn, ops: [SUPPLIES]},
                {txn: journal, ops: [SUPPLIES]},
            ]
        );
        expect(result).toEqual({ok: true});
        expect(saves).toHaveLength(1);
        expect(saves[0]?.overrides.map((entry) => [entry.item.ref, entry.ops])).toEqual([
            [{kind: "csv", ordinal: 0}, [SUPPLIES, REVIEWED]],
            [{kind: "csv", ordinal: 2}, [SUPPLIES]],
        ]);
        expect(saves[0]?.journalEdits).toEqual([{index: 40, fingerprint: "fp-40-0", ops: [SUPPLIES]}]);
        // Nothing was written over the network.
        expect(seen.some((r) => r.method === "POST")).toBe(false);
        // The card is saved and keeps its edits for ←; the recategorised sibling left the Unknowns run.
        expect(review.cards[0]?.status).toBe("saved");
        expect(review.cards[0]?.draft).toEqual(draft);
        expect(review.tally.fixed).toBe(1);
        expect(review.cards.map((card) => card.item.txn.index)).toEqual([1, 4]);
        expect(review.current?.item.txn.index).toBe(4);
    });

    it("in Review All, a sibling the save changed stays and says which card changed it", async () => {
        serveReview({queue: () => queuePageWire("unknowns", [])});
        const items = ITEMS();
        review.openStaged("all", items, recordingSink().sink);
        await review.save([SUPPLIES], [{txn: items[2] as StagedTxn, ops: [SUPPLIES]}]);
        expect(review.cards.find((card) => card.item.txn.index === 3)?.updatedFrom).toBe(1);
        expect(review.tally.fixed).toBe(0);
    });

    it("stays on the card, with the error, when the sink refuses", async () => {
        serveReview({queue: () => queuePageWire("unknowns", [])});
        review.openStaged("unknowns", ITEMS(), recordingSink({ok: false, failure: {kind: "validation", message: "nope"}}).sink);
        const result = await review.save([REVIEWED]);
        expect(result.ok).toBe(false);
        expect(review.cursor).toBe(0);
        expect(review.cards[0]?.status).toBe("open");
        expect(review.failure).toEqual({index: 1, failure: {kind: "validation", message: "nope"}});
    });

    it("ignores the journal's generation", async () => {
        const seen = serveReview({queue: () => queuePageWire("unknowns", [])});
        review.openStaged("unknowns", ITEMS(), recordingSink().sink);
        review.syncGeneration("g9");
        await review.settled();
        expect(seen.some((r) => r.url.pathname.endsWith("/api/review/queue"))).toBe(false);
    });
});
