import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "../domain/money";
import type {BudgetReport} from "./types";
import {barGeometry, budgetLeaves, budgetTotals, magnitudeAmount, primaryValue, summarizeBudget, UNBUDGETED, type BudgetLine} from "./budgetSummary";

/** A single-commodity `$` amount at places 0. */
const usd = (n: number): MixedAmount => new Map([["$", dec(n, 0)]]);

const REPORT: BudgetReport = {
    kind: "budget",
    buckets: ["2026-01", "2026-02"],
    rows: [
        {account: UNBUDGETED, depth: 1, cells: [{actual: usd(-375), goal: null}, {actual: new Map(), goal: null}]},
        {account: "expenses:food", depth: 2, cells: [{actual: usd(352), goal: usd(400)}, {actual: usd(390), goal: usd(400)}]},
        {account: "expenses:fun", depth: 2, cells: [{actual: usd(210), goal: usd(150)}, {actual: usd(95), goal: usd(150)}]},
    ],
    totals: [{actual: usd(-23), goal: usd(550)}, {actual: usd(65), goal: usd(550)}],
};

describe("UNIT budgetSummary — summarizeBudget", () => {
    it("sums each account's cells into one actual/goal pair", () => {
        const lines = summarizeBudget(REPORT);
        expect(lines).toHaveLength(3);

        const food = lines.find((l) => l.account === "expenses:food")!;
        expect(food.actual.get("$")).toEqual({m: 742n, p: 0}); // 352 + 390
        expect(food.goal?.get("$")).toEqual({m: 800n, p: 0}); // 400 + 400
    });

    it("keeps an unbudgeted row's goal null and folds its zero cell into the actual", () => {
        const unbudgeted = summarizeBudget(REPORT).find((l) => l.account === UNBUDGETED)!;
        expect(unbudgeted.goal).toBeNull();
        expect(unbudgeted.actual.get("$")).toEqual({m: -375n, p: 0});
    });
});

describe("UNIT budgetSummary — budgetTotals", () => {
    it("sums budgeted accounts and excludes unbudgeted", () => {
        const {actual, goal} = budgetTotals(summarizeBudget(REPORT));
        expect(actual.get("$")).toEqual({m: 1047n, p: 0}); // 742 (food) + 305 (fun); unbudgeted excluded
        expect(goal.get("$")).toEqual({m: 1100n, p: 0}); // 800 + 300
    });

    it("leaf sums equal the outermost total (consistent overall number)", () => {
        // Parent budgeted $450 = food:dining $150 + food:groceries $300; transport:bus $60.
        const lines: BudgetLine[] = [
            {account: "expenses:food", depth: 2, actual: usd(425), goal: usd(450)},
            {account: "expenses:food:dining", depth: 3, actual: usd(120), goal: usd(150)},
            {account: "expenses:food:groceries", depth: 3, actual: usd(280), goal: usd(300)},
            {account: "expenses:transport:bus", depth: 3, actual: usd(55), goal: usd(60)},
        ];
        expect(budgetTotals(lines).goal.get("$")).toEqual({m: 510n, p: 0}); // 450 (outermost food) + 60
    });

    it("counts only the top-level budget when a budgeted child nests under a budgeted parent", () => {
        const lines: BudgetLine[] = [
            {account: "expenses", depth: 1, actual: usd(742), goal: usd(800)}, // inclusive parent
            {account: "expenses:food", depth: 2, actual: usd(742), goal: usd(800)}, // its only budgeted child
        ];
        const {actual, goal} = budgetTotals(lines);
        // Without the nesting guard this would double to 1484/1600.
        expect(actual.get("$")).toEqual({m: 742n, p: 0});
        expect(goal.get("$")).toEqual({m: 800n, p: 0});
    });
});

describe("UNIT budgetSummary — budgetLeaves", () => {
    it("hides an aggregate parent when deeper budgeted rows are present, keeps standalone budgets", () => {
        const lines: BudgetLine[] = [
            {account: "expenses", depth: 1, actual: usd(500), goal: usd(510)}, // aggregate parent → hidden
            {account: "expenses:food", depth: 2, actual: usd(425), goal: usd(450)},
            {account: "expenses:transport", depth: 2, actual: usd(55), goal: usd(60)},
            {account: "taxes", depth: 1, actual: usd(200), goal: usd(250)}, // standalone depth-1 → kept
            {account: "<unbudgeted>", depth: 1, actual: usd(-765), goal: null}, // excluded (goal null)
        ];
        expect(budgetLeaves(lines).map((l) => l.account)).toEqual(["expenses:food", "expenses:transport", "taxes"]);
    });

    it("keeps a parent whose only budgeted relatives are NOT its descendants", () => {
        const lines: BudgetLine[] = [
            {account: "expenses:food", depth: 2, actual: usd(100), goal: usd(200)},
            {account: "expenses:foodstuffs", depth: 2, actual: usd(50), goal: usd(75)}, // not a child of food (prefix guard)
        ];
        expect(budgetLeaves(lines).map((l) => l.account)).toEqual(["expenses:food", "expenses:foodstuffs"]);
    });
});

describe("UNIT budgetSummary — magnitudeAmount", () => {
    it("flips a negative (income) amount to magnitude; leaves positive/empty untouched", () => {
        expect(magnitudeAmount(usd(-5000)).get("$")).toEqual({m: 5000n, p: 0}); // income budget → positive
        expect(magnitudeAmount(usd(400)).get("$")).toEqual({m: 400n, p: 0});
        expect(magnitudeAmount(new Map()).size).toBe(0);
    });
});

describe("UNIT budgetSummary — primaryValue", () => {
    it("returns the single-commodity magnitude, 0 for empty, null for multi-commodity", () => {
        expect(primaryValue(usd(352))).toBe(352);
        expect(primaryValue(new Map())).toBe(0);
        expect(primaryValue(new Map([["$", dec(100, 0)], ["EUR", dec(50, 0)]]))).toBeNull();
    });
});

describe("UNIT budgetSummary — barGeometry", () => {
    it("leaves headroom past the marker when under budget", () => {
        const g = barGeometry(352, 400); // scaleMax = 500
        expect(g.over).toBe(false);
        expect(g.markerPct).toBeCloseTo(80, 5); // 400/500
        expect(g.underPct).toBeCloseTo(70.4, 5); // 352/500
        expect(g.overPct).toBe(0);
        expect(g.ratio).toBeCloseTo(0.88, 5);
    });

    it("saturates the fill and slides the marker left when over budget", () => {
        const g = barGeometry(210, 150); // scaleMax = 210
        expect(g.over).toBe(true);
        expect(g.markerPct).toBeCloseTo(71.4286, 3); // 150/210
        expect(g.underPct).toBeCloseTo(71.4286, 3); // fill capped at marker for the green part
        expect(g.overPct).toBeCloseTo(28.5714, 3); // 100 - marker
        expect(g.ratio).toBeCloseTo(1.4, 5);
    });

    it("handles a zero budget (spent with no goal amount)", () => {
        expect(barGeometry(375, 0)).toEqual({underPct: 100, overPct: 0, markerPct: 100, ratio: null, over: true});
        expect(barGeometry(0, 0)).toEqual({underPct: 0, overPct: 0, markerPct: 100, ratio: null, over: false});
    });
});
