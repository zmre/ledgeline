import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction, TxnStatus} from "$lib/domain/types";
import type {JournalFilter} from "$lib/stores/filters.svelte";
import {
    accountFlow,
    commodityStyles,
    computeWindow,
    filteredTotals,
    filterTxns,
    formatTotals,
    periodLabel,
    sortTxnsDesc,
    txnComments,
    txnFlowAmounts,
} from "./rowModel";

const usdStyle: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const eurStyle: AmountStyle = {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: null};

const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: usdStyle});
const eur = (cents: number): Amount => ({commodity: "EUR", qty: dec(cents, 2), style: eurStyle});

interface PostingSpec {
    account: string;
    amounts: Amount[];
    comment?: string;
}

interface TxnSpec {
    description?: string;
    status?: TxnStatus;
    comment?: string;
}

function txn(index: number, date: ISODate, postings: PostingSpec[], spec: TxnSpec = {}): Transaction {
    const full: Posting[] = postings.map((p) => ({account: p.account, amounts: p.amounts, status: "unmarked", comment: p.comment ?? "", tags: []}));
    const description = spec.description ?? `txn ${index}`;
    const haystack = [description, spec.comment ?? "", ...postings.flatMap((p) => [p.account, p.comment ?? ""])]
        .filter((part) => part !== "")
        .join("\n")
        .toLowerCase();
    return {
        index,
        date,
        status: spec.status ?? "unmarked",
        description,
        code: "",
        comment: spec.comment ?? "",
        tags: [],
        postings: full,
        haystack,
    };
}

const filter = (over: Partial<JournalFilter> = {}): JournalFilter => ({from: null, to: null, accounts: new Set<string>(), query: "", ...over});

const groceries = txn(
    1,
    "2026-07-03",
    [
        {account: "expenses:food:groceries", amounts: [usd(5624)]},
        {account: "liabilities:cc:visa", amounts: [usd(-5624)]},
    ],
    {description: "Safeway | weekly groceries"}
);
const rent = txn(
    2,
    "2026-07-01",
    [
        {account: "expenses:housing:rent", amounts: [usd(187500)]},
        {account: "assets:bank:checking", amounts: [usd(-187500)]},
    ],
    {description: "Oakview Properties | rent", comment: "auto-pay"}
);
const paycheck = txn(
    3,
    "2026-06-30",
    [
        {account: "assets:bank:checking", amounts: [usd(320000)]},
        {account: "assets:retirement:401k", amounts: [usd(40000)]},
        {account: "income:salary", amounts: [usd(-360000)]},
    ],
    {description: "Acme Corp | paycheck"}
);
const sample = [groceries, rent, paycheck];

