import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import {sortBudgetLines, type BudgetSortKey, type SortDir, type SortFields} from "./sort";

/** A single-commodity `$` amount at places 0. */
const usd = (n: number): MixedAmount => new Map([["$", dec(n, 0)]]);

/** The sorter reads exactly these two fields, whatever the item really is. */
type Line = SortFields;

const line = (account: string, amount: number): Line => ({account, amount: usd(amount)});

const names = (items: readonly Line[], key: BudgetSortKey, dir: SortDir): string[] =>
    sortBudgetLines(items, key, dir, (item) => item).map((item) => item.account);

describe("UNIT budget/sort — sortBudgetLines by account", () => {
    const lines = [line("expenses:rent", 1500), line("expenses:food", 400), line("income:salary", 5000)];

    it("orders by name in both directions", () => {
        expect(names(lines, "account", "asc")).toEqual(["expenses:food", "expenses:rent", "income:salary"]);
        expect(names(lines, "account", "desc")).toEqual(["income:salary", "expenses:rent", "expenses:food"]);
    });

    it("never mutates the array it was handed", () => {
        const before = lines.map((l) => l.account);
        names(lines, "account", "desc");
        expect(lines.map((l) => l.account)).toEqual(before);
    });
});

describe("UNIT budget/sort — sortBudgetLines by amount", () => {
    it("compares the amounts it is handed (the engine's annualised figures), by magnitude", () => {
        // $100/week arrives as the engine's $5,200 a year and outranks $300/month's
        // $3,600 — the order the written figures alone would get backwards.
        const lines = [line("monthly-300", 3600), line("weekly-100", 5200), line("yearly-9000", 9000)];
        expect(names(lines, "amount", "desc")).toEqual(["yearly-9000", "weekly-100", "monthly-300"]);
        expect(names(lines, "amount", "asc")).toEqual(["monthly-300", "weekly-100", "yearly-9000"]);
    });

    it("ranks on the magnitude, so a credit-normal income goal is not sorted upside down", () => {
        const lines = [line("income:dividends", -25), line("income:salary", -5000)];
        expect(names(lines, "amount", "desc")).toEqual(["income:salary", "income:dividends"]);
    });

    it("is stable for equal amounts — the journal's own order survives", () => {
        const lines = [line("b", 100), line("a", 100), line("c", 100)];
        expect(names(lines, "amount", "asc")).toEqual(["b", "a", "c"]);
        expect(names(lines, "amount", "desc")).toEqual(["b", "a", "c"]);
    });

    it("sorts an amount the engine would not rank (null) LAST in both directions", () => {
        const lines: Line[] = [{account: "odd", amount: null}, line("small", 1), line("big", 1000)];
        expect(names(lines, "amount", "asc")).toEqual(["small", "big", "odd"]);
        expect(names(lines, "amount", "desc")).toEqual(["big", "small", "odd"]);
    });

    it("sorts a multi-commodity goal LAST, because it has no single magnitude", () => {
        const mixed: Line = {
            account: "mixed",
            amount: new Map([
                ["$", dec(1, 0)],
                ["EUR", dec(1, 0)],
            ]),
        };
        expect(names([mixed, line("small", 1), line("big", 1000)], "amount", "desc")).toEqual(["big", "small", "mixed"]);
    });

    it("treats an empty goal as zero rather than as unsortable", () => {
        const empty: Line = {account: "empty", amount: new Map()};
        expect(names([line("big", 1000), empty], "amount", "asc")).toEqual(["empty", "big"]);
    });
});
