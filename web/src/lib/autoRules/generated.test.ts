// Generated postings (plan 30 Phase 5) through the pure layers: the row
// decoder keeps the marker, the edit form never holds one, the PATCH builder
// addresses the written posting by its SERVED position, and the row model's
// chips and amount read only what the file writes.

import {describe, expect, it} from "vitest";
import {accountPatch, formToBody, postingIndicesForAccount, txnToForm} from "#lib/api/editMapping.js";
import {ApiShapeError} from "#lib/api/client.js";
import {decodeRow} from "#lib/api/journalViewDecode.js";
import {formatAmount} from "#lib/domain/money.js";
import {accountFlow, generatedPostings, generatingRules, txnFlowAmounts, writtenPostings} from "#lib/journal/rowModel.js";
import {autoSale, autoSaleWire} from "#lib/testing/autoRow.js";
import {autoRuleSearch} from "./nav";

describe("UNIT generated postings — decode", () => {
    it("keeps the rule a generated posting names, and leaves written ones unmarked", () => {
        const sale = autoSale();
        expect(sale.postings.map((posting) => posting.generated ?? null)).toEqual([
            null,
            {rule: 0, query: "revenue:sales"},
            {rule: 0, query: "revenue:sales"},
            null,
        ]);
    });

    it("refuses a malformed marker rather than treating the posting as written", () => {
        const wire = autoSaleWire();
        const postings = wire.postings as Record<string, unknown>[];
        postings[1] = {...postings[1], generated: {rule: "0", query: "revenue:sales"}};
        expect(() => decodeRow(wire)).toThrow(ApiShapeError);
        postings[1] = {...postings[1], generated: {rule: 0}};
        expect(() => decodeRow(wire)).toThrow(ApiShapeError);
    });
});

describe("UNIT generated postings — edit mapping", () => {
    it("txnToForm drops generated postings, so formToBody can never send one", () => {
        const form = txnToForm(autoSale());
        expect(form.postings.map((posting) => posting.account)).toEqual(["revenue:sales", "assets:checking"]);
        const body = formToBody(form, "$");
        expect(body.postings).toHaveLength(2);
        expect(JSON.stringify(body)).not.toContain("generated");
        expect(JSON.stringify(body)).not.toContain("liabilities:tax:sales");
    });

    it("recategorizes the WRITTEN posting at its served position, skipping the rule's one on the same account", () => {
        const sale = autoSale();
        expect(postingIndicesForAccount(sale, "assets:checking")).toEqual([3]);
        expect(accountPatch(sale, "assets:checking", "assets:savings")).toEqual({postings: [{index: 3, account: "assets:savings"}]});
    });

    it("never patches an account only a rule posts to", () => {
        expect(accountPatch(autoSale(), "liabilities:tax:sales", "liabilities:tax:other")).toEqual({postings: []});
    });
});

describe("UNIT generated postings — row model", () => {
    it("reads the source→destination flow from the written postings only", () => {
        expect(accountFlow(autoSale())).toEqual({kind: "flow", source: "revenue:sales", dest: "assets:checking"});
    });

    it("shows the sale's own amount, not the sale plus its tax", () => {
        const amounts = txnFlowAmounts(autoSale());
        expect(amounts.map((a) => formatAmount(a))).toEqual(["$333.33"]);
    });

    it("splits written from generated and names each rule once", () => {
        const sale = autoSale();
        expect(writtenPostings(sale)).toHaveLength(2);
        expect(generatedPostings(sale).map((posting) => posting.account)).toEqual(["liabilities:tax:sales", "assets:checking"]);
        expect(generatingRules(sale)).toEqual([{rule: 0, query: "revenue:sales"}]);
    });
});

describe("UNIT autoRuleSearch", () => {
    it("lands on the Automatic postings tab with the rule focused", () => {
        expect(autoRuleSearch(2)).toBe("?tab=auto&rule=2");
    });
});
