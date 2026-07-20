import {describe, expect, it} from "vitest";
import type {Amount, AmountStyle, Posting, Transaction, TxnStatus} from "$lib/domain/types";
import {
    accountPatch,
    blankForm,
    decToInput,
    descriptionPatch,
    dominantCommodity,
    encodeDec,
    formToBody,
    parseAmountInput,
    postingIndicesForAccount,
    statusPatch,
    txnToForm,
    validateForm,
} from "./editMapping";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

function amount(commodity: string, m: bigint, p: number, cost?: Amount["cost"]): Amount {
    return {commodity, qty: {m, p}, style: STYLE, ...(cost !== undefined ? {cost} : {})};
}

function posting(account: string, amounts: Amount[], extra: {status?: TxnStatus; comment?: string} = {}): Posting {
    return {account, amounts, status: extra.status ?? "unmarked", comment: extra.comment ?? "", tags: []};
}

function txn(index: number, description: string, postings: Posting[], extra: {date2?: string; comment?: string} = {}): Transaction {
    return {
        index,
        date: "2026-07-20",
        status: "cleared",
        description,
        code: "",
        comment: extra.comment ?? "",
        tags: [],
        postings,
        haystack: "",
        ...(extra.date2 !== undefined ? {date2: extra.date2} : {}),
    };
}

describe("UNIT editMapping — Dec ⇆ string/wire", () => {
    it("encodes a Dec to the string-mantissa wire form", () => {
        expect(encodeDec({m: 5624n, p: 2})).toEqual({mantissa: "5624", places: 2});
        expect(encodeDec({m: -5624n, p: 2})).toEqual({mantissa: "-5624", places: 2});
        expect(encodeDec({m: 0n, p: 0})).toEqual({mantissa: "0", places: 0});
    });

    it("renders a Dec as an exact (un-rounded, un-grouped) input string", () => {
        expect(decToInput({m: 5624n, p: 2})).toBe("56.24");
        expect(decToInput({m: -5624n, p: 2})).toBe("-56.24");
        expect(decToInput({m: 5n, p: 2})).toBe("0.05");
        expect(decToInput({m: 1234567n, p: 0})).toBe("1234567");
        expect(decToInput({m: 123456789n, p: 8})).toBe("1.23456789");
    });

    it("round-trips string → Dec → string", () => {
        for (const s of ["56.24", "-56.24", "0.05", "1234567", "1.23456789", "0"]) {
            const dec = parseAmountInput(s);
            expect(dec).not.toBeNull();
            expect(decToInput(dec!)).toBe(s);
        }
    });
});

describe("UNIT editMapping — parseAmountInput", () => {
    it("parses signs, fractional-only, and leading zeros; strips grouping", () => {
        expect(parseAmountInput("56.24")).toEqual({m: 5624n, p: 2});
        expect(parseAmountInput("-56.24")).toEqual({m: -5624n, p: 2});
        expect(parseAmountInput(".5")).toEqual({m: 5n, p: 1});
        expect(parseAmountInput("007.50")).toEqual({m: 750n, p: 2});
        expect(parseAmountInput("1,234.56")).toEqual({m: 123456n, p: 2});
        expect(parseAmountInput("1 234.56")).toEqual({m: 123456n, p: 2});
        expect(parseAmountInput("100")).toEqual({m: 100n, p: 0});
    });

    it("returns null for blank / sign-only / non-numeric (the elided-leg marker)", () => {
        expect(parseAmountInput("")).toBeNull();
        expect(parseAmountInput("   ")).toBeNull();
        expect(parseAmountInput("-")).toBeNull();
        expect(parseAmountInput("abc")).toBeNull();
        expect(parseAmountInput("1.2.3")).toBeNull();
    });
});

