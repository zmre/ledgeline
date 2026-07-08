import {describe, expect, it} from "vitest";
import type {ISODate, Posting, Transaction} from "$lib/domain/types";
import {contentFingerprint} from "./journal.svelte";

function posting(account: string): Posting {
    return {account, amounts: [], status: "unmarked", comment: "", tags: []};
}

function txn(index: number, date: ISODate, haystack: string): Transaction {
    return {index, date, status: "cleared", description: "t", code: "", comment: "", tags: [], postings: [posting("expenses:food")], haystack};
}

describe("UNIT journal contentFingerprint", () => {
    const base = [txn(1, "2026-01-05", "groceries $100.00 expenses:food"), txn(2, "2026-07-05", "rent $1,800.00 expenses:housing")];

    it("is stable for identical content", () => {
        const same = [txn(1, "2026-01-05", "groceries $100.00 expenses:food"), txn(2, "2026-07-05", "rent $1,800.00 expenses:housing")];
        expect(contentFingerprint(same, ["expenses"], [])).toBe(contentFingerprint(base, ["expenses"], []));
    });

    it("changes when a MID-LIST transaction is edited in place (count and last txn unchanged)", () => {
        const edited = [txn(1, "2026-01-05", "groceries $999.00 expenses:food"), txn(2, "2026-07-05", "rent $1,800.00 expenses:housing")];
        expect(contentFingerprint(edited, ["expenses"], [])).not.toBe(contentFingerprint(base, ["expenses"], []));
    });

    it("changes when a txn date or status changes", () => {
        const redated = [txn(1, "2026-01-06", "groceries $100.00 expenses:food"), base[1]];
        expect(contentFingerprint(redated, ["expenses"], [])).not.toBe(contentFingerprint(base, ["expenses"], []));
    });

    it("changes when account names or prices change", () => {
        expect(contentFingerprint(base, ["expenses", "assets"], [])).not.toBe(contentFingerprint(base, ["expenses"], []));
        const style = {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};
        const price = {date: "2026-07-01", commodity: "EUR", price: {commodity: "$", qty: {m: 117n, p: 2}, style}};
        expect(contentFingerprint(base, ["expenses"], [price])).not.toBe(contentFingerprint(base, ["expenses"], []));
    });
});
