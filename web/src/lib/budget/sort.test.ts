import {describe, expect, it} from "vitest";
import {dec, toNumber, type MixedAmount} from "$lib/domain/money";
import {annualised, sortBudgetLines, type BudgetSortKey, type SortDir, type SortFields} from "./sort";

/** A single-commodity `$` amount at places 0. */
const usd = (n: number): MixedAmount => new Map([["$", dec(n, 0)]]);

/** The sorter reads exactly these three fields, whatever the item really is. */
type Line = SortFields;

const line = (account: string, amount: number, period = "monthly"): Line => ({account, amount: usd(amount), period});

const names = (items: readonly Line[], key: BudgetSortKey, dir: SortDir): string[] =>
    sortBudgetLines(items, key, dir, (item) => item).map((item) => item.account);

describe("UNIT budget/sort — annualised", () => {
    it("scales each period to a year", () => {
        const table: [string, number][] = [
            ["daily", 365],
            ["weekly", 52],
            ["monthly", 12],
            ["quarterly", 4],
            ["yearly", 1],
        ];
        for (const [period, factor] of table) {
            expect(toNumber(annualised(usd(100), period).get("$")!)).toBe(100 * factor);
        }
    });

    it("leaves an unrecognised period's amount alone rather than guessing a factor", () => {
        // `~ every 2 weeks`: the editor shows it read-only, and this keeps that
        // same honesty. `sortBudgetLines` is what puts it last.
        expect(toNumber(annualised(usd(100), "every 2 weeks").get("$")!)).toBe(100);
    });

    it("scales every commodity of a multi-commodity goal", () => {
        const goal: MixedAmount = new Map([
            ["$", dec(10, 0)],
            ["EUR", dec(20, 0)],
        ]);
        const yearly = annualised(goal, "monthly");
        expect(toNumber(yearly.get("$")!)).toBe(120);
        expect(toNumber(yearly.get("EUR")!)).toBe(240);
    });
});

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
    it("compares ANNUALISED magnitudes, so periods rank against each other", () => {
        // $100/week is $5,200 a year and outranks $300/month's $3,600, which the
        // written figures alone say the opposite of.
        const lines = [line("monthly-300", 300, "monthly"), line("weekly-100", 100, "weekly"), line("yearly-9000", 9000, "yearly")];
        expect(names(lines, "amount", "desc")).toEqual(["yearly-9000", "weekly-100", "monthly-300"]);
        expect(names(lines, "amount", "asc")).toEqual(["monthly-300", "weekly-100", "yearly-9000"]);
    });

    it("ranks on the magnitude, so a credit-normal income goal is not sorted upside down", () => {
        const lines = [line("income:dividends", -25, "yearly"), line("income:salary", -5000, "yearly")];
        expect(names(lines, "amount", "desc")).toEqual(["income:salary", "income:dividends"]);
    });

    it("is stable for equal amounts — the journal's own order survives", () => {
        const lines = [line("b", 100), line("a", 100), line("c", 100)];
        expect(names(lines, "amount", "asc")).toEqual(["b", "a", "c"]);
        expect(names(lines, "amount", "desc")).toEqual(["b", "a", "c"]);
    });

    it("sorts an unrecognised period LAST in both directions", () => {
        const lines = [line("odd", 999_999, "every 2 weeks"), line("small", 1), line("big", 1000)];
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
            period: "monthly",
        };
        expect(names([mixed, line("small", 1), line("big", 1000)], "amount", "desc")).toEqual(["big", "small", "mixed"]);
    });

    it("treats an empty goal as zero rather than as unsortable", () => {
        const empty: Line = {account: "empty", amount: new Map(), period: "monthly"};
        expect(names([line("big", 1000), empty], "amount", "asc")).toEqual(["empty", "big"]);
    });
});
