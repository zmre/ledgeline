import {describe, expect, it} from "vitest";
import {accountTotals, atDepth, rollUp} from "./aggregate";
import {dec, type MixedAmount} from "./money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction, TxnStatus} from "./types";

const defaultStyle: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: defaultStyle});
const eur = (cents: number): Amount => ({commodity: "EUR", qty: dec(cents, 2), style: defaultStyle});

interface PostingSpec {
    account: string;
    amounts: Amount[];
    status?: TxnStatus;
    date?: ISODate;
}

function txn(index: number, date: ISODate, postings: PostingSpec[], status: TxnStatus = "unmarked"): Transaction {
    const full: Posting[] = postings.map((p) => ({
        account: p.account,
        amounts: p.amounts,
        status: p.status ?? "unmarked",
        comment: "",
        tags: [],
        ...(p.date !== undefined ? {date: p.date} : {}),
    }));
    return {index, date, status, description: `txn ${index}`, code: "", comment: "", tags: [], postings: full, haystack: ""};
}

const sample: Transaction[] = [
    txn(
        1,
        "2026-01-05",
        [
            {account: "expenses:food:groceries", amounts: [usd(8720)]},
            {account: "assets:bank:checking", amounts: [usd(-8720)]},
        ],
        "cleared"
    ),
    txn(2, "2026-01-20", [
        {account: "expenses:food:dining", amounts: [eur(4500)]},
        {account: "liabilities:card", amounts: [usd(-4860)]},
    ]),
    txn(
        3,
        "2026-02-02",
        [
            {account: "expenses:food:groceries", amounts: [usd(1280)], status: "pending"},
            {account: "assets:bank:checking", amounts: [usd(-1280)], date: "2026-02-03"},
        ],
        "cleared"
    ),
];

const cents = (totals: Map<string, MixedAmount>, account: string, commodity: string): bigint | undefined => totals.get(account)?.get(commodity)?.m;

describe("UNIT aggregate", () => {
    describe("accountTotals", () => {
        it("sums per full account name across all transactions", () => {
            const totals = accountTotals(sample);
            expect(cents(totals, "expenses:food:groceries", "$")).toBe(10000n);
            expect(cents(totals, "assets:bank:checking", "$")).toBe(-10000n);
            expect(cents(totals, "expenses:food:dining", "EUR")).toBe(4500n);
            expect(cents(totals, "liabilities:card", "$")).toBe(-4860n);
        });

        it("applies the inclusive date range to the posting's effective date", () => {
            const totals = accountTotals(sample, {from: "2026-01-05", to: "2026-02-02"});
            // txn 3's checking posting has its own pdate 2026-02-03 → excluded
            expect(cents(totals, "assets:bank:checking", "$")).toBe(-8720n);
            // but txn 3's groceries posting (txn date 2026-02-02) is included
            expect(cents(totals, "expenses:food:groceries", "$")).toBe(10000n);
        });

        it("filters by selected accounts including sub-accounts", () => {
            const totals = accountTotals(sample, {accounts: ["expenses:food"]});
            expect(totals.has("assets:bank:checking")).toBe(false);
            expect(totals.has("liabilities:card")).toBe(false);
            expect(cents(totals, "expenses:food:groceries", "$")).toBe(10000n);
            expect(cents(totals, "expenses:food:dining", "EUR")).toBe(4500n);
        });

        it("treats an empty accounts array as no account filter", () => {
            const totals = accountTotals(sample, {accounts: []});
            expect(totals.size).toBe(4);
        });

        it("matches status with posting-level override falling back to the transaction", () => {
            const cleared = accountTotals(sample, {status: "cleared"});
            // txn 3's groceries posting is explicitly pending, so only txn 1's postings + txn 3's checking posting are cleared
            expect(cents(cleared, "expenses:food:groceries", "$")).toBe(8720n);
            expect(cents(cleared, "assets:bank:checking", "$")).toBe(-10000n);
            const pending = accountTotals(sample, {status: "pending"});
            expect(cents(pending, "expenses:food:groceries", "$")).toBe(1280n);
        });

        it("drops commodities that sum to zero", () => {
            const zeroing = [
                txn(9, "2026-03-01", [
                    {account: "assets:cash", amounts: [usd(500)]},
                    {account: "assets:cash", amounts: [usd(-500), eur(100)]},
                ]),
            ];
            const totals = accountTotals(zeroing);
            expect(totals.get("assets:cash")?.has("$")).toBe(false);
            expect(cents(totals, "assets:cash", "EUR")).toBe(100n);
        });
    });

    describe("rollUp", () => {
        it("adds each account into itself and every ancestor", () => {
            const rolled = rollUp(accountTotals(sample));
            expect(cents(rolled, "expenses", "$")).toBe(10000n);
            expect(cents(rolled, "expenses", "EUR")).toBe(4500n);
            expect(cents(rolled, "expenses:food", "$")).toBe(10000n);
            expect(cents(rolled, "expenses:food:groceries", "$")).toBe(10000n);
            expect(cents(rolled, "assets", "$")).toBe(-10000n);
            expect(cents(rolled, "assets:bank", "$")).toBe(-10000n);
        });
    });

    describe("atDepth", () => {
        it("keeps only accounts with at most `depth` segments", () => {
            const rolled = rollUp(accountTotals(sample));
            const depth1 = atDepth(rolled, 1);
            expect([...depth1.keys()].sort()).toEqual(["assets", "expenses", "liabilities"]);
            const depth2 = atDepth(rolled, 2);
            expect(depth2.has("expenses:food")).toBe(true);
            expect(depth2.has("expenses:food:groceries")).toBe(false);
        });
    });
});
