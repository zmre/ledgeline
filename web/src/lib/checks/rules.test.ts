import {describe, expect, it} from "vitest";
import {dec} from "../domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction, TxnStatus} from "../domain/types";
import {today} from "../reports/periods";
import {groupByTxn, maxSeverity, runChecks, ALL_RULES, type Problem} from "./engine";

const usdStyle: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: usdStyle});
const eur = (cents: number): Amount => ({commodity: "EUR", qty: dec(cents, 2), style: {...usdStyle, side: "R"}});
/** `qty` AAPL (4 dp) with a cost annotation in USD cents (`per` ⇒ `@`, else `@@`). */
const aapl = (tenThousandths: number, costCents: number, per: boolean): Amount => ({
    commodity: "AAPL",
    qty: dec(tenThousandths, 4),
    style: {...usdStyle, digitGroups: null, precision: 4},
    cost: {commodity: "$", qty: dec(costCents, 2), per},
});

interface PostingSpec {
    account: string;
    amounts: Amount[];
}

interface TxnSpec {
    description?: string;
    status?: TxnStatus;
    date?: ISODate;
}

function txn(index: number, postings: PostingSpec[], spec: TxnSpec = {}): Transaction {
    const full: Posting[] = postings.map((p) => ({account: p.account, amounts: p.amounts, status: "unmarked", comment: "", tags: []}));
    const description = spec.description ?? `txn ${index}`;
    return {
        index,
        date: spec.date ?? "2026-07-01",
        status: spec.status ?? "unmarked",
        description,
        code: "",
        comment: "",
        tags: [],
        postings: full,
        haystack: description.toLowerCase(),
    };
}

const byRule = (problems: Problem[], rule: string): Problem[] => problems.filter((p) => p.rule === rule);
const NO_PRICES = {prices: []};
const run1 = (t: Transaction, rule: string): Problem[] => byRule(runChecks([t], NO_PRICES), rule);

describe("UNIT checks/rules unbalanced", () => {
    it("accepts a fully amounted transaction that sums to zero per commodity", () => {
        const t = txn(1, [
            {account: "expenses:food", amounts: [usd(5624)]},
            {account: "liabilities:cc:visa", amounts: [usd(-5624)]},
        ]);
        expect(run1(t, "unbalanced")).toEqual([]);
    });

    it("accepts one elided (amountless) posting absorbing any remainder", () => {
        const t = txn(1, [
            {account: "expenses:housing:rent", amounts: [usd(187500)]},
            {account: "assets:bank:checking", amounts: []},
        ]);
        expect(run1(t, "unbalanced")).toEqual([]);
    });

    it("flags two or more amountless postings as an error", () => {
        const t = txn(7, [
            {account: "expenses:a", amounts: [usd(100)]},
            {account: "assets:x", amounts: []},
            {account: "assets:y", amounts: []},
        ]);
        const problems = run1(t, "unbalanced");
        expect(problems).toHaveLength(1);
        expect(problems[0]).toMatchObject({txnIndex: 7, severity: "error"});
        expect(problems[0].message).toContain("2 postings have no amount");
    });

    it("flags a nonzero residue, naming the commodity and remainder", () => {
        const t = txn(3, [
            {account: "expenses:food", amounts: [usd(5000)]},
            {account: "assets:bank:checking", amounts: [usd(-4750)]},
        ]);
        const problems = run1(t, "unbalanced");
        expect(problems).toHaveLength(1);
        expect(problems[0].severity).toBe("error");
        expect(problems[0].message).toContain("$ 2.50 remaining");
    });

    it("treats commodities independently — two self-balancing commodities are fine", () => {
        const t = txn(4, [
            {account: "a", amounts: [usd(100), eur(200)]},
            {account: "b", amounts: [usd(-100), eur(-200)]},
        ]);
        expect(run1(t, "unbalanced")).toEqual([]);
    });

    it("flags when one commodity balances but another does not", () => {
        const t = txn(5, [
            {account: "a", amounts: [usd(100), eur(200)]},
            {account: "b", amounts: [usd(-100), eur(-150)]},
        ]);
        const problems = run1(t, "unbalanced");
        expect(problems).toHaveLength(1);
        expect(problems[0].message).toContain("EUR 0.50");
    });

    it("balances @ per-unit costs in the cost commodity (10 AAPL @ $220.00 vs $-2200.00)", () => {
        const t = txn(6, [
            {account: "assets:broker:aapl", amounts: [aapl(100000, 22000, true)]},
            {account: "assets:broker:cash", amounts: [usd(-220000)]},
        ]);
        expect(run1(t, "unbalanced")).toEqual([]);
    });

    it("balances @@ total costs with the posting amount's sign", () => {
        const buy = txn(8, [
            {account: "assets:broker:aapl", amounts: [aapl(45000, 111735, false)]}, // 4.5 AAPL @@ $1117.35
            {account: "assets:broker:cash", amounts: [usd(-111735)]},
        ]);
        const sell = txn(9, [
            {account: "assets:broker:aapl", amounts: [aapl(-45000, 111735, false)]},
            {account: "assets:broker:cash", amounts: [usd(111735)]},
        ]);
        expect(byRule(runChecks([buy, sell], NO_PRICES), "unbalanced")).toEqual([]);
    });

    it("flags a cost-converted residue", () => {
        const t = txn(10, [
            {account: "assets:broker:aapl", amounts: [aapl(100000, 22000, true)]},
            {account: "assets:broker:cash", amounts: [usd(-219900)]}, // $1.00 short
        ]);
        expect(run1(t, "unbalanced")).toHaveLength(1);
    });

    it("ignores a postingless transaction (nothing to balance)", () => {
        expect(run1(txn(11, []), "unbalanced")).toEqual([]);
    });
});

