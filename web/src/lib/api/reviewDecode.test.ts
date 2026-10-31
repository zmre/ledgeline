// The review decoders: every field checked, nothing defaulted to a quiet zero.
// The live round trip (engine → JSON → these decoders) is
// `review/reviewLive.integration.test.ts`; these pin the shapes and refusals.

import {describe, expect, it} from "vitest";
import {queueItemWire, queuePageWire, reviewTxnWire, similarWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {wireRow} from "#lib/testing/journalViewFixture.js";
import {ApiShapeError} from "./client";
import {decodeJournalSummary, decodeRow} from "./journalViewDecode";
import {decodeBatchResult, decodePayees, decodeQueuePage, decodeSimilarSet, decodeTags} from "./reviewDecode";

const CARD = txnFixture(12, "SQ *BLUE BOTTLE 4412 OAKLAND");
const SUGGESTION = {account: "expenses:food:coffee", description: "Blue Bottle Coffee", source: "exactKey", confidence: 7500, strong: true, count: 3};

describe("UNIT review decoders", () => {
    it("decodes a queue page: rows, markers, fingerprints, keys and suggestions", () => {
        const page = decodeQueuePage(
            queuePageWire("unknowns", [queueItemWire(CARD, {key: "sq blue bottle oakland", suggestions: [SUGGESTION]})], {total: 8, notEditable: 1})
        );
        expect(page).toMatchObject({generation: "g1", kind: "unknowns", total: 8, offset: 0, notEditable: 1});
        const item = page.items[0];
        expect(item?.txn.description).toBe("SQ *BLUE BOTTLE 4412 OAKLAND");
        expect(item?.txn.postings[0]?.amounts[0]?.qty).toEqual({m: 500n, p: 2});
        expect(item).toMatchObject({unknown: true, reviewed: false, fingerprint: "fp-12-0", key: "sq blue bottle oakland"});
        expect(item?.suggestions).toEqual([SUGGESTION]);
        expect(Object.isFrozen(item)).toBe(true);
    });

    it("keeps a null fingerprint (a read-only engine) and a null key", () => {
        const page = decodeQueuePage(queuePageWire("recents", [queueItemWire(CARD, {fingerprint: null, key: null})]));
        expect(page.items[0]).toMatchObject({fingerprint: null, key: null});
    });

    it.each<[string, (body: Record<string, unknown>) => void]>([
        ["an unknown kind", (body) => (body.kind = "all")],
        ["a missing total", (body) => delete body.total],
        ["a missing marker", (body) => delete (body.items as Record<string, unknown>[])[0]?.unknown],
        ["a numeric fingerprint", (body) => ((body.items as Record<string, unknown>[])[0]!.fingerprint = 7)],
        ["an unknown suggestion source", (body) => ((body.items as Record<string, unknown>[])[0]!.suggestions = [{...SUGGESTION, source: "magic"}])],
        ["a fractional confidence", (body) => ((body.items as Record<string, unknown>[])[0]!.suggestions = [{...SUGGESTION, confidence: 0.75}])],
    ])("refuses a queue page with %s", (_, spoil) => {
        const body = queuePageWire("unknowns", [queueItemWire(CARD, {suggestions: [SUGGESTION]})]);
        spoil(body);
        expect(() => decodeQueuePage(body)).toThrow(ApiShapeError);
    });

    it("decodes a similar set with its queued candidates and precedent", () => {
        const sibling = txnFixture(9, "SQ *BLUE BOTTLE 0917 OAKLAND");
        const set = decodeSimilarSet(similarWire(12, [reviewTxnWire(sibling)], {inQueue: [9], precedent: {account: "expenses:food:coffee", count: 2, of: 3}}));
        expect(set).toMatchObject({index: 12, total: 1, inQueue: [9], precedent: {account: "expenses:food:coffee", count: 2, of: 3}});
        expect(set.candidates[0]?.txn.index).toBe(9);
        expect(decodeSimilarSet(similarWire(12, [])).precedent).toBeNull();
        expect(() => decodeSimilarSet(similarWire(12, [], {inQueue: [0]}))).toThrow(ApiShapeError);
    });

    it("decodes a batch result, payees and tags", () => {
        const batch = decodeBatchResult({generation: "g2", edited: [reviewTxnWire(CARD, {unknown: false, reviewed: true, fingerprint: "fp-12-1"})]});
        expect(batch.generation).toBe("g2");
        expect(batch.edited[0]).toMatchObject({unknown: false, reviewed: true, fingerprint: "fp-12-1"});
        expect(decodePayees({generation: "g1", payees: [{payee: "Netflix", count: 6, last: "2026-09-14"}]}).payees).toEqual([
            {payee: "Netflix", count: 6, last: "2026-09-14"},
        ]);
        const tags = decodeTags({generation: "g1", tags: [{name: "trip", count: 3, values: [{value: "paris", count: 2}]}]});
        expect(tags.tags[0]).toEqual({name: "trip", count: 3, values: [{value: "paris", count: 2}]});
        expect(() => decodePayees({generation: "g1", payees: [{payee: "x", count: -1, last: "2026-01-01"}]})).toThrow(ApiShapeError);
    });

    it("reads the rows' markers, defaulting them only when absent", () => {
        expect(decodeRow({...wireRow(CARD), unknown: true, reviewed: false})).toMatchObject({unknown: true, reviewed: false});
        // A forecast row (projections) carries none.
        expect(decodeRow(wireRow(CARD))).toMatchObject({unknown: false, reviewed: false});
        expect(() => decodeRow({...wireRow(CARD), unknown: "yes"})).toThrow(ApiShapeError);
    });

    it("reads the summary's review counts, and null from an engine without them", () => {
        const base = {generation: "g1", total: 0, visibleNet: [], commodities: [], perCommodity: [], maxDepth: 1, primary: null};
        expect(decodeJournalSummary(base).review).toBeNull();
        const counts = {unknown: 8, unreviewedRecent: 17, notEditable: 0, notEditableRecent: 0};
        expect(decodeJournalSummary({...base, review: counts}).review).toEqual(counts);
        expect(() => decodeJournalSummary({...base, review: {...counts, unknown: undefined}})).toThrow(ApiShapeError);
    });
});
