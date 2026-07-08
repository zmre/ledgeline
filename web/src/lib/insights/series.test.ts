import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {normalizeTransactions} from "$lib/api/normalize";
import {dec, formatDec, toNumber, type Dec} from "$lib/domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction} from "$lib/domain/types";
import {OTHER, bigNumbers, bucketKey, commoditiesInUse, lineData, maxAccountDepth, pieData, rankedAccounts, styleFor} from "./series";

// ---------- helpers ----------

const USD_STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

function usd(cents: number): Amount {
    return {commodity: "$", qty: dec(cents, 2), style: USD_STYLE};
}

function eur(cents: number): Amount {
    return {commodity: "EUR", qty: dec(cents, 2), style: {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]}};
}

function posting(account: string, ...amounts: Amount[]): Posting {
    return {account, amounts, status: "unmarked", comment: "", tags: []};
}

let nextIndex = 1;
function txn(date: ISODate, ...postings: Posting[]): Transaction {
    return {index: nextIndex++, date, status: "cleared", description: "t", code: "", comment: "", tags: [], postings, haystack: ""};
}

function fmt(d: Dec): string {
    return formatDec(d, USD_STYLE);
}

// ---------- bucketKey (local stand-in for WP-06 periods.ts) ----------

describe("UNIT bucketKey", () => {
    it("daily is the date itself", () => {
        expect(bucketKey("2025-03-14", "daily")).toBe("2025-03-14");
    });

    it("monthly is YYYY-MM", () => {
        expect(bucketKey("2025-12-31", "monthly")).toBe("2025-12");
        expect(bucketKey("2026-01-01", "monthly")).toBe("2026-01");
    });

    it("weekly is the Monday of the week", () => {
        expect(bucketKey("2024-07-01", "weekly")).toBe("2024-07-01"); // a Monday maps to itself
        expect(bucketKey("2024-07-03", "weekly")).toBe("2024-07-01"); // Wednesday
        expect(bucketKey("2024-07-07", "weekly")).toBe("2024-07-01"); // Sunday still belongs to Monday's week
        expect(bucketKey("2024-07-08", "weekly")).toBe("2024-07-08"); // next Monday starts a new week
    });

    it("weekly crosses month and year boundaries", () => {
        expect(bucketKey("2024-03-01", "weekly")).toBe("2024-02-26"); // Friday, week began in February
        expect(bucketKey("2026-01-01", "weekly")).toBe("2025-12-29"); // Thursday, week began in December
        expect(bucketKey("2024-02-29", "weekly")).toBe("2024-02-26"); // leap day
    });
});

// ---------- depth clamping ----------

describe("UNIT pieData depth clamping", () => {
    const txns = [
        txn("2025-01-05", posting("expenses:food:groceries", usd(10_00)), posting("assets:bank:checking", usd(-10_00))),
        txn("2025-01-06", posting("expenses:food:restaurants", usd(20_00)), posting("assets:bank:checking", usd(-20_00))),
        txn("2025-01-07", posting("expenses:housing:rent", usd(100_00)), posting("assets:bank:checking", usd(-100_00))),
    ];

    it("depth 1 groups to root accounts", () => {
        const slices = pieData(txns, {depth: 1, commodity: "$"});
        expect(slices.map((s) => s.account).sort()).toEqual(["assets", "expenses"]);
        const expenses = slices.find((s) => s.account === "expenses");
        expect(expenses?.value).toBeCloseTo(130, 10);
        expect(expenses?.formatted).toBe("$130.00");
    });

    it("depth 2 splits expenses into food/housing", () => {
        const slices = pieData(txns, {depth: 2, commodity: "$"});
        expect(slices.map((s) => s.account).sort()).toEqual(["assets:bank", "expenses:food", "expenses:housing"]);
        expect(slices.find((s) => s.account === "expenses:food")?.value).toBeCloseTo(30, 10);
    });

    it("depth beyond the deepest account is a no-op", () => {
        const at3 = pieData(txns, {depth: 3, commodity: "$"});
        const at9 = pieData(txns, {depth: 9, commodity: "$"});
        expect(at9).toEqual(at3);
    });

    it("maxAccountDepth reports the deepest posting account", () => {
        expect(maxAccountDepth(txns)).toBe(3);
    });
});

// ---------- top-N + other ----------