describe("UNIT checks/rules pending", () => {
    it("flags pending transactions as warnings; cleared/unmarked pass", () => {
        const problems = byRule(
            runChecks([txn(1, [], {status: "pending"}), txn(2, [], {status: "cleared"}), txn(3, [], {status: "unmarked"})], NO_PRICES),
            "pending"
        );
        expect(problems).toEqual([{txnIndex: 1, rule: "pending", severity: "warning", message: "transaction is marked pending (!)"}]);
    });
});

describe("UNIT checks/rules uncategorized", () => {
    const post = (account: string): PostingSpec => ({account, amounts: [usd(100)]});

    it.each(["expenses:unknown", "assets:uncategorized", "expenses:misc:Unknown", "income"])("flags %s as a warning", (account) => {
        const problems = run1(txn(1, [post(account)]), "uncategorized");
        expect(problems).toHaveLength(1);
        expect(problems[0]).toMatchObject({severity: "warning", message: `posting to uncategorized account "${account}"`});
    });

    it.each(["expenses:food:groceries", "income:salary", "expenses:unknowable", "assets:bank:checking"])("passes %s", (account) => {
        expect(run1(txn(1, [post(account)]), "uncategorized")).toEqual([]);
    });

    it("flags a bare top-level expenses posting", () => {
        expect(run1(txn(2, [post("expenses")]), "uncategorized")).toHaveLength(1);
    });

    it("reports each offending account once per transaction", () => {
        const t = txn(3, [post("expenses:unknown"), post("expenses:unknown"), post("income")]);
        const problems = run1(t, "uncategorized");
        expect(problems.map((p) => p.message)).toEqual(['posting to uncategorized account "expenses:unknown"', 'posting to uncategorized account "income"']);
    });
});

describe("UNIT checks/rules missing-description", () => {
    it("flags empty and whitespace-only descriptions as info", () => {
        const problems = byRule(
            runChecks([txn(1, [], {description: ""}), txn(2, [], {description: "   "}), txn(3, [], {description: "rent"})], NO_PRICES),
            "missing-description"
        );
        expect(problems.map((p) => p.txnIndex)).toEqual([1, 2]);
        expect(problems.every((p) => p.severity === "info")).toBe(true);
    });
});

describe("UNIT checks/rules future-date", () => {
    it("flags dates strictly after today as info; today and the past pass", () => {
        const problems = byRule(
            runChecks([txn(1, [], {date: "9999-12-31"}), txn(2, [], {date: today()}), txn(3, [], {date: "1970-01-01"})], NO_PRICES),
            "future-date"
        );
        expect(problems).toEqual([{txnIndex: 1, rule: "future-date", severity: "info", message: "transaction is dated in the future (9999-12-31)"}]);
    });
});

describe("UNIT checks/engine", () => {
    it("runChecks defaults to ALL_RULES and honors an explicit subset", () => {
        const t = txn(1, [{account: "expenses:unknown", amounts: [usd(100)]}], {status: "pending", description: ""});
        // one elided-less residue-free txn: postings sum to +$1.00 → also unbalanced
        const all = runChecks([t], NO_PRICES);
        expect(new Set(all.map((p) => p.rule))).toEqual(new Set(["unbalanced", "pending", "uncategorized", "missing-description"]));
        const onlyPending = runChecks([t], NO_PRICES, [ALL_RULES[1]]);
        expect(onlyPending.map((p) => p.rule)).toEqual(["pending"]);
    });

    it("maxSeverity picks the worst level; null when clean", () => {
        expect(maxSeverity([])).toBeNull();
        const info: Problem = {txnIndex: 1, rule: "r", severity: "info", message: ""};
        const warning: Problem = {...info, severity: "warning"};
        const error: Problem = {...info, severity: "error"};
        expect(maxSeverity([info])).toBe("info");
        expect(maxSeverity([info, warning])).toBe("warning");
        expect(maxSeverity([warning, error, info])).toBe("error");
    });

    it("groupByTxn buckets problems by transaction index in order", () => {
        const p = (txnIndex: number, rule: string): Problem => ({txnIndex, rule, severity: "info", message: ""});
        const grouped = groupByTxn([p(1, "a"), p(2, "b"), p(1, "c")]);
        expect(grouped.get(1)?.map((x) => x.rule)).toEqual(["a", "c"]);
        expect(grouped.get(2)?.map((x) => x.rule)).toEqual(["b"]);
        expect(grouped.has(3)).toBe(false);
    });
});
