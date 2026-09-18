// The budget bars, mounted.
//
// `budgetSummary.test.ts` already proves the pace arithmetic and every row of
// the health table, so nothing here re-checks a number. What it asks is the one
// thing only a mount can answer, and it is the regression the whole of plans/20
// Phase 1 exists for: a revenue line that beat its target rendered RED and read
// "$3,000 over", because the fill was split at the goal marker and the label was
// computed from a remainder with no sense of which column it was in.
//
// Every assertion is SCOPED TO A ROW. Each section's header bar is classified
// from the section totals, so with one line per section the header says exactly
// what the row says — and an unscoped `getByText` would find both and fail on
// the ambiguity rather than on the claim.
//
// jsdom has no layout engine, so nothing here asks where the pace tick SITS —
// only that it reached the screen and is labelled. The geometry it derives from
// is arithmetic, and is covered as arithmetic.

import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import type {AccountType} from "$lib/domain/accountTypes";
import {dec} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {BudgetReport} from "$lib/reports/types";
import BudgetSummary from "./BudgetSummary.svelte";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const STYLES = new Map([["$", STYLE]]);
const DECLARED: ReadonlyMap<string, AccountType> = new Map([
    ["income", "revenue"],
    ["expenses", "expense"],
]);

const usd = (n: number) => new Map([["$", dec(n, 0)]]);

/** A one-bucket report with one income row and one expense row, in whole dollars. */
function report(incomeActual: number, incomeGoal: number, expenseActual: number, expenseGoal: number): BudgetReport {
    return {
        kind: "budget",
        buckets: ["2026-01"],
        rows: [
            // Income goals are written NEGATIVE, per hledger's credit convention.
            {account: "income:consulting", depth: 2, cells: [{actual: usd(-incomeActual), goal: usd(-incomeGoal)}]},
            {account: "expenses:food", depth: 2, cells: [{actual: usd(expenseActual), goal: usd(expenseGoal)}]},
        ],
        totals: [{actual: usd(expenseActual - incomeActual), goal: usd(expenseGoal - incomeGoal)}],
    };
}

/** Mount over January 2026, as of `asOf` — a prop, so no clock is mocked. */
function mount(current: BudgetReport, asOf = "2026-01-31", sort: "account" | "amount" = "account", dir: "asc" | "desc" = "asc") {
    return render(BudgetSummary, {
        report: current,
        styles: STYLES,
        declared: DECLARED,
        from: "2026-01-01",
        to: "2026-01-31",
        asOf,
        sort,
        dir,
    });
}

/** The row button for `account` — the element carrying its label and its bar. */
const row = (account: string): HTMLElement => screen.getByText(account, {selector: "span"}).closest("button") as HTMLElement;

/** The single-colour fill inside a row's track. */
const fill = (account: string): Element => row(account).querySelector("[aria-hidden='true']")?.firstElementChild as Element;

describe("COMPONENT BudgetSummary — a revenue line that beat its target", () => {
    // $13,000 earned against a $10,000 target, on the LAST day of the span, so
    // pace is the goal and nothing about proration is in play.
    const beaten = () => mount(report(13_000, 10_000, 100, 1000));

    it("is green, not red — the bug this phase exists for", () => {
        beaten();
        expect(fill("income:consulting").className).toContain("bg-success");
        expect(fill("income:consulting").className).not.toContain("bg-error");
    });

    it('says "above target", not "over"', () => {
        beaten();
        const label = within(row("income:consulting")).getByText(/above target/);
        expect(label.textContent).toContain("$3,000");
        expect(label.className).toContain("text-success");
    });
});

describe("COMPONENT BudgetSummary — the expense mirror", () => {
    it("is red and says 'over' when 130% of the budget is spent", () => {
        mount(report(10_000, 10_000, 1300, 1000));
        expect(fill("expenses:food").className).toContain("bg-error");
        const label = within(row("expenses:food")).getByText(/over$/);
        expect(label.textContent).toContain("$300");
        expect(label.className).toContain("text-error");
    });
});

describe("COMPONENT BudgetSummary — pace mid-span", () => {
    // The 16th of a 31-day month ⇒ 16/31 elapsed. The income line has earned 30%
    // of its target and is behind pace; the expense line has spent 30% of its
    // budget and is comfortably under it.
    const midSpan = () => mount(report(3000, 10_000, 300, 1000), "2026-01-16");

    it("reads a revenue line short of pace as behind, in red", () => {
        midSpan();
        expect(within(row("income:consulting")).getByText(/behind pace/).className).toContain("text-error");
        expect(fill("income:consulting").className).toContain("bg-error");
    });

    it("still reads an expense line under pace as healthy, in green", () => {
        midSpan();
        expect(fill("expenses:food").className).toContain("bg-success");
        expect(within(row("expenses:food")).getByText(/left$/).className).toContain("text-success");
    });

    it("renders a labelled pace tick", () => {
        midSpan();
        // Presence and labelling only — jsdom cannot say where it sits.
        expect(within(row("expenses:food")).getAllByTitle("Where this should be by 2026-01-16").length).toBe(1);
    });

    it("draws no pace tick once the span has ended", () => {
        mount(report(3000, 10_000, 300, 1000), "2026-02-15");
        expect(screen.queryByTitle(/Where this should be/)).toBeNull();
    });
});

describe("COMPONENT BudgetSummary — sorting", () => {
    const many: BudgetReport = {
        kind: "budget",
        buckets: ["2026-01"],
        rows: [
            {account: "expenses:zoo", depth: 2, cells: [{actual: usd(10), goal: usd(50)}]},
            {account: "expenses:art", depth: 2, cells: [{actual: usd(10), goal: usd(900)}]},
        ],
        totals: [{actual: usd(20), goal: usd(950)}],
    };

    /** The category names, in the order they were rendered. */
    const accounts = (): string[] =>
        screen
            .getAllByRole("button")
            .map((button) => button.querySelector("span")?.textContent ?? "")
            .filter((name) => name.startsWith("expenses:"));

    it("orders by account name ascending by default", () => {
        mount(many);
        expect(accounts()).toEqual(["expenses:art", "expenses:zoo"]);
    });

    it("orders by goal amount, biggest first, when asked", () => {
        mount(many, "2026-01-31", "amount", "desc");
        expect(accounts()).toEqual(["expenses:art", "expenses:zoo"]);
    });

    it("reverses on the direction control", () => {
        mount(many, "2026-01-31", "amount", "asc");
        expect(accounts()).toEqual(["expenses:zoo", "expenses:art"]);
    });
});
