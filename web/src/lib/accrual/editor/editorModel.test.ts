// The D-editor table (plan 45), read from a card: which accrual line each
// posting is, the tags a schedule writes onto its source posting, and what
// holds a draft back.

import {describe, expect, it} from "vitest";
import type {PostingForm} from "#lib/api/editMapping.js";
import type {AccountClass, AccountClasses, AccountType} from "#lib/domain/accountClass.js";
import type {ReviewForm} from "#lib/review/types.js";
import {commentTag} from "../model";
import type {AccrualRole} from "../types";
import {applyDrafts, classifyCard, defaultTarget, draftProblem, freshDraft, summarize} from "./editorModel";

const ROLES: Record<string, AccrualRole> = {
    "liabilities:deferred-revenue": "deferred-revenue",
    "assets:prepaid": "prepaid",
    "assets:receivable": "receivable",
    "liabilities:payable": "payable",
};
const TYPES: Record<string, AccountType> = {
    "assets:checking": "cash",
    "revenue:sales": "revenue",
    "expenses:insurance": "expense",
    "expenses:fees": "expense",
    "liabilities:deferred-revenue": "liability",
    "assets:prepaid": "asset",
    "assets:receivable": "asset",
    "liabilities:payable": "liability",
};
const classes: AccountClasses = new Map(
    Object.entries(TYPES).map(([name, type]): [string, AccountClass] => [name, {type, typeDeclared: true, term: null, termGuessed: false} as AccountClass])
);
const roleOf = (account: string): AccrualRole | null => ROLES[account] ?? null;

function leg(account: string, amount: string, comment = ""): PostingForm {
    return {account, amount, commodity: "$", status: "unmarked", comment, cost: null, type: "regular", balanceAssertion: null};
}
function card(...postings: PostingForm[]): ReviewForm {
    return {date: "2026-11-17", date2: "", status: "unmarked", code: "", description: "Customer X Payment", comment: "", tags: [], postings};
}
const kinds = (form: ReviewForm, tags: [string, string][] = []): string[] =>
    classifyCard(form, roleOf, classes, tags).map((line) => `${line.posting}:${line.kind}`);

describe("UNIT accrual editorModel classifyCard (the D-editor table)", () => {
    it("reads a deferred-revenue credit with no recognition tag as a deferral to schedule", () => {
        expect(kinds(card(leg("assets:checking", "12000"), leg("liabilities:deferred-revenue", "-12000")))).toEqual(["1:unscheduled"]);
    });

    it("reads the same line with recognition: as scheduled, and a prepaid debit likewise", () => {
        expect(kinds(card(leg("assets:checking", "-6000"), leg("assets:prepaid", "6000", "vendor: Northwind, recognition: nw0301")))).toEqual(["1:scheduled"]);
        expect(kinds(card(leg("assets:checking", "-6000"), leg("assets:prepaid", "6000")))).toEqual(["1:unscheduled"]);
    });

    it("offers earned-at-once-or-over-a-term for revenue beside cash with no deferral", () => {
        expect(kinds(card(leg("assets:checking", "500"), leg("revenue:sales", "-500")))).toEqual(["1:earned"]);
        expect(kinds(card(leg("liabilities:payable", "-300"), leg("expenses:insurance", "300")))).toEqual(["0:invoice", "1:earned"]);
    });

    it("reads AR debits and AP credits as invoices, and their opposites from cash as payments", () => {
        expect(kinds(card(leg("assets:receivable", "1000"), leg("revenue:sales", "-1000")))).toEqual(["0:invoice", "1:earned"]);
        expect(kinds(card(leg("assets:checking", "970"), leg("expenses:fees", "30"), leg("assets:receivable", "-1000")))).toEqual(["2:payment"]);
        // An AR credit with no cash leg (a write-off) is neither.
        expect(kinds(card(leg("expenses:fees", "1000"), leg("assets:receivable", "-1000")))).toEqual([]);
    });

    it("reads a deferred debit outside a recognition entry as a credit note, and leaves recognition entries alone", () => {
        expect(kinds(card(leg("liabilities:deferred-revenue", "1000"), leg("assets:checking", "-1000")))).toEqual(["0:credit"]);
        const entry = card(leg("liabilities:deferred-revenue", "1000"), leg("revenue:sales", "-1000"));
        expect(kinds(entry, [["recognized", "2026-11-17"]])).toEqual([]);
    });

    it("carries the party, invoice number and match tag from the card", () => {
        const form = {
            ...card(leg("assets:receivable", "1000", "customer: Customer X, match: m1"), leg("revenue:sales", "-1000")),
            tags: [["invoice", "2004"]] as [string, string][],
        };
        const [line] = classifyCard(form, roleOf, classes);
        expect(line).toMatchObject({kind: "invoice", party: {kind: "customer", name: "Customer X"}, invoice: "2004", matched: true});
    });
});