describe("UNIT editMapping — form ⇆ body", () => {
    it("prefills a form from a transaction (amounts as exact strings)", () => {
        const t = txn(4, "Coffee", [posting("expenses:coffee", [amount("$", 500n, 2)]), posting("assets:cash", [amount("$", -500n, 2)])]);
        const form = txnToForm(t);
        expect(form.date).toBe("2026-07-20");
        expect(form.status).toBe("cleared");
        expect(form.description).toBe("Coffee");
        expect(form.postings).toHaveLength(2);
        expect(form.postings[0]).toMatchObject({account: "expenses:coffee", amount: "5.00", commodity: "$"});
        expect(form.postings[1].amount).toBe("-5.00");
    });

    it("carries a cost annotation through the form as a wire cost", () => {
        const t = txn(4, "Buy AAPL", [
            posting("assets:broker", [amount("AAPL", 10n, 0, {commodity: "$", qty: {m: 15000n, p: 2}, per: true})]),
            posting("assets:cash", []),
        ]);
        const form = txnToForm(t);
        expect(form.postings[0].cost).toEqual({kind: "unit", amount: {commodity: "$", quantity: {mantissa: "15000", places: 2}}});
    });

    it("builds a wire body, dropping blank-account rows and eliding a blank amount", () => {
        const form = blankForm("2026-07-20", "$");
        form.description = "Safeway";
        form.status = "cleared";
        form.postings[0] = {account: "expenses:food:groceries", amount: "56.24", commodity: "$", status: "unmarked", comment: "", cost: null};
        form.postings[1] = {account: "liabilities:cc:visa", amount: "", commodity: "$", status: "unmarked", comment: "", cost: null};
        const body = formToBody(form, "$");
        expect(body).toEqual({
            date: "2026-07-20",
            status: "cleared",
            description: "Safeway",
            postings: [
                {account: "expenses:food:groceries", amount: {commodity: "$", quantity: {mantissa: "5624", places: 2}}},
                {account: "liabilities:cc:visa"},
            ],
        });
    });

    it("never emits a blank commodity — a cleared commodity falls back to the default", () => {
        const form = blankForm("2026-07-20", "EUR");
        // A row whose commodity the user cleared, plus a row that keeps a value.
        form.postings[0] = {account: "expenses:food", amount: "50", commodity: "  ", status: "unmarked", comment: "", cost: null};
        form.postings[1] = {account: "assets:bank", amount: "-50", commodity: "EUR", status: "unmarked", comment: "", cost: null};
        const body = formToBody(form, "EUR");
        expect(body.postings[0].amount).toEqual({commodity: "EUR", quantity: {mantissa: "50", places: 0}});
        expect(body.postings[1].amount).toEqual({commodity: "EUR", quantity: {mantissa: "-50", places: 0}});
        // No amount on the body ever carries an empty commodity.
        for (const posting of body.postings) {
            if (posting.amount !== undefined) expect(posting.amount.commodity).not.toBe("");
        }
    });

    it("falls back to $ when both the row commodity and the supplied default are blank", () => {
        const form = blankForm("2026-07-20", "$");
        form.postings[0] = {account: "expenses:x", amount: "1", commodity: "", status: "unmarked", comment: "", cost: null};
        form.postings[1].account = "assets:y";
        const body = formToBody(form, "   ");
        expect(body.postings[0].amount).toEqual({commodity: "$", quantity: {mantissa: "1", places: 0}});
    });

    it("omits an unmarked status and empty optional fields, and forwards a position", () => {
        const form = blankForm("2026-07-20", "$");
        form.postings[0].account = "a:b";
        form.postings[0].amount = "1.00";
        form.postings[1].account = "c:d";
        const body = formToBody(form, "$", "dateOrdered");
        expect(body.status).toBeUndefined();
        expect(body.code).toBeUndefined();
        expect(body.description).toBeUndefined();
        expect(body.date2).toBeUndefined();
        expect(body.position).toBe("dateOrdered");
    });

    it("prefills date2, the full comment (tags included), and per-posting status + comment", () => {
        const t = txn(
            4,
            "Groceries",
            [
                posting("expenses:food", [amount("$", 500n, 2)], {status: "cleared", comment: "on sale"}),
                posting("assets:cash", [amount("$", -500n, 2)]),
            ],
            {date2: "2026-07-22", comment: "weekly shop, category:food"}
        );
        const form = txnToForm(t);
        expect(form.date2).toBe("2026-07-22");
        expect(form.comment).toBe("weekly shop, category:food");
        expect(form.postings[0]).toMatchObject({status: "cleared", comment: "on sale"});
        expect(form.postings[1]).toMatchObject({status: "unmarked", comment: ""});
    });

    it("defaults date2 to empty when the transaction has no secondary date", () => {
        const t = txn(1, "x", [posting("a:b", [])]);
        expect(txnToForm(t).date2).toBe("");
    });

    it("emits date2, the txn comment (tags), and per-posting status + comment", () => {
        const form = blankForm("2026-07-20", "$");
        form.date2 = "2026-07-22";
        form.comment = "note, category:food";
        form.postings[0] = {account: "expenses:food", amount: "5.00", commodity: "$", status: "cleared", comment: "on sale", cost: null};
        form.postings[1] = {account: "assets:cash", amount: "", commodity: "$", status: "unmarked", comment: "", cost: null};
        const body = formToBody(form, "$");
        expect(body.date2).toBe("2026-07-22");
        expect(body.comment).toBe("note, category:food");
        expect(body.postings[0]).toEqual({
            account: "expenses:food",
            status: "cleared",
            comment: "on sale",
            amount: {commodity: "$", quantity: {mantissa: "500", places: 2}},
        });
        expect(body.postings[1]).toEqual({account: "assets:cash"});
    });

    it("omits an empty date2/comment and an unmarked/blank posting status + comment", () => {
        const form = blankForm("2026-07-20", "$");
        form.date2 = "   ";
        form.comment = "";
        form.postings[0].account = "a:b";
        form.postings[0].comment = "  ";
        const body = formToBody(form, "$");
        expect(body.date2).toBeUndefined();
        expect(body.comment).toBeUndefined();
        expect(body.postings[0].status).toBeUndefined();
        expect(body.postings[0].comment).toBeUndefined();
    });
});

