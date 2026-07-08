import {describe, expect, it} from "vitest";
import {dec} from "../domain/money";
import {balanceSheet} from "./balanceSheet";
import {txn, usd} from "./test-helpers";

const ma = (cents: number) => new Map([["$", dec(cents, 2)]]);

// $1000 opening, $25 on the card, $500 moved to savings, $10 posted directly
// to the interior account assets:bank, and one posting AFTER asOf.
const txns = [
    txn("2026-01-05", [
        ["assets:bank:checking", usd(100000)],
        ["equity:opening", usd(-100000)],
    ]),
    txn("2026-02-10", [
        ["expenses:food", usd(2500)],
        ["liabilities:cc:visa", usd(-2500)],
    ]),
    txn("2026-03-15", [
        ["assets:bank:savings", usd(50000)],
        ["assets:bank:checking", usd(-50000)],
    ]),
    txn("2026-04-01", [
        ["assets:bank", usd(1000)],
        ["income:interest", usd(-1000)],
    ]),
    txn("2026-07-01", [
        ["assets:bank:checking", usd(99999)],
        ["income:salary", usd(-99999)],
    ]),
];

describe("UNIT reports/balanceSheet", () => {
    it("reports assets and sign-flipped liabilities as of an inclusive date", () => {
        const report = balanceSheet(txns, {asOf: "2026-06-30", depth: 3});
        expect(report.asOf).toBe("2026-06-30");
        expect(report.sections.map((s) => s.title)).toEqual(["Assets", "Liabilities"]);

        const [assets, liabilities] = report.sections;
        expect(assets.rows.map((r) => [r.account, r.depth])).toEqual([
            ["assets", 1],
            ["assets:bank", 2],
            ["assets:bank:checking", 3],
            ["assets:bank:savings", 3],
        ]);
        expect(assets.rows[2].inclusive).toEqual(ma(50000)); // checking: 1000 − 500, July txn excluded
        expect(assets.rows[3].inclusive).toEqual(ma(50000));
        expect(assets.total).toEqual(ma(101000));

        // Liabilities display positive (internally −$25).
        expect(liabilities.rows.map((r) => r.account)).toEqual(["liabilities", "liabilities:cc", "liabilities:cc:visa"]);
        expect(liabilities.rows[0].inclusive).toEqual(ma(2500));
        expect(liabilities.total).toEqual(ma(2500));

        // Net: $1010 − $25. Equity/income/expenses never appear on the bs.
        expect(report.grandTotal).toEqual(ma(98500));
    });

    it("distinguishes own (direct postings) from inclusive (rolled-up) totals", () => {
        const report = balanceSheet(txns, {asOf: "2026-06-30", depth: 2});
        const bank = report.sections[0].rows.find((r) => r.account === "assets:bank");
        expect(bank?.own).toEqual(ma(1000)); // only the direct $10 posting
        expect(bank?.inclusive).toEqual(ma(101000)); // checking + savings + own
        const root = report.sections[0].rows.find((r) => r.account === "assets");
        expect(root?.own).toEqual(new Map());
        expect(root?.inclusive).toEqual(ma(101000));
    });

    it("clamps to depth 1: one row per root, subtree totals intact", () => {
        const report = balanceSheet(txns, {asOf: "2026-06-30", depth: 1});
        expect(report.sections[0].rows).toEqual([{account: "assets", depth: 1, own: new Map(), inclusive: ma(101000)}]);
        expect(report.sections[1].rows).toEqual([{account: "liabilities", depth: 1, own: new Map(), inclusive: ma(2500)}]);
        expect(report.grandTotal).toEqual(ma(98500));
    });

    it("returns empty sections for an asOf before all activity", () => {
        const report = balanceSheet(txns, {asOf: "2025-12-31", depth: 3});
        expect(report.sections[0].rows).toEqual([]);
        expect(report.grandTotal).toEqual(new Map());
    });
});
