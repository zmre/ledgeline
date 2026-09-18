import {describe, expect, it} from "vitest";
import {dec, toNumber, type MixedAmount} from "../domain/money";
import type {BudgetReport} from "./types";
import {
    barGeometry,
    budgetHealth,
    budgetLeaves,
    budgetTotals,
    elapsedFraction,
    magnitudeAmount,
    paceAmount,
    primaryValue,
    summarizeBudget,
    UNBUDGETED,
    type BudgetLine,
} from "./budgetSummary";

/** A single-commodity `$` amount at places 0. */
const usd = (n: number): MixedAmount => new Map([["$", dec(n, 0)]]);

const REPORT: BudgetReport = {
    kind: "budget",
    buckets: ["2026-01", "2026-02"],
    rows: [
        {
            account: UNBUDGETED,
            depth: 1,
            cells: [
                {actual: usd(-375), goal: null},
                {actual: new Map(), goal: null},
            ],
        },
        {
            account: "expenses:food",
            depth: 2,
            cells: [
                {actual: usd(352), goal: usd(400)},
                {actual: usd(390), goal: usd(400)},
            ],
        },
        {
            account: "expenses:fun",
            depth: 2,
            cells: [
                {actual: usd(210), goal: usd(150)},
                {actual: usd(95), goal: usd(150)},
            ],
        },
    ],
    totals: [
        {actual: usd(-23), goal: usd(550)},
        {actual: usd(65), goal: usd(550)},
    ],
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
        expect(
            primaryValue(
                new Map([
                    ["$", dec(100, 0)],
                    ["EUR", dec(50, 0)],
                ])
            )
        ).toBeNull();
    });
});

describe("UNIT budgetSummary — elapsedFraction", () => {
    it("is one day's worth on the span's first day", () => {
        // 31 days in January; the pace mark is not at zero on the 1st, because
        // the 1st is a day you have already had.
        expect(elapsedFraction("2026-01-01", "2026-01-31", "2026-01-01")).toBeCloseTo(1 / 31, 10);
    });

    it("is half way through a year at the end of June", () => {
        // 181 of 365 days done — "month six would be half", as asked.
        expect(elapsedFraction("2026-01-01", "2026-12-31", "2026-06-30")).toBeCloseTo(181 / 365, 10);
    });

    it("counts the leap day", () => {
        expect(elapsedFraction("2024-01-01", "2024-12-31", "2024-12-31")).toBe(1);
        expect(elapsedFraction("2024-02-01", "2024-02-29", "2024-02-15")).toBeCloseTo(15 / 29, 10);
    });

    it("is 1 for a span that has already ended — a finished period is fully paced", () => {
        expect(elapsedFraction("2025-01-01", "2025-12-31", "2026-07-08")).toBe(1);
        expect(elapsedFraction("2025-01-01", "2025-12-31", "2025-12-31")).toBe(1);
    });

    it("is 0 before the span starts, and 1 on a single-day span's own day", () => {
        expect(elapsedFraction("2026-05-01", "2026-05-31", "2026-04-30")).toBe(0);
        expect(elapsedFraction("2026-05-10", "2026-05-10", "2026-05-10")).toBe(1);
    });
});

describe("UNIT budgetSummary — paceAmount", () => {
    it("prorates each commodity exactly, without a float round trip", () => {
        const goal: MixedAmount = new Map([
            ["$", dec(400, 0)],
            ["EUR", dec(100, 0)],
        ]);
        const half = paceAmount(goal, 0.5);
        // Exact Decs, not floats: 200.000000 and 50.000000 at the pace scale.
        expect(half.get("$")).toEqual({m: 200_000_000n, p: 6});
        expect(toNumber(half.get("EUR")!)).toBe(50);
    });

    it("is the whole goal at a fraction of 1 and nothing at 0", () => {
        const goal = new Map([["$", dec(1200, 0)]]);
        expect(primaryValue(paceAmount(goal, 1))).toBe(1200);
        expect(primaryValue(paceAmount(goal, 0))).toBe(0);
    });

    it("leaves an empty goal empty", () => {
        expect(paceAmount(new Map(), 0.5).size).toBe(0);
    });
});

describe("UNIT budgetSummary — budgetHealth", () => {
    // The six rows of the label table in plans/20, as arithmetic. Goal 1,000;
    // pace 500 (half way through the span).
    it("expense: at or under pace is healthy", () => {
        expect(budgetHealth(400, 500, 1000, false)).toBe("healthy");
        expect(budgetHealth(500, 500, 1000, false)).toBe("healthy");
    });

    it("expense: past pace but still under goal is behind", () => {
        expect(budgetHealth(700, 500, 1000, false)).toBe("behind");
    });

    it("expense: past the goal is over, not merely behind", () => {
        expect(budgetHealth(1100, 500, 1000, false)).toBe("over");
        // The envelope case that matters most: the span has ended, so pace IS
        // the goal, and a penny past it still reads `over`.
        expect(budgetHealth(1001, 1000, 1000, false)).toBe("over");
    });

    it("revenue: at or past the goal is healthy — this is the whole bug", () => {
        // Earning 130% of target used to render red and read "$300 over".
        expect(budgetHealth(1300, 500, 1000, true)).toBe("healthy");
    });

    it("revenue: at or past pace but under goal is still healthy", () => {
        expect(budgetHealth(600, 500, 1000, true)).toBe("healthy");
        expect(budgetHealth(500, 500, 1000, true)).toBe("healthy");
    });

    it("revenue: short of pace is behind", () => {
        expect(budgetHealth(400, 500, 1000, true)).toBe("behind");
    });

    it("treats an exactly-zero actual by the same rule in both columns", () => {
        expect(budgetHealth(0, 500, 1000, true)).toBe("behind"); // earned nothing, half way through
        expect(budgetHealth(0, 500, 1000, false)).toBe("healthy"); // spent nothing, half way through
        expect(budgetHealth(0, 0, 0, false)).toBe("healthy"); // no goal, no spending
    });
});

describe("UNIT budgetSummary — barGeometry", () => {
    it("leaves headroom past the marker when under budget", () => {
        const g = barGeometry(352, 400, 400); // scaleMax = 500
        expect(g.markerPct).toBeCloseTo(80, 5); // 400/500
        expect(g.fillPct).toBeCloseTo(70.4, 5); // 352/500
        expect(g.ratio).toBeCloseTo(0.88, 5);
    });

    it("saturates the fill and slides the marker left when over budget", () => {
        const g = barGeometry(210, 150, 150); // scaleMax = 210
        expect(g.markerPct).toBeCloseTo(71.4286, 3); // 150/210
        expect(g.fillPct).toBe(100); // one fill, no split at the marker
        expect(g.ratio).toBeCloseTo(1.4, 5);
    });

    it("places the pace mark on the same scale as the marker", () => {
        const g = barGeometry(352, 400, 200); // half way through the span
        expect(g.pacePct).toBeCloseTo(40, 5); // 200/500
        expect(g.markerPct).toBeCloseTo(80, 5);
    });

    it("drops the pace mark when it would be drawn on top of the goal marker", () => {
        expect(barGeometry(352, 400, 400).pacePct).toBeNull();
        // Within half a percentage point of the marker: one tick, not two.
        expect(barGeometry(352, 400, 399).pacePct).toBeNull();
    });

    it("handles a zero budget (spent with no goal amount)", () => {
        expect(barGeometry(375, 0, 0)).toEqual({fillPct: 100, markerPct: 100, pacePct: null, ratio: null});
        expect(barGeometry(0, 0, 0)).toEqual({fillPct: 0, markerPct: 100, pacePct: null, ratio: null});
    });
});
