// The goals editor's `hidden` filter (plan 41 WP-B).
//
// The table above shows every goal that backs one of its rows,
// and the editor lists only the rest. The two claims worth pinning: the filter
// removes exactly what it is told to, and it never turns a journal WITH goals
// into the "No budget goals yet" screen — that one offers to create a budget
// file, which would be a wrong and alarming thing to offer.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {BudgetGoal, BudgetListing} from "#lib/budget/types.js";
import {dec} from "#lib/domain/money.js";
import BudgetEditor from "./BudgetEditor.svelte";

const STYLES = new Map([["$", {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: null}]]);

function goal(index: number, account: string): BudgetGoal {
    return {
        index,
        line: index + 2,
        account,
        unbalanced: true,
        amount: null,
        entry: {commodity: "$", value: dec(100, 0)},
        inverted: false,
        annual: null,
        locked: null,
    };
}

const LISTING: BudgetListing = {
    editable: true,
    defaultTarget: "main",
    canCreateFile: true,
    createFileName: "budget.journal",
    joinable: {monthly: {journalId: "main", block: 0}},
    files: [
        {
            journalId: "main",
            label: "main.journal",
            revision: "r",
            writable: true,
            rules: [
                {
                    block: 0,
                    line: 1,
                    period: {raw: "monthly", simple: "monthly"},
                    description: "",
                    locked: null,
                    goals: [goal(0, "expenses:food"), goal(1, "expenses")],
                },
            ],
        },
    ],
};

function mount(hidden?: ReadonlySet<string>) {
    return render(BudgetEditor, {
        listing: LISTING,
        styles: STYLES,
        busy: false,
        sort: "account",
        dir: "asc",
        removing: null,
        onAdd: vi.fn(),
        onEdit: vi.fn(),
        onRemove: vi.fn(),
        onRemoveCancel: vi.fn(),
        onRemoveConfirm: vi.fn(),
        onCreateFile: vi.fn(),
        hidden,
    });
}

describe("COMPONENT BudgetEditor — goals the table already shows", () => {
    it("lists every goal when nothing is hidden (the table has no report or listing yet)", () => {
        mount();
        expect(screen.getByText("expenses:food")).toBeTruthy();
        expect(screen.getByText("expenses")).toBeTruthy();
        expect(screen.getByRole("heading", {name: "Budget goals"})).toBeTruthy();
    });

    it("leaves out the hidden goals and keeps the rest", () => {
        mount(new Set(["main#0"]));
        expect(screen.queryByText("expenses:food")).toBeNull();
        expect(screen.getByText("expenses")).toBeTruthy();
        expect(screen.getByRole("heading", {name: "Other budget goals"})).toBeTruthy();
        expect(screen.getByTestId("add-goal")).toBeTruthy();
    });

    it("says every goal is in the table, rather than that there are none", () => {
        mount(new Set(["main#0", "main#1"]));
        expect(screen.getByTestId("budget-editor-all-in-table")).toBeTruthy();
        expect(screen.queryByTestId("no-goals")).toBeNull();
        expect(screen.queryByTestId("create-budget-file")).toBeNull();
        expect(screen.getByTestId("add-goal")).toBeTruthy();
    });
});
