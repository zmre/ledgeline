import {describe, expect, it} from "vitest";
import {dec} from "../domain/money";
import {incomeStatement} from "./incomeStatement";
import {txn, usd} from "./test-helpers";

const ma = (cents: number) => new Map([["$", dec(cents, 2)]]);

const txns = [
    // Before the range:
    txn("2025-12-31", [
        ["income:salary", usd(-500000)],
        ["assets:bank:checking", usd(500000)],
    ]),
    txn("2026-01-15", [
        ["income:salary", usd(-400000)],
        ["assets:bank:checking", usd(400000)],
    ]),
    txn("2026-02-20", [
        ["expenses:food:groceries", usd(15000)],
        ["liabilities:cc", usd(-15000)],
    ]),
    // "revenues" root categorizes as revenue alongside "income":
    txn("2026-03-05", [
        ["revenues:consulting", usd(-20000)],
        ["assets:bank:checking", usd(20000)],
    ]),
    // After the range:
    txn("2026-07-01", [
        ["expenses:food", usd(9999)],
        ["assets:bank:checking", usd(-9999)],
    ]),
];

describe("UNIT reports/incomeStatement", () => {
    it("reports sign-flipped revenues and natural expenses over an inclusive range", () => {
        const report = incomeStatement(txns, {from: "2026-01-01", to: "2026-06-30", depth: 2});
        expect(report.from).toBe("2026-01-01");
        expect(report.to).toBe("2026-06-30");
        expect(report.sections.map((s) => s.title)).toEqual(["Revenues", "Expenses"]);

        const [revenues, expenses] = report.sections;
        expect(revenues.rows.map((r) => [r.account, r.inclusive])).toEqual([
            ["income", ma(400000)], // displayed positive; Dec txn out of range
            ["income:salary", ma(400000)],
            ["revenues", ma(20000)],
            ["revenues:consulting", ma(20000)],
        ]);
        expect(revenues.total).toEqual(ma(420000)); // sums BOTH revenue roots

        expect(expenses.rows.map((r) => [r.account, r.inclusive])).toEqual([
            ["expenses", ma(15000)], // July txn out of range
            ["expenses:food", ma(15000)],
        ]);
        expect(expenses.total).toEqual(ma(15000));

        // Net income = revenues − expenses.
        expect(report.grandTotal).toEqual(ma(405000));
    });

    it("range boundaries are inclusive on both ends", () => {
        const report = incomeStatement(txns, {from: "2025-12-31", to: "2026-07-01", depth: 1});
        expect(report.sections[0].total).toEqual(ma(920000)); // 5000 + 4000 + 200
        expect(report.sections[1].total).toEqual(ma(24999)); // 150.00 + 99.99
        expect(report.grandTotal).toEqual(ma(895001));
    });
});