describe("UNIT editMapping — validateForm", () => {
    it("passes a dated form with at least one account", () => {
        const form = blankForm("2026-07-20", "$");
        form.postings[0].account = "expenses:x";
        expect(validateForm(form)).toEqual([]);
    });

    it("flags a missing/malformed date, no postings, and a bad amount", () => {
        const noDate = blankForm("", "$");
        expect(validateForm(noDate).some((m) => m.includes("date"))).toBe(true);

        const badDate = blankForm("2026/07/20", "$");
        badDate.postings[0].account = "a:b";
        expect(validateForm(badDate).some((m) => m.includes("YYYY-MM-DD"))).toBe(true);

        const noPostings = blankForm("2026-07-20", "$");
        expect(validateForm(noPostings).some((m) => m.includes("posting"))).toBe(true);

        const badAmount = blankForm("2026-07-20", "$");
        badAmount.postings[0].account = "a:b";
        badAmount.postings[0].amount = "12x";
        expect(validateForm(badAmount).some((m) => m.includes("valid amount"))).toBe(true);
    });
});

describe("UNIT editMapping — PATCH builders", () => {
    it("builds a description patch", () => {
        expect(descriptionPatch("New payee")).toEqual({description: "New payee"});
    });

    it("builds a status patch (the inline cleared/pending toggle)", () => {
        expect(statusPatch("cleared")).toEqual({status: "cleared"});
        expect(statusPatch("pending")).toEqual({status: "pending"});
        expect(statusPatch("unmarked")).toEqual({status: "unmarked"});
    });

    it("finds every posting position on an account", () => {
        const t = txn(1, "Split", [posting("expenses:a", [amount("$", 100n, 2)]), posting("expenses:a", [amount("$", 200n, 2)]), posting("assets:bank", [])]);
        expect(postingIndicesForAccount(t, "expenses:a")).toEqual([0, 1]);
        expect(postingIndicesForAccount(t, "assets:bank")).toEqual([2]);
        expect(postingIndicesForAccount(t, "nope")).toEqual([]);
    });

    it("recategorizes all postings on the old account to the new one", () => {
        const t = txn(1, "Groceries", [posting("expenses:food", [amount("$", 500n, 2)]), posting("assets:cash", [amount("$", -500n, 2)])]);
        expect(accountPatch(t, "expenses:food", "expenses:groceries")).toEqual({postings: [{index: 0, account: "expenses:groceries"}]});
    });

    it("yields no posting edits when the old account is absent (caller skips the request)", () => {
        const t = txn(1, "x", [posting("assets:cash", [])]);
        expect(accountPatch(t, "expenses:missing", "expenses:new")).toEqual({postings: []});
    });
});

describe("UNIT editMapping — dominantCommodity", () => {
    it("picks the most frequent commodity, defaulting to $ for an empty journal", () => {
        expect(dominantCommodity([])).toBe("$");
        const txns = [
            txn(1, "a", [posting("x", [amount("EUR", 1n, 0)]), posting("y", [amount("EUR", -1n, 0)])]),
            txn(2, "b", [posting("x", [amount("$", 1n, 0)])]),
        ];
        expect(dominantCommodity(txns)).toBe("EUR");
    });
});
