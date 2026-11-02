// INTEGRATION: the transaction-review wire (plan 32) against a live engine
// (`just test-integration`, which serves fixtures/sample.journal as of
// 2026-07-08, the date every other live suite pins). This pins the CONTRACT —
// the engine's real bytes decode through the SPA's decoders, and the routes
// agree with each other — without writing: the engine is serving a committed
// fixture, so the batch is exercised only on paths that refuse before writing.
// The writes themselves are pinned in Rust (crates/ledgeline-server/tests/
// review_endpoints.rs) over a temp copy of fixtures/review/.

import {describe, expect, it} from "vitest";
import {decodeJournalSummary, decodeRowsPage} from "#lib/api/journalViewDecode.js";
import {ConflictError, LedgelineApi, ValidationError, type WireEditOp} from "#lib/api/native.js";
import {decodeDescriptionSuggestions, decodePayees, decodeQueuePage, decodeReviewItem, decodeSimilarSet, decodeTags} from "#lib/api/reviewDecode.js";
import {formToBody, formToOps, reviewFormOf, txnToForm} from "#lib/api/editMapping.js";

const apiUrl = process.env.LEDGELINE_API_URL;
const TODAY = "2026-07-08";
const ALL = {from: null, to: null, accounts: new Set<string>(), query: ""};

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live review routes", () => {
    const api = (): LedgelineApi => new LedgelineApi(apiUrl ?? "");

    it("serves both queues, and the summary's badges agree with them", async () => {
        const unknowns = decodeQueuePage(await api().reviewQueue("unknowns", {today: TODAY, maxDays: 0}));
        const recents = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 50}));
        expect(unknowns.kind).toBe("unknowns");
        expect(recents.kind).toBe("recents");
        expect(recents.total).toBeGreaterThan(0);
        expect(recents.items.length).toBe(Math.min(recents.total, 50));
        for (const item of [...unknowns.items, ...recents.items]) {
            expect(typeof item.fingerprint).toBe("string");
            expect(item.txn.date <= TODAY).toBe(true);
        }
        expect(unknowns.items.every((item) => item.unknown)).toBe(true);
        expect(recents.items.every((item) => !item.reviewed)).toBe(true);
        const summary = decodeJournalSummary(await api().journalSummary(ALL, {today: TODAY, limit: 50, maxDays: 0}));
        expect(summary.review).not.toBeNull();
        expect(summary.review?.unknown).toBe(unknowns.total);
        const windowed = decodeJournalSummary(await api().journalSummary(ALL, {today: TODAY, limit: 50}));
        expect(windowed.review?.unreviewedRecent).toBe(Math.min(recents.total, 50));
    });

    it("serves one transaction as the queue item the queue serves (the editor's single mode)", async () => {
        const recents = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 50}));
        const queued = recents.items[0]!;
        const {generation, item} = decodeReviewItem(await api().reviewItem(queued.txn.index, TODAY));
        expect(generation).toBe(recents.generation);
        expect(item).toEqual(queued);
        // A reviewed transaction no queue holds is served too, with its fingerprint.
        const page = decodeRowsPage(await api().journalRows(ALL, {offset: 0, limit: 200, today: TODAY}));
        const reviewed = page.rows.find((row) => row.reviewed);
        if (reviewed !== undefined) {
            const single = decodeReviewItem(await api().reviewItem(reviewed.txn.index, TODAY)).item;
            expect(single.txn.index).toBe(reviewed.txn.index);
            expect(typeof single.fingerprint).toBe("string");
        }
    });

    it("serves a similar set for a queued card", async () => {
        const recents = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 50}));
        const keyed = recents.items.find((item) => item.key !== null);
        expect(keyed).toBeDefined();
        const queued = recents.items.map((item) => item.txn.index);
        const set = decodeSimilarSet(await api().reviewSimilar(keyed!.txn.index, TODAY, queued));
        expect(set.index).toBe(keyed!.txn.index);
        expect(set.key).toBe(keyed!.key);
        expect(set.candidates.every((candidate) => candidate.txn.index !== keyed!.txn.index && !candidate.reviewed)).toBe(true);
        expect(set.inQueue.every((index) => queued.includes(index))).toBe(true);
    });

    it("serves the payee and tag vocabularies, and rows carry review markers", async () => {
        const payees = decodePayees(await api().payees());
        expect(payees.payees.length).toBeGreaterThan(0);
        expect(payees.payees[0]!.count >= (payees.payees.at(-1)?.count ?? 0)).toBe(true);
        decodeTags(await api().tags());
        const page = decodeRowsPage(await api().journalRows(ALL, {offset: 0, limit: 20, today: TODAY}));
        expect(page.rows.every((row) => typeof row.unknown === "boolean" && typeof row.reviewed === "boolean")).toBe(true);
    });

    it("marks elided legs on the rows; the form leaves them blank and writes them back blank (D35)", async () => {
        const page = decodeRowsPage(await api().journalRows(ALL, {offset: 0, limit: 200, today: TODAY}));
        const row = page.rows.find((candidate) => candidate.txn.postings.some((posting) => posting.elided === true));
        expect(row).toBeDefined();
        const txn = row!.txn;
        const at = txn.postings.findIndex((posting) => posting.elided === true);
        // The engine still serves the inferred amount, for everything that reads it.
        expect(txn.postings[at]!.amounts.length).toBeGreaterThan(0);
        const form = txnToForm(txn);
        expect(form.postings[at]).toMatchObject({amount: "", elided: true});
        expect(formToBody(form, "$").postings[at]!.amount).toBeUndefined();
        // Untouched, the review card saves nothing at all.
        expect(formToOps(txn, reviewFormOf(txn))).toEqual([]);
    });

    it("suggests for a description alone, in the queue items' shape (D36)", async () => {
        const payees = decodePayees(await api().payees());
        const payee = payees.payees.find((use) => /[a-z]/i.test(use.payee));
        expect(payee).toBeDefined();
        const found = decodeDescriptionSuggestions(await api().reviewSuggest(payee!.payee, TODAY));
        expect(typeof found.generation).toBe("string");
        expect(found.key).not.toBeNull();
        expect(found.suggestions.length).toBeLessThanOrEqual(3);
        const none = decodeDescriptionSuggestions(await api().reviewSuggest("12345", TODAY));
        expect(none).toMatchObject({key: null, suggestions: []});
    });

    it("refuses the new header ops' bad input (400) and a stale one (409) without writing", async () => {
        const recents = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 1}));
        const card = recents.items[0]!;
        const edit = (op: WireEditOp, fingerprint = card.fingerprint ?? "") =>
            api().batchTransactions({edits: [{index: card.txn.index, fingerprint, ops: [op]}]});
        await expect(edit({op: "setDate", date: "2026-02-30"})).rejects.toBeInstanceOf(ValidationError);
        await expect(edit({op: "setCode", code: "a)b"})).rejects.toBeInstanceOf(ValidationError);
        await expect(edit({op: "setStatus", status: "pending"}, "stale")).rejects.toBeInstanceOf(ConflictError);
        const again = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 1}));
        expect(again.generation).toBe(recents.generation);
        expect(again.items[0]?.fingerprint).toBe(card.fingerprint);
    });

    it("refuses a stale fingerprint (409) and an empty batch (400) without writing", async () => {
        const recents = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 1}));
        const card = recents.items[0]!;
        await expect(
            api().batchTransactions({edits: [{index: card.txn.index, fingerprint: "stale", ops: [{op: "setTxnTag", name: "reviewed", value: TODAY}]}]})
        ).rejects.toBeInstanceOf(ConflictError);
        await expect(api().batchTransactions({edits: []})).rejects.toBeInstanceOf(ValidationError);
        const again = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 1}));
        expect(again.generation).toBe(recents.generation);
        expect(again.items[0]?.fingerprint).toBe(card.fingerprint);
    });
});