describe("UNIT rowModel", () => {
    describe("filterTxns", () => {
        it("applies the date range inclusively on both ends against txn.date", () => {
            const hit = filterTxns(sample, filter({from: "2026-07-01", to: "2026-07-03"}));
            expect(hit.map((t) => t.index).sort()).toEqual([1, 2]);
            // boundary dates are included
            expect(filterTxns(sample, filter({from: "2026-06-30", to: "2026-06-30"})).map((t) => t.index)).toEqual([3]);
            expect(filterTxns(sample, filter({from: "2026-07-02", to: null})).map((t) => t.index)).toEqual([1]);
            expect(filterTxns(sample, filter({from: null, to: "2026-06-29"}))).toEqual([]);
        });

        it("matches a txn when ANY posting matches ANY selected account, including subtrees", () => {
            const bank = filterTxns(sample, filter({accounts: new Set(["assets:bank"])}));
            expect(bank.map((t) => t.index).sort()).toEqual([2, 3]);
            // exact account name and prefix-with-colon both match; string prefix alone does not
            expect(filterTxns(sample, filter({accounts: new Set(["assets:bank:check"])}))).toEqual([]);
            expect(filterTxns(sample, filter({accounts: new Set(["income:salary"])})).map((t) => t.index)).toEqual([3]);
        });

        it("treats an empty account selection as all accounts", () => {
            expect(filterTxns(sample, filter())).toHaveLength(3);
        });

        it("matches the query case-insensitively against the haystack", () => {
            expect(filterTxns(sample, filter({query: "SAFEWAY"})).map((t) => t.index)).toEqual([1]);
            expect(filterTxns(sample, filter({query: "  auto-pay "})).map((t) => t.index)).toEqual([2]);
            expect(filterTxns(sample, filter({query: "no such thing"}))).toEqual([]);
            expect(filterTxns(sample, filter({query: "   "}))).toHaveLength(3);
        });

        it("combines date, account, and query criteria", () => {
            const hit = filterTxns(sample, filter({from: "2026-07-01", to: "2026-07-31", accounts: new Set(["expenses"]), query: "rent"}));
            expect(hit.map((t) => t.index)).toEqual([2]);
        });
    });

    describe("filteredTotals", () => {
        it("sums all postings when the selection is empty (balanced txns net to zero)", () => {
            expect(filteredTotals(sample, new Set()).size).toBe(0);
        });

        it("sums only postings in the selected accounts (subtree match)", () => {
            const totals = filteredTotals(sample, new Set(["expenses"]));
            expect(totals.get("$")?.m).toBe(5624n + 187500n);
        });

        it("keeps commodities separate and drops zero entries", () => {
            const fx = txn(9, "2026-07-04", [
                {account: "assets:cash:usd", amounts: [usd(-10000)]},
                {account: "assets:cash:eur", amounts: [eur(9000)]},
            ]);
            const totals = filteredTotals([fx], new Set(["assets:cash"]));
            expect(totals.get("$")?.m).toBe(-10000n);
            expect(totals.get("EUR")?.m).toBe(9000n);
            const both = filteredTotals([groceries], new Set(["expenses", "liabilities"]));
            expect(both.size).toBe(0); // +5624 and -5624 cancel
        });
    });

    describe("sortTxnsDesc", () => {
        it("sorts by date descending, then index descending, without mutating the input", () => {
            const shuffled = [paycheck, groceries, rent, txn(4, "2026-07-01", [{account: "expenses:misc", amounts: [usd(100)]}])];
            const sorted = sortTxnsDesc(shuffled);
            expect(sorted.map((t) => t.index)).toEqual([1, 4, 2, 3]);
            expect(shuffled.map((t) => t.index)).toEqual([3, 1, 2, 4]);
        });
    });

    describe("accountFlow", () => {
        it("renders source → dest for a simple 2-posting txn", () => {
            expect(accountFlow(groceries)).toEqual({kind: "flow", source: "liabilities:cc:visa", dest: "expenses:food:groceries"});
        });

        it("degrades to a plain account list for N-way splits (>2 distinct sides)", () => {
            expect(accountFlow(paycheck)).toEqual({kind: "list", accounts: ["assets:bank:checking", "assets:retirement:401k", "income:salary"]});
        });

        it("still arrows a multi-commodity exchange with one source and one destination", () => {
            const fx = txn(10, "2026-07-05", [
                {account: "assets:cash:usd", amounts: [usd(-10000)]},
                {account: "assets:cash:eur", amounts: [eur(9000)]},
            ]);
            expect(accountFlow(fx)).toEqual({kind: "flow", source: "assets:cash:usd", dest: "assets:cash:eur"});
        });

        it("degrades when a posting's commodities disagree in sign", () => {
            const mixed = txn(11, "2026-07-05", [
                {account: "assets:broker", amounts: [usd(-10000), eur(9000)]},
                {account: "expenses:fees", amounts: [usd(10000), eur(-9000)]},
            ]);
            expect(accountFlow(mixed).kind).toBe("list");
        });

        it("ignores zero-net postings when picking sides and dedupes repeated accounts", () => {
            const zeroed = txn(12, "2026-07-06", [
                {account: "expenses:food", amounts: [usd(500)]},
                {account: "expenses:food", amounts: [usd(700)]},
                {account: "assets:cash", amounts: [usd(-1200)]},
                {account: "equity:rounding", amounts: []},
            ]);
            expect(accountFlow(zeroed)).toEqual({kind: "flow", source: "assets:cash", dest: "expenses:food"});
        });
    });

    describe("txnFlowAmounts", () => {
        it("shows the positive (destination) side per commodity", () => {
            const amounts = txnFlowAmounts(groceries);
            expect(amounts).toHaveLength(1);
            expect(amounts[0].commodity).toBe("$");
            expect(amounts[0].qty.m).toBe(5624n);
        });

        it("sums multiple destinations and keeps commodities on separate lines", () => {
            const amounts = txnFlowAmounts(paycheck);
            expect(amounts[0].qty.m).toBe(360000n);
            const fx = txn(13, "2026-07-05", [
                {account: "assets:cash:usd", amounts: [usd(-10000)]},
                {account: "assets:cash:eur", amounts: [eur(9000)]},
            ]);
            expect(txnFlowAmounts(fx).map((a) => a.commodity)).toEqual(["EUR"]);
        });

        it("falls back to |negatives| when nothing is positive, and [] for zero txns", () => {
            const negOnly = txn(14, "2026-07-05", [{account: "assets:cash", amounts: [usd(-500)]}]);
            expect(txnFlowAmounts(negOnly)[0].qty.m).toBe(500n);
            const empty = txn(15, "2026-07-05", [{account: "assets:cash", amounts: []}]);
            expect(txnFlowAmounts(empty)).toEqual([]);
        });
    });

    describe("txnComments", () => {
        it("collects the txn comment and account-prefixed posting comments", () => {
            const commented = txn(16, "2026-07-05", [{account: "expenses:misc", amounts: [usd(100)], comment: "see receipt"}], {comment: "quarterly true-up"});
            expect(txnComments(commented)).toEqual(["quarterly true-up", "expenses:misc: see receipt"]);
            expect(txnComments(groceries)).toEqual([]);
        });
    });

    describe("formatTotals / commodityStyles", () => {
        it("formats per-commodity lines using the journal's own styles, sorted by commodity", () => {
            const fx = txn(20, "2026-07-05", [
                {account: "assets:cash:eur", amounts: [eur(9000)]},
                {account: "assets:cash:usd", amounts: [usd(-10000)]},
            ]);
            const styles = commodityStyles([...sample, fx]);
            expect(styles.get("$")).toBe(usdStyle);
            expect(styles.get("EUR")).toBe(eurStyle);
            const totals: MixedAmount = new Map([
                ["EUR", dec(-4500, 2)],
                ["$", dec(123456, 2)],
            ]);
            expect(formatTotals(totals, styles)).toEqual([
                {text: "$1,234.56", negative: false},
                {text: "-45,00 EUR", negative: true},
            ]);
        });

        it("falls back to a neutral style for commodities never seen in the journal", () => {
            const lines = formatTotals(new Map([["BTC", dec(5, 1)]]), new Map());
            expect(lines).toEqual([{text: "0.5 BTC", negative: false}]);
        });
    });

    describe("periodLabel", () => {
        it("labels closed, open-ended, and unbounded ranges", () => {
            expect(periodLabel("2026-07-01", "2026-07-31")).toBe("2026-07-01 – 2026-07-31");
            expect(periodLabel("2026-07-01", null)).toBe("from 2026-07-01");
            expect(periodLabel(null, "2026-07-31")).toBe("through 2026-07-31");
            expect(periodLabel(null, null)).toBe("all dates");
        });
    });

    describe("computeWindow", () => {
        it("covers the viewport plus overscan and clamps at both ends", () => {
            const top = computeWindow(0, 800, 40, 1000, 12);
            expect(top.start).toBe(0);
            expect(top.end).toBe(Math.ceil(800 / 40) + 12);
            expect(top.padTop).toBe(0);
            const mid = computeWindow(4000, 800, 40, 1000, 12);
            expect(mid.start).toBe(100 - 12);
            expect(mid.end).toBe(120 + 12);
            const bottom = computeWindow(1000 * 40, 800, 40, 1000, 12);
            expect(bottom.end).toBe(1000);
            expect(bottom.padBottom).toBe(0);
            expect(computeWindow(0, 800, 40, 0, 12)).toEqual({start: 0, end: 0, padTop: 0, padBottom: 0});
        });

        it("preserves total scroll height: padTop + rendered*pitch + padBottom === total*pitch", () => {
            for (const scrollTop of [0, 37, 4000, 123456, 39999 * 40]) {
                const w = computeWindow(scrollTop, 800, 40, 40000, 12);
                expect(w.padTop + (w.end - w.start) * 40 + w.padBottom).toBe(40000 * 40);
            }
        });
    });

    describe("50k synthetic dataset", () => {
        const big: Transaction[] = Array.from({length: 50_000}, (_, i) => {
            const year = 2020 + (i % 6);
            const month = 1 + (i % 12);
            const day = 1 + (i % 28);
            const pad = (n: number): string => String(n).padStart(2, "0");
            return txn(
                i + 1,
                `${year}-${pad(month)}-${pad(day)}`,
                [
                    {account: `expenses:cat${i % 40}:sub${i % 7}`, amounts: [usd(100 + (i % 9000))]},
                    {account: i % 2 === 0 ? "assets:bank:checking" : "liabilities:cc:visa", amounts: [usd(-(100 + (i % 9000)))]},
                ],
                {description: `synthetic vendor ${i % 500}`}
            );
        });

        it("filters + sorts 50k txns quickly (full-scan cost, paid once per filter change)", () => {
            const start = performance.now();
            const sorted = sortTxnsDesc(filterTxns(big, filter()));
            const elapsed = performance.now() - start;
            expect(sorted).toHaveLength(50_000);
            expect(sorted[0].date >= sorted[sorted.length - 1].date).toBe(true);
            expect(elapsed).toBeLessThan(2000); // typically well under 200ms; generous bound for CI noise
        });

        it("keeps the rendered-row count independent of total row count at any scroll offset", () => {
            const viewport = 800;
            const pitch = 40;
            const overscan = 12;
            const maxRendered = Math.ceil(viewport / pitch) + 1 + 2 * overscan;
            for (const scrollTop of [0, 1, 999, 100_000, 1_234_567, 50_000 * pitch - viewport, 50_000 * pitch]) {
                const w = computeWindow(scrollTop, viewport, pitch, big.length, overscan);
                expect(w.end - w.start).toBeLessThanOrEqual(maxRendered);
                expect(w.padTop + (w.end - w.start) * pitch + w.padBottom).toBe(big.length * pitch);
            }
        });
    });
});