describe("UNIT top-N + other bucketing", () => {
    const txns = [
        txn(
            "2025-01-05",
            posting("expenses:a", usd(500_00)),
            posting("expenses:b", usd(400_00)),
            posting("expenses:c", usd(300_00)),
            posting("expenses:d", usd(200_00)),
            posting("expenses:e", usd(100_00)),
            posting("assets:bank", usd(-1500_00))
        ),
    ];

    it("folds the tail into OTHER, keeping maxSlices groups total", () => {
        const slices = pieData(txns, {depth: 2, commodity: "$", maxSlices: 3});
        expect(slices.map((s) => s.account)).toEqual(["assets:bank", "expenses:a", OTHER]);
        // OTHER = b + c + d + e = 1000
        expect(slices.find((s) => s.account === OTHER)?.value).toBeCloseTo(1000, 10);
    });

    it("slices sum to the period total of the commodity", () => {
        const slices = pieData(txns, {depth: 2, commodity: "$", maxSlices: 3});
        const sum = slices.reduce((acc, s) => acc + s.value, 0);
        expect(sum).toBeCloseTo(0, 10); // balanced single-commodity journal: total is zero
    });

    it("no OTHER group when accounts fit within maxSlices", () => {
        const slices = pieData(txns, {depth: 1, commodity: "$", maxSlices: 6});
        expect(slices.map((s) => s.account).sort()).toEqual(["assets", "expenses"]);
    });

    it("lineData caps series the same way and ranks by magnitude", () => {
        const series = lineData(txns, {depth: 2, commodity: "$", interval: "monthly", maxSeries: 3});
        expect(series.map((s) => s.account)).toEqual(["assets:bank", "expenses:a", OTHER]);
    });

    it("rankedAccounts orders by absolute volume descending", () => {
        expect(rankedAccounts(txns, 2, "$")).toEqual(["assets:bank", "expenses:a", "expenses:b", "expenses:c", "expenses:d", "expenses:e"]);
    });
});

// ---------- interval bucketing across boundaries ----------

describe("UNIT lineData interval bucketing", () => {
    const txns = [
        txn("2024-11-15", posting("expenses:food", usd(10_00)), posting("assets:bank", usd(-10_00))),
        txn("2025-02-10", posting("expenses:food", usd(40_00)), posting("assets:bank", usd(-40_00))),
    ];

    it("monthly buckets span the year boundary with zero-filled gaps", () => {
        const series = lineData(txns, {depth: 1, commodity: "$", interval: "monthly"});
        const food = series.find((s) => s.account === "expenses");
        expect(food?.points.map((p) => p.bucket)).toEqual(["2024-11", "2024-12", "2025-01", "2025-02"]);
        expect(food?.points.map((p) => p.value)).toEqual([10, 0, 0, 40]);
    });

    it("weekly buckets are consecutive Mondays across the year boundary", () => {
        const weekly = [
            txn("2025-12-30", posting("expenses:food", usd(5_00)), posting("assets:bank", usd(-5_00))), // Tuesday, week of 2025-12-29
            txn("2026-01-07", posting("expenses:food", usd(7_00)), posting("assets:bank", usd(-7_00))), // Wednesday, week of 2026-01-05
        ];
        const series = lineData(weekly, {depth: 1, commodity: "$", interval: "weekly"});
        const food = series.find((s) => s.account === "expenses");
        expect(food?.points.map((p) => p.bucket)).toEqual(["2025-12-29", "2026-01-05"]);
        expect(food?.points.map((p) => p.value)).toEqual([5, 7]);
    });

    it("daily buckets zero-fill across a month boundary", () => {
        const daily = [
            txn("2024-02-28", posting("expenses:food", usd(1_00)), posting("assets:bank", usd(-1_00))),
            txn("2024-03-01", posting("expenses:food", usd(3_00)), posting("assets:bank", usd(-3_00))),
        ];
        const series = lineData(daily, {depth: 1, commodity: "$", interval: "daily"});
        const food = series.find((s) => s.account === "expenses");
        expect(food?.points.map((p) => p.bucket)).toEqual(["2024-02-28", "2024-02-29", "2024-03-01"]); // 2024 is a leap year
        expect(food?.points.map((p) => p.value)).toEqual([1, 0, 3]);
    });

    it("uses the posting date override when present", () => {
        const withPdate = [txn("2025-01-31", {...posting("expenses:food", usd(9_00)), date: "2025-02-01"}, posting("assets:bank", usd(-9_00)))];
        const series = lineData(withPdate, {depth: 1, commodity: "$", interval: "monthly"});
        const food = series.find((s) => s.account === "expenses");
        expect(food?.points.find((p) => p.bucket === "2025-02")?.value).toBe(9);
    });
});

// ---------- sign conventions ----------

