// The budget bars, mounted.
//
// The engine decides every number and verdict (`reports/budget_summary.rs`,
// locked to the TypeScript it replaced by `budget_summary_golden.rs`), so
// nothing here re-checks one. What this asks is the thing only a mount can
// answer, and it is the regression the whole of plans/20 Phase 1 exists for: a
// revenue line that beat its target rendered RED and read "$3,000 over",
// because the fill was split at the goal marker and the label was computed from
// a remainder with no sense of which column it was in. The bars below are what
// the engine sends for those cases; the component must render its verdict
// faithfully — colour, sentence, amount, tick.
//
// Every assertion is SCOPED TO A ROW. Each section's header bar is a bar too,
// and an unscoped `getByText` would find both and fail on the ambiguity rather
// than on the claim.
//
// jsdom has no layout engine, so nothing here asks where the pace tick SITS —
// only that it reached the screen and is labelled.

import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {BudgetBar, BudgetReport, BudgetSummarySection} from "#lib/reports/types.js";
import BudgetSummary from "./BudgetSummary.svelte";
import {periodsOf} from "#lib/reports/test-helpers.js";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const STYLES = new Map([["$", STYLE]]);

const usd = (n: number) => new Map([["$", dec(n, 0)]]);

/** An engine bar in whole dollars (magnitudes), with the verdict the engine reached. */
function bar(account: string, actual: number, goal: number, pace: number, verdict: Pick<BudgetBar, "health" | "label">, income = false): BudgetBar {
    return {account, income, actual: usd(actual), goal: usd(goal), pace: usd(pace), remaining: usd(goal - actual), ...verdict};
}

/** A one-bucket January report carrying `sections`. */
function report(sections: BudgetSummarySection[]): BudgetReport {
    return {
        kind: "budget",
        buckets: ["2026-01"],
        periods: periodsOf(["2026-01"]),
        rows: [],
        totals: [],
        summary: {from: "2026-01-01", to: "2026-01-31", asOf: "2026-01-31", elapsedDays: 1, spanDays: 1, sections},
    };
}

/** One section whose overall bar is its only bar (so the header says what the row says). */
const section = (kind: "revenue" | "expense", only: BudgetBar): BudgetSummarySection => ({kind, overall: {...only, account: ""}, bars: [only]});

function mount(current: BudgetReport, asOf = "2026-01-31", sort: "account" | "amount" = "account", dir: "asc" | "desc" = "asc") {
    return render(BudgetSummary, {report: current, styles: STYLES, from: "2026-01-01", to: "2026-01-31", asOf, sort, dir});
}

/** The row button for `account` — the element carrying its label and its bar. */
const row = (account: string): HTMLElement => screen.getByText(account, {selector: "span"}).closest("button") as HTMLElement;

/** The single-colour fill inside a row's track. */
const fill = (account: string): Element => row(account).querySelector("[aria-hidden='true']")?.firstElementChild as Element;

describe("COMPONENT BudgetSummary — a revenue line that beat its target", () => {
    // $13,000 earned against a $10,000 target on the last day of the span.
    const beaten = () =>
        mount(
            report([
                section(
                    "revenue",
                    bar("income:consulting", 13_000, 10_000, 10_000, {health: "healthy", label: {kind: "aboveTarget", amount: usd(3000)}}, true)
                ),
            ])
        );

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

    it("words the section header as earnings", () => {
        beaten();
        expect(screen.getByText(/Income · Earned/)).toBeDefined();
    });
});

describe("COMPONENT BudgetSummary — the expense mirror", () => {
    it("is red and says 'over' when the engine says over", () => {
        mount(report([section("expense", bar("expenses:food", 1300, 1000, 1000, {health: "over", label: {kind: "over", amount: usd(300)}}))]));
        expect(fill("expenses:food").className).toContain("bg-error");
        const label = within(row("expenses:food")).getByText(/over$/);
        expect(label.textContent).toContain("$300");
        expect(label.className).toContain("text-error");
    });
});

describe("COMPONENT BudgetSummary — pace mid-span", () => {
    // 16/31 of January elapsed: income at 30% of target is behind pace; an
    // expense at 30% of budget is comfortably under it.
    const midSpan = () =>
        mount(
            report([
                section("revenue", bar("income:consulting", 3000, 10_000, 5161, {health: "behind", label: {kind: "behindPace", amount: usd(2161)}}, true)),
                section("expense", bar("expenses:food", 300, 1000, 516, {health: "healthy", label: {kind: "left", amount: usd(700)}})),
            ]),
            "2026-01-16"
        );

    it("reads a revenue line short of pace as behind, in red", () => {
        midSpan();
        const label = within(row("income:consulting")).getByText(/behind pace/);
        expect(label.className).toContain("text-error");
        expect(label.textContent).toContain("$2,161");
        expect(fill("income:consulting").className).toContain("bg-error");
    });

    it("still reads an expense line under pace as healthy, in green", () => {
        midSpan();
        expect(fill("expenses:food").className).toContain("bg-success");
        expect(within(row("expenses:food")).getByText(/left$/).className).toContain("text-success");
    });

    it("renders a labelled pace tick", () => {
        midSpan();
        expect(within(row("expenses:food")).getAllByTitle("Where this should be by 2026-01-16").length).toBe(1);
    });

    it("draws no pace tick once pace has reached the goal", () => {
        mount(report([section("expense", bar("expenses:food", 300, 1000, 1000, {health: "healthy", label: {kind: "left", amount: usd(700)}}))]), "2026-02-15");
        expect(screen.queryByTitle(/Where this should be/)).toBeNull();
    });
});

describe("COMPONENT BudgetSummary — a line with no verdict", () => {
    it("draws no bar and keeps the neutral wording for a multi-commodity line", () => {
        const mixed: BudgetBar = {
            ...bar("expenses:mixed", 3, 10, 10, {health: null, label: {kind: "onPlan", amount: null}}),
            goal: new Map([
                ["$", dec(10, 0)],
                ["EUR", dec(5, 0)],
            ]),
        };
        mount(report([section("expense", mixed)]));
        expect(row("expenses:mixed").querySelector("[aria-hidden='true']")).toBeNull();
        expect(within(row("expenses:mixed")).getByText("on plan").className).toContain("text-base-content/60");
    });
});

describe("COMPONENT BudgetSummary — empty", () => {
    it("explains how to add goals when the engine sends no sections", () => {
        mount(report([]));
        expect(screen.getByTestId("budget-empty")).toBeDefined();
    });
});

describe("COMPONENT BudgetSummary — sorting", () => {
    const healthy = {health: "healthy" as const, label: {kind: "left" as const, amount: usd(40)}};
    const many = report([
        {
            kind: "expense",
            overall: bar("", 20, 950, 950, healthy),
            bars: [bar("expenses:zoo", 10, 50, 50, healthy), bar("expenses:art", 10, 900, 900, healthy)],
        },
    ]);

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