describe("UNIT accrual editorModel drafts", () => {
    const form = card(leg("assets:checking", "12000"), leg("liabilities:deferred-revenue", "-12000", "customer: Customer X, note"), leg("revenue:sales", "0"));

    it("starts a draft on the transaction date for 12 months, recognizing into the card's revenue leg", () => {
        const [line] = classifyCard(form, roleOf, classes).filter((l) => l.kind === "unscheduled");
        if (line === undefined) throw new Error("no line");
        const draft = freshDraft("k7f3a2", form, [line], [defaultTarget(form, line, classes, null)], line.party, "new");
        expect(draft).toMatchObject({
            id: "k7f3a2",
            start: "2026-11-17",
            term: {months: 12},
            proration: "anniversary",
            party: {kind: "customer", name: "Customer X"},
            description: "Customer X Recognized",
            lines: [{sourceAccount: "liabilities:deferred-revenue", targetAccount: "revenue:sales", commodity: "$", quantity: "-12000"}],
        });
        expect(draftProblem(draft)).toBeNull();
        expect(defaultTarget(form, line, classes, "revenue:subscriptions")).toBe("revenue:subscriptions");
    });

    it("writes the source tags onto exactly the draft's postings, keeping free text and other tags", () => {
        const [line] = classifyCard(form, roleOf, classes).filter((l) => l.kind === "unscheduled");
        if (line === undefined) throw new Error("no line");
        const draft = {...freshDraft("k7f3a2", form, [line], ["revenue:sales"], line.party, "renewal"), start: "2026-12-01", proration: "calendar" as const};
        const tagged = applyDrafts(form, [{draft, postings: [1]}]);
        const comment = tagged.postings[1]?.comment ?? "";
        expect(comment).toBe(
            "note, customer: Customer X, recognition: k7f3a2, service-start: 2026-12-01, term-months: 12, proration: calendar, contract: renewal"
        );
        expect(commentTag(comment, "recognition")).toBe("k7f3a2");
        expect(tagged.postings[0]?.comment).toBe("");
        // Re-applied with other terms, the old tags go: never two term tags.
        const again = applyDrafts(tagged, [{draft: {...draft, term: {through: "2027-06-30"}}, postings: [1]}]);
        expect(again.postings[1]?.comment).not.toContain("term-months");
        expect(commentTag(again.postings[1]?.comment ?? "", "service-end")).toBe("2027-06-30");
    });

    it("says what holds a draft back", () => {
        const base = freshDraft(
            "k7f3a2",
            form,
            classifyCard(form, roleOf, classes).filter((l) => l.kind === "unscheduled"),
            [""],
            null,
            null
        );
        expect(draftProblem(base)).toBe("Choose the account to recognize into.");
        const targeted = {...base, lines: base.lines.map((l) => ({...l, targetAccount: "revenue:sales"}))};
        expect(draftProblem({...targeted, term: {through: "2026-11-01"}})).toBe("The end date must be after the service start.");
        expect(draftProblem({...targeted, term: {through: ""}})).toBe("Choose a term.");
    });

    it("summarizes a served schedule's period statuses", () => {
        const periods = [
            {date: "2026-09-17", status: "posted"},
            {date: "2026-10-17", status: "closed"},
            {date: "2026-11-17", status: "due"},
            {date: "2026-12-17", status: "future"},
        ];
        expect(summarize(periods, "2026-11-17")).toEqual({periods: 4, posted: 1, due: 1, closed: 1, next: "2026-12-17"});
    });
});