describe("UNIT bigNumbers sign conventions", () => {
    const txns = [
        // hledger convention: revenue postings are negative
        txn("2025-01-01", posting("income:salary", usd(-5000_00)), posting("assets:bank", usd(5000_00))),
        txn("2025-01-02", posting("expenses:rent", usd(1800_00)), posting("assets:bank", usd(-1800_00))),
        txn("2025-01-03", posting("revenues:interest", usd(-10_00)), posting("assets:bank", usd(10_00))),
    ];

    it("income displays positive, expenses positive, net = income - expenses", () => {
        const {income, expenses, net} = bigNumbers(txns, "$");
        expect(toNumber(income)).toBeCloseTo(5010, 10);
        expect(toNumber(expenses)).toBeCloseTo(1800, 10);
        expect(toNumber(net)).toBeCloseTo(3210, 10);
    });

    it("a refund (negative expense) reduces expenses", () => {
        const withRefund = [...txns, txn("2025-01-04", posting("expenses:rent", usd(-100_00)), posting("assets:bank", usd(100_00)))];
        expect(toNumber(bigNumbers(withRefund, "$").expenses)).toBeCloseTo(1700, 10);
    });

    it("ignores asset/liability/equity postings and other commodities", () => {
        const {income, expenses} = bigNumbers(txns, "EUR");
        expect(toNumber(income)).toBe(0);
        expect(toNumber(expenses)).toBe(0);
    });
});

// ---------- commodities ----------

describe("UNIT commoditiesInUse", () => {
    it("sorts by frequency, ties alphabetical", () => {
        const txns = [
            txn("2025-01-01", posting("expenses:a", usd(1_00)), posting("assets:bank", usd(-1_00))),
            txn("2025-01-02", posting("expenses:a", usd(1_00)), posting("assets:bank", usd(-1_00))),
            txn("2025-01-03", posting("expenses:b", eur(2_00)), posting("assets:wise", eur(-2_00))),
        ];
        expect(commoditiesInUse(txns)).toEqual(["$", "EUR"]);
    });

    it("styleFor returns the commodity's posting style", () => {
        const txns = [txn("2025-01-03", posting("expenses:b", eur(2_00)), posting("assets:wise", eur(-2_00)))];
        expect(styleFor(txns, "EUR").side).toBe("R");
        expect(styleFor(txns, "EUR").decimalPoint).toBe(",");
    });
});

// ---------- fixture cross-check against hledger CLI ----------

describe("INTEGRATION fixture big numbers vs hledger is", () => {
    const raw: unknown = JSON.parse(readFileSync(fileURLToPath(new URL("../../../../fixtures/api/v1.52/transactions.json", import.meta.url)), "utf8"));
    const all = normalizeTransactions(raw);

    function month(prefix: string): Transaction[] {
        return all.filter((t) => t.date.startsWith(prefix));
    }

    it("2025-03 matches `hledger is -p 2025-03`: income $5,400.00 expenses $3,553.23 net $1,846.77", () => {
        const {income, expenses, net} = bigNumbers(month("2025-03"), "$");
        expect(fmt(income)).toBe("5,400.00");
        expect(fmt(expenses)).toBe("3,553.23");
        expect(fmt(net)).toBe("1,846.77");
    });

    it("2025-09 matches `hledger is -p 2025-09` in $ AND EUR (multi-commodity month)", () => {
        const txns = month("2025-09");
        const usdNums = bigNumbers(txns, "$");
        expect(fmt(usdNums.income)).toBe("5,660.00");
        expect(fmt(usdNums.expenses)).toBe("3,673.68");
        expect(fmt(usdNums.net)).toBe("1,986.32");
        const eurNums = bigNumbers(txns, "EUR");
        expect(fmt(eurNums.income)).toBe("0.00");
        expect(fmt(eurNums.expenses)).toBe("704.50");
        expect(fmt(eurNums.net)).toBe("-704.50");
    });

    it("pie slices sum to the period total for the commodity", () => {
        const txns = month("2025-03");
        let expected = 0;
        for (const t of txns) {
            for (const p of t.postings) {
                for (const a of p.amounts) {
                    if (a.commodity === "$") expected += toNumber(a.qty);
                }
            }
        }
        const slices = pieData(txns, {depth: 2, commodity: "$", maxSlices: 6});
        const sum = slices.reduce((acc, s) => acc + s.value, 0);
        expect(sum).toBeCloseTo(expected, 6);
    });

    it("commoditiesInUse on the fixture leads with $", () => {
        expect(commoditiesInUse(all)[0]).toBe("$");
        expect(commoditiesInUse(all)).toContain("EUR");
        expect(commoditiesInUse(all)).toContain("AAPL");
    });
});
