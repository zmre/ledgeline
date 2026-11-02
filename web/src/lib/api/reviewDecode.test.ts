// The review decoders: every field checked, nothing defaulted to a quiet zero.
// The live round trip (engine → JSON → these decoders) is
// `review/reviewLive.integration.test.ts`; these pin the shapes and refusals.

import {describe, expect, it} from "vitest";
import {queueItemWire, queuePageWire, reviewTxnWire, similarWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {wireRow} from "#lib/testing/journalViewFixture.js";
import {ApiShapeError} from "./client";
import {decodeJournalSummary, decodeRow} from "./journalViewDecode";
import {decodeBatchResult, decodeDescriptionSuggestions, decodePayees, decodeQueuePage, decodeRulesFor, decodeSimilarSet, decodeTags} from "./reviewDecode";

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

describe("UNIT review decoders — plan 32 Phase 7a", () => {
    const FILL = {amount: {commodity: "$", quantity: {mantissa: "1549", places: 2}}, account: "liabilities:visa"};

    it("decodes a description's suggestions, with a recurring fill only where one is sent", () => {
        const body = decodeDescriptionSuggestions({
            generation: "g3",
            key: "netflix",
            suggestions: [{...SUGGESTION, account: "expenses:streaming", recurring: FILL}, SUGGESTION],
        });
        expect(body.generation).toBe("g3");
        expect(body.key).toBe("netflix");
        expect(body.suggestions[0]?.recurring).toEqual({amount: {commodity: "$", qty: {m: 1549n, p: 2}}, account: "liabilities:visa"});
        expect(body.suggestions[1]).toEqual(SUGGESTION);
        expect("recurring" in (body.suggestions[1] ?? {})).toBe(false);
        expect(Object.isFrozen(body.suggestions[0]?.recurring)).toBe(true);
        expect(decodeDescriptionSuggestions({generation: "g3", key: null, suggestions: []}).key).toBeNull();
    });

    it.each<[string, unknown]>([
        ["a missing key", {generation: "g", suggestions: []}],
        ["a fill with no account", {generation: "g", key: "k", suggestions: [{...SUGGESTION, recurring: {amount: FILL.amount}}]}],
        [
            "a fill with a float mantissa",
            {
                generation: "g",
                key: "k",
                suggestions: [{...SUGGESTION, recurring: {...FILL, amount: {commodity: "$", quantity: {mantissa: "15.49", places: 2}}}}],
            },
        ],
        [
            "a fill with no commodity",
            {generation: "g", key: "k", suggestions: [{...SUGGESTION, recurring: {...FILL, amount: {quantity: {mantissa: "1", places: 0}}}}]},
        ],
    ])("refuses a description's suggestions with %s", (_, body) => {
        expect(() => decodeDescriptionSuggestions(body)).toThrow(ApiShapeError);
    });

    it("keeps a row posting's elided marker, and only a true one", () => {
        const wire = wireRow(CARD);
        const postings = wire.postings as Record<string, unknown>[];
        postings[1] = {...postings[1], elided: true};
        const row = decodeRow(wire);
        expect(row.txn.postings[1]?.elided).toBe(true);
        expect(row.txn.postings[0]).not.toHaveProperty("elided");
        postings[1] = {...postings[1], elided: false};
        expect(decodeRow(wire).txn.postings[1]).not.toHaveProperty("elided");
        postings[1] = {...postings[1], elided: "yes"};
        expect(() => decodeRow(wire)).toThrow(ApiShapeError);
    });
});

describe("UNIT review decoders — rules for a journal transaction (D27)", () => {
    const candidate = (rulesId: string, recommended: boolean, extra: Record<string, unknown> = {}) => ({
        rulesId,
        label: rulesId.replace(/^.*\//, "").replace(/\.csv\.rules$/, ""),
        family: "citi",
        familySize: 2,
        year: 2026,
        account1: "liabilities:credit-card:citi",
        reasons: ["Imports into liabilities:credit-card:citi, this transaction's account"],
        recommended,
        matchField: "description",
        fields: ["date", "description", "amount"],
        ...extra,
    });

    it("decodes the candidates, best first, with the one recommended", () => {
        const found = decodeRulesFor({
            index: 3,
            needsChoice: false,
            candidates: [
                candidate("import/citi-2026.csv.rules", true),
                candidate("import/citi-2025.csv.rules", false, {year: null, fields: null, matchField: null}),
            ],
        });
        expect(found.index).toBe(3);
        expect(found.needsChoice).toBe(false);
        expect(found.candidates.map((c) => c.rulesId)).toEqual(["import/citi-2026.csv.rules", "import/citi-2025.csv.rules"]);
        expect(found.candidates[0]).toMatchObject({recommended: true, year: 2026, familySize: 2, matchField: "description"});
        expect(found.candidates[1]).toMatchObject({recommended: false, year: null, fields: null, matchField: null});
        expect(Object.isFrozen(found.candidates[0])).toBe(true);
    });

    it("decodes no candidates, and a tie that needs a choice", () => {
        expect(decodeRulesFor({index: 9, needsChoice: false, candidates: []}).candidates).toEqual([]);
        const tie = decodeRulesFor({index: 5, needsChoice: true, candidates: [candidate("a.rules", false), candidate("b.rules", false)]});
        expect(tie.needsChoice).toBe(true);
    });

    it("refuses a body whose recommendation and needsChoice disagree", () => {
        const two = [candidate("a.rules", true), candidate("b.rules", true)];
        expect(() => decodeRulesFor({index: 1, needsChoice: false, candidates: two})).toThrow(ApiShapeError);
        expect(() => decodeRulesFor({index: 1, needsChoice: false, candidates: [candidate("a.rules", false)]})).toThrow(ApiShapeError);
        expect(() => decodeRulesFor({index: 1, needsChoice: true, candidates: [candidate("a.rules", true)]})).toThrow(ApiShapeError);
        expect(() => decodeRulesFor({index: 1, needsChoice: true, candidates: []})).toThrow(ApiShapeError);
        expect(() => decodeRulesFor({index: 1, needsChoice: false, candidates: [candidate("a.rules", true, {reasons: "x"})]})).toThrow(ApiShapeError);
    });
});
