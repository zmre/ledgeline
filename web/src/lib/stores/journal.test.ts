import {describe, expect, it} from "vitest";
import type {Dec} from "$lib/domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction} from "$lib/domain/types";
import {contentFingerprint} from "./journal.svelte";

function posting(account: string): Posting {
    return {account, amounts: [], status: "unmarked", comment: "", tags: []};
}

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

function amount(commodity: string, m: bigint, p: number): Amount {
    return {commodity, qty: {m, p} as Dec, style: STYLE};
}

/** A txn whose single posting carries `amounts`, with a caller-supplied haystack. */
function txnWithAmounts(haystack: string, amounts: Amount[]): Transaction {
    return {
        index: 1,
        date: "2026-06-29",
        status: "cleared",
        description: "Dividend",
        code: "",
        comment: "",
        tags: [],
        postings: [{account: "expenses:fees:broker", amounts, status: "unmarked", comment: "", tags: []}],
        haystack,
    };
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

    it("changes when an amount moves BELOW the 2-place display cap", () => {
        // A real case: clearing a $-0.16 broker fee on a dividend reinvest lets
        // the engine infer $-0.16343. The haystack is built for SEARCH, so its
        // amounts are rounded to 2 places and both render "$-0.16" — identical
        // text. If the fingerprint trusted only that, the post-edit refresh
        // would fetch the corrected transaction and then throw it away, leaving
        // the old amount on screen and the "unbalanced" problem in the badge.
        const haystack = "dividend\nexpenses:fees:broker\n$-0.16\n$";
        const rounded = txnWithAmounts(haystack, [amount("$", -16n, 2)]);
        const exact = txnWithAmounts(haystack, [amount("$", -16343n, 5)]);
        expect(rounded.haystack).toBe(exact.haystack); // the trap this guards
        expect(contentFingerprint([exact], [], [])).not.toBe(contentFingerprint([rounded], [], []));
    });

    it("changes when a share count differs past the 2-place display cap", () => {
        // Same trap for quantities: 15.244 and 15.245 shares both display "15.24".
        const haystack = "reinvest\nassets:broker\n15.24 VWEHX\nvwehx";
        const a = txnWithAmounts(haystack, [amount("VWEHX", 15244n, 3)]);
        const b = txnWithAmounts(haystack, [amount("VWEHX", 15245n, 3)]);
        expect(contentFingerprint([a], [], [])).not.toBe(contentFingerprint([b], [], []));
    });

    it("changes when only a cost annotation changes", () => {
        const haystack = "reinvest\nassets:broker\n0.29 VFIAX\nvfiax";
        const withCost = (costM: bigint): Transaction =>
            txnWithAmounts(haystack, [{...amount("VFIAX", 293n, 3), cost: {commodity: "$", qty: {m: costM, p: 2} as Dec, per: true}}]);
        expect(contentFingerprint([withCost(67851n)], [], [])).not.toBe(contentFingerprint([withCost(67852n)], [], []));
    });

    it("is still stable when nothing changed, amounts included", () => {
        const make = (): Transaction => txnWithAmounts("dividend\nexpenses:fees:broker\n$-0.16\n$", [amount("$", -16343n, 5)]);
        expect(contentFingerprint([make()], [], [])).toBe(contentFingerprint([make()], [], []));
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

    it("changes when a declared account type changes (cash-flow must recompute)", () => {
        const asAsset = [{name: "assets:bank:checking", type: "asset" as const}];
        const asCash = [{name: "assets:bank:checking", type: "cash" as const}];
        expect(contentFingerprint(base, ["expenses"], [], asCash)).not.toBe(contentFingerprint(base, ["expenses"], [], asAsset));
        // The default (no decls) stays backward-compatible with the 3-arg call.
        expect(contentFingerprint(base, ["expenses"], [])).toBe(contentFingerprint(base, ["expenses"], [], []));
    });
});
