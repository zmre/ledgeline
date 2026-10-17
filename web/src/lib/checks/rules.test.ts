import {describe, expect, it} from "vitest";
import {classesOf} from "../domain/accountClassFixtures";
import {dec} from "../domain/money";
import type {Amount, AmountStyle, ISODate, Posting, PostingType, Transaction, TxnStatus} from "../domain/types";
import {today} from "../reports/periods";
import {groupByTxn, maxSeverity, runChecks, ALL_RULES, type Problem} from "./engine";

const usdStyle: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: usdStyle});
const eur = (cents: number): Amount => ({commodity: "EUR", qty: dec(cents, 2), style: {...usdStyle, side: "R"}});

interface PostingSpec {
    account: string;
    amounts: Amount[];
    /** hledger's `ptype`; omitted = `"regular"`, exactly as the normalizer emits it. */
    type?: PostingType;
}

interface TxnSpec {
    description?: string;
    status?: TxnStatus;
    date?: ISODate;
}

function txn(index: number, postings: PostingSpec[], spec: TxnSpec = {}): Transaction {
    const full: Posting[] = postings.map((p) => {
        const posting: Posting = {account: p.account, amounts: p.amounts, status: "unmarked", comment: "", tags: []};
        if (p.type !== undefined) posting.type = p.type;
        return posting;
    });
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

describe("UNIT checks/rules negative-cash", () => {
    const classes = classesOf(["assets:bank:checking", "cash"], ["assets:broker:taxable", "asset"], ["liabilities:card", "liability"], {
        name: "liabilities:mortgage",
        type: "liability",
        term: "noncurrent",
    });
    const ctx = {prices: [], classes};
    /** Money out of `account` and into an expense, dated in the past so `asOf` (today) includes it. */
    const spend = (index: number, account: string, cents: number): Transaction =>
        txn(
            index,
            [
                {account: "expenses:food", amounts: [usd(cents)]},
                {account, amounts: [usd(-cents)]},
            ],
            {date: "2026-01-01"}
        );

    it("flags a cash account that has run negative, anchored to the ACCOUNT", () => {
        const problems = byRule(runChecks([spend(1, "assets:bank:checking", 5000)], ctx), "negative-cash");
        expect(problems).toHaveLength(1);
        expect(problems[0]).toMatchObject({
            txnIndex: null,
            account: "assets:bank:checking",
            severity: "warning",
            message: "cash balance is negative: $ -50.00 as of 2026-01-01",
        });
    });

    it("says nothing when the account was funded first", () => {
        const funded = txn(
            1,
            [
                {account: "assets:bank:checking", amounts: [usd(100_000)]},
                {account: "equity:opening", amounts: [usd(-100_000)]},
            ],
            {date: "2025-12-01"}
        );
        expect(byRule(runChecks([funded, spend(2, "assets:bank:checking", 5000)], ctx), "negative-cash")).toEqual([]);
    });

    it("leaves liabilities alone — a negative balance is what owing money looks like", () => {
        expect(byRule(runChecks([spend(1, "liabilities:card", 5000)], ctx), "negative-cash")).toEqual([]);
    });

    it("ignores non-cash assets, whose sign is the holdings engine's business", () => {
        expect(byRule(runChecks([spend(1, "assets:broker:taxable", 5000)], ctx), "negative-cash")).toEqual([]);
    });

    it("reports each commodity separately", () => {
        const mixed = txn(
            1,
            [
                {account: "assets:bank:checking", amounts: [usd(-5000), eur(-2500)]},
                {account: "expenses:food", amounts: [usd(5000), eur(2500)]},
            ],
            {date: "2026-01-01"}
        );
        const problems = byRule(runChecks([mixed], ctx), "negative-cash");
        expect(problems.map((p) => p.message)).toEqual([
            "cash balance is negative: $ -50.00 as of 2026-01-01",
            "cash balance is negative: EUR -25.00 as of 2026-01-01",
        ]);
    });

    it("matches the Balances view's scope: a future-dated overdraft is not one yet", () => {
        expect(byRule(runChecks([spend(1, "assets:bank:checking", 5000)], ctx), "negative-cash")).toHaveLength(1);
        const later = txn(
            2,
            [
                {account: "expenses:food", amounts: [usd(5000)]},
                {account: "assets:bank:checking", amounts: [usd(-5000)]},
            ],
            {date: "9999-12-31"}
        );
        expect(byRule(runChecks([later], ctx), "negative-cash")).toEqual([]);
    });

    it("falls back to hledger's name heuristic when the journal declares nothing", () => {
        const problems = byRule(runChecks([spend(1, "assets:bank:checking", 5000)], NO_PRICES), "negative-cash");
        expect(problems.map((p) => p.account)).toEqual(["assets:bank:checking"]);
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
        const all = runChecks([t], NO_PRICES);
        expect(new Set(all.map((p) => p.rule))).toEqual(new Set(["pending", "uncategorized", "missing-description"]));
        const onlyPending = runChecks([t], NO_PRICES, [ALL_RULES[0]]);
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
