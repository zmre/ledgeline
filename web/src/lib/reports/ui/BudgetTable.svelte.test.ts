// The budget table, mounted (plan 41 WP-B).
//
// What only a mount can answer: that the four columns are there and the Total
// row closes each section; that the Budgeted figure offers exactly the edit the
// goals behind it allow (one → the modal, several → a list, none editable → a
// lock), with the pencil inside the same button; that the bar's colour is the
// track-fixed gradient with the right MEANING per section — an income bar must
// never carry red — and that the goal marker is outside the track's clip.
//
// The numbers and verdicts are the engine's and are not re-checked here; nor is
// where anything sits (jsdom has no layout engine, so the container-query fold
// is checked by eye, not here).

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {rowGoals} from "#lib/budget/rowGoals.js";
import type {BudgetFile, BudgetGoal, BudgetListing, BudgetRule} from "#lib/budget/types.js";
import {dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {BudgetBar, BudgetReport, BudgetSummarySection} from "#lib/reports/types.js";
import {periodsOf} from "#lib/reports/test-helpers.js";
import BudgetTable from "./BudgetTable.svelte";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const STYLES = new Map([["$", STYLE]]);
const usd = (n: number) => new Map([["$", dec(n, 0)]]);

function bar(account: string, actual: number, goal: number, verdict: Pick<BudgetBar, "health" | "label">, income = false): BudgetBar {
    return {account, income, actual: usd(actual), goal: usd(goal), pace: usd(goal), remaining: usd(goal - actual), ...verdict};
}

const LEFT = (n: number): Pick<BudgetBar, "health" | "label"> => ({health: "healthy", label: {kind: "left", amount: usd(n)}});

function section(kind: "revenue" | "expense", bars: BudgetBar[], overall: BudgetBar): BudgetSummarySection {
    return {kind, overall: {...overall, account: ""}, bars};
}

function report(sections: BudgetSummarySection[]): BudgetReport {
    return {
        kind: "budget",
        buckets: ["2026-01"],
        periods: periodsOf(["2026-01"]),
        rows: [],
        totals: [],
        summary: {from: "2026-01-01", to: "2026-01-31", asOf: "2026-01-31", elapsedDays: 31, spanDays: 31, sections},
    };
}

function goal(index: number, account: string, locked: string | null = null): BudgetGoal {
    return {
        index,
        line: index + 2,
        account,
        unbalanced: true,
        amount: null,
        entry: {commodity: "$", value: dec(100, 0)},
        inverted: false,
        annual: null,
        locked,
    };
}

const rule = (block: number, period: "monthly" | "yearly", goals: BudgetGoal[]): BudgetRule => ({
    block,
    line: 1,
    period: {raw: period, simple: period},
    description: "",
    locked: null,
    goals,
});

const file = (rules: BudgetRule[]): BudgetFile => ({journalId: "main", label: "main.journal", revision: "r", writable: true, rules});

function listing(rules: BudgetRule[]): BudgetListing {
    return {editable: true, defaultTarget: "main", canCreateFile: false, createFileName: "budget.journal", files: [file(rules)], joinable: {}};
}

/** The standard page: food (one goal), gifts (monthly + yearly), utilities (locked), salary (income). */
const PAGE = report([
    section(
        "revenue",
        [bar("income:salary", 13_000, 10_000, {health: "healthy", label: {kind: "aboveTarget", amount: usd(3000)}}, true)],
        bar("", 13_000, 10_000, LEFT(0), true)
    ),
    section(
        "expense",
        [
            bar("expenses:food", 130, 300, LEFT(170)),
            bar("expenses:gifts", 0, 600, LEFT(600)),
            bar("expenses:gas", 200, 175, {health: "over", label: {kind: "over", amount: usd(25)}}),
            bar("expenses:utilities", 50, 100, LEFT(50)),
        ],
        bar("", 380, 1175, LEFT(795))
    ),
]);

const LISTING = listing([
    rule(0, "monthly", [
        goal(0, "expenses:food"),
        goal(1, "expenses:gifts"),
        goal(2, "expenses:gas"),
        goal(3, "expenses:utilities", "the amount is an expression"),
    ]),
    rule(1, "yearly", [goal(4, "expenses:gifts"), goal(5, "income:salary")]),
]);

function mount(over: {listing?: BudgetListing | null} = {}) {
    const current = over.listing === undefined ? LISTING : over.listing;
    const accounts = PAGE.summary.sections.flatMap((s) => s.bars.map((b) => b.account));
    const handlers = {onEdit: vi.fn(), onRemove: vi.fn(), onRemoveCancel: vi.fn(), onRemoveConfirm: vi.fn()};
    render(BudgetTable, {
        report: PAGE,
        styles: STYLES,
        from: "2026-01-01",
        to: "2026-01-31",
        asOf: "2026-01-31",
        sort: "account",
        dir: "asc",
        listing: current,
        goalsByAccount: current === null ? new Map() : rowGoals(current, accounts, 2).byAccount,
        busy: false,
        removing: null,
        ...handlers,
    });
    return handlers;
}

/** The row for `account`. */
const row = (account: string): HTMLElement => screen.getByRole("button", {name: account}).closest("[role='row']") as HTMLElement;

describe("COMPONENT BudgetTable — columns", () => {
    it("heads each section with Account, Budgeted, Actual and Remaining", () => {
        mount();
        const expenses = screen.getByTestId("budget-table-expense");
        const headers = within(expenses)
            .getAllByRole("columnheader")
            .map((h) => h.textContent?.trim());
        expect(headers).toEqual(["Budgeted expenses", "Budgeted", "Actual", "Remaining"]);
    });

    it("puts the Total row last in each section, from the section's overall bar", () => {
        mount();
        for (const kind of ["revenue", "expense"]) {
            const rows = within(screen.getByTestId(`budget-table-${kind}`)).getAllByRole("row");
            expect(rows.at(-1)?.dataset.testid).toBe("budget-total-row");
            expect(rows.filter((r) => r.dataset.testid === "budget-total-row")).toHaveLength(1);
        }
        const total = within(screen.getByTestId("budget-table-expense")).getByTestId("budget-total-row");
        expect(total.textContent).toContain("Total");
        expect(total.textContent).toContain("$1,175.00");
        expect(total.textContent).toContain("$380.00");
        expect(total.textContent).toContain("$795.00 left");
    });

    it("shows budgeted, actual and remaining on one row", () => {
        mount();
        const food = row("expenses:food");
        expect(food.textContent).toContain("$300.00");
        expect(food.textContent).toContain("$130.00");
        expect(within(food).getByTestId("budget-remaining").textContent).toBe("$170.00 left");
    });

    it("colours Remaining by verdict: error only for an expense past its goal", () => {
        mount();
        expect(within(row("expenses:gas")).getByTestId("budget-remaining").className).toContain("text-error");
        expect(within(row("expenses:food")).getByTestId("budget-remaining").className).toContain("text-success");
    });
});

describe("COMPONENT BudgetTable — the Budgeted cell's edit affordance", () => {
    it("opens the one editable goal directly", async () => {
        const {onEdit} = mount();
        await fireEvent.click(within(row("expenses:food")).getByTestId("budgeted-edit"));
        expect(onEdit).toHaveBeenCalledTimes(1);
        const [journalId, , editedGoal] = onEdit.mock.calls[0] as [string, BudgetRule, BudgetGoal];
        expect(journalId).toBe("main");
        expect(editedGoal.index).toBe(0);
    });

    it("draws the pencil inside the edit button, before the figure, and names the account", () => {
        mount();
        const edit = within(row("expenses:food")).getByRole("button", {name: "Edit budget for expenses:food"});
        expect(edit.dataset.testid).toBe("budgeted-edit");
        const pencil = within(edit).getByTestId("budgeted-pencil");
        expect(pencil.querySelector("svg")).not.toBeNull();
        // Before the number, in the same button: a click on either is one click.
        expect(edit.firstElementChild).toBe(pencil);
        expect(edit.textContent?.trim()).toBe("$300.00");
    });

    it("gives the several-goals trigger the same pencil and name, keeping its ▾", () => {
        mount();
        const trigger = within(row("expenses:gifts")).getByTestId("budgeted-goals");
        expect(trigger.getAttribute("aria-label")).toMatch(/^Edit budget for expenses:gifts/);
        expect(trigger.firstElementChild?.getAttribute("data-testid")).toBe("budgeted-pencil");
        expect(trigger.textContent).toContain("▾");
    });

    it("lists several goals behind one row, each with its own Edit", async () => {
        const {onEdit, onRemove} = mount();
        const gifts = row("expenses:gifts");
        expect(within(gifts).queryByTestId("budgeted-edit")).toBeNull();
        await fireEvent.click(within(gifts).getByTestId("budgeted-goals"));

        const popover = screen.getByTestId("budgeted-goals-popover");
        expect(popover.textContent).toContain("Monthly");
        expect(popover.textContent).toContain("Annual");
        const edits = within(popover).getAllByRole("button", {name: "Edit"});
        expect(edits).toHaveLength(2);

        await fireEvent.click(within(popover).getAllByRole("button", {name: "Remove"})[1]);
        expect((onRemove.mock.calls[0] as [string, BudgetGoal])[1].index).toBe(4);

        await fireEvent.click(edits[0]);
        expect((onEdit.mock.calls[0] as [string, BudgetRule, BudgetGoal])[2].index).toBe(1);
        // Editing closes the list, so the modal is not opened underneath it.
        expect(screen.queryByTestId("budgeted-goals-popover")).toBeNull();
    });

    it("is plain text with the engine's reason when no backing goal is editable", () => {
        mount();
        const utilities = row("expenses:utilities");
        expect(within(utilities).queryByRole("button", {name: /Edit budget/})).toBeNull();
        expect(within(utilities).queryByTestId("budgeted-pencil")).toBeNull();
        expect(within(utilities).getByTestId("budgeted-locked").getAttribute("title")).toBe("the amount is an expression");
    });

    it("offers no edit at all while the listing is unavailable", () => {
        mount({listing: null});
        expect(screen.queryAllByTestId("budgeted-edit")).toHaveLength(0);
        expect(screen.queryAllByTestId("budgeted-goals")).toHaveLength(0);
        expect(row("expenses:food").textContent).toContain("$300.00");
    });

    it("never makes the Total editable", () => {
        mount();
        const total = within(screen.getByTestId("budget-table-expense")).getByTestId("budget-total-row");
        expect(within(total).queryByRole("button")).toBeNull();
    });
});

describe("COMPONENT BudgetTable — gradient bars", () => {
    const fill = (account: string): HTMLElement => within(row(account)).getByTestId("budget-bar-fill");

    it("ramps an expense bar through orange to red past the goal marker", () => {
        mount();
        const style = fill("expenses:food").getAttribute("style") ?? "";
        expect(style).toContain("--color-success");
        expect(style).toContain("--color-warning");
        expect(style).toContain("--color-orange");
        expect(style).toContain("--color-error");
        expect(style).toContain("clip-path");
    });

    it("never paints income red, even when it beat its target", () => {
        mount();
        const style = fill("income:salary").getAttribute("style") ?? "";
        expect(fill("income:salary").dataset.kind).toBe("revenue");
        expect(style).toContain("--color-info");
        expect(style).not.toContain("--color-error");
        expect(style).not.toContain("--color-orange");
    });
});

describe("COMPONENT BudgetTable — the goal marker", () => {
    it("stands outside the clipped track, so an overrun fill cannot hide it", () => {
        mount();
        const gas = row("expenses:gas");
        const track = within(gas).getByTestId("budget-bar-track");
        const marker = within(gas).getByTestId("budget-goal-marker");
        expect(track.className).toContain("overflow-hidden");
        expect(track.contains(marker)).toBe(false);
        expect(track.contains(within(gas).getByTestId("budget-bar-fill"))).toBe(true);
        // Sibling of the track, so it shares its box and can stand proud of it.
        expect(marker.parentElement).toBe(track.parentElement);
        expect(marker.className).toContain("bg-base-content");
        expect(marker.className).toContain("-inset-y-1");
    });
});

describe("COMPONENT BudgetTable — the account name", () => {
    it("is a button titled as a journal link", () => {
        mount();
        expect(screen.getByRole("button", {name: "expenses:food"}).getAttribute("title")).toBe("View expenses:food in the journal");
    });
});
