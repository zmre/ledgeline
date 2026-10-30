// The Accounts report's display model: section titles, last-column headings,
// and folded chains — shared by the screen and the workbook.

import {describe, expect, it} from "vitest";
import {accountsFixture, usd} from "#lib/testing/projectionFixture.js";
import {
    accountDisplaySections,
    DELTA,
    filterAccountReport,
    parentAccounts,
    READING_HELP,
    reportAccounts,
    summaryHeading,
    visibleAccountRows,
} from "./accountReportRows";

describe("UNIT reports/ui/accountReportRows", () => {
    it("titles each section and heads its last column by how it reads", () => {
        const sections = accountDisplaySections(accountsFixture());
        expect(sections.map((s) => [s.title, summaryHeading(s.reading)])).toEqual([
            ["Assets", `${DELTA} over the period`],
            ["Revenue", "Total"],
            ["Expenses", "Total"],
        ]);
    });

    it("folds a parent into its only child when it has nothing of its own, in every column and the last", () => {
        const [assets, , expenses] = accountDisplaySections(accountsFixture());
        expect(assets!.rows.map((r) => r.row.account)).toEqual(["assets:checking"]);
        // `expenses` has two children, so it stays, with them indented beneath it.
        expect(expenses!.rows.map((r) => [r.row.account, r.indent])).toEqual([
            ["expenses", 0],
            ["expenses:rent", 1],
            ["expenses:shopping", 1],
        ]);
    });

    it("keeps a parent whose own figure differs from its only child's in the last column alone", () => {
        const report = accountsFixture();
        const assets = report.sections[0]!;
        const parent = {...assets.rows[0]!, summary: new Map([["$", {m: 1n, p: 0}]])};
        const changed = {...report, sections: [{...assets, rows: [parent, assets.rows[1]!]}]};
        expect(accountDisplaySections(changed)[0]!.rows.map((r) => r.row.account)).toEqual(["assets", "assets:checking"]);
    });

    it("says in words what each reading's columns are", () => {
        expect(READING_HELP.balance).toContain("balance at the end of each period");
        expect(READING_HELP.balance).toContain(DELTA);
        expect(READING_HELP.change).toContain("Total");
    });

    it("offers every account the report has a row for, once, sorted", () => {
        expect(reportAccounts(accountsFixture())).toEqual([
            "assets",
            "assets:checking",
            "expenses",
            "expenses:rent",
            "expenses:shopping",
            "income",
            "income:salary",
        ]);
    });

    it("is the whole report when nothing is chosen", () => {
        const report = accountsFixture();
        expect(filterAccountReport(report, new Set())).toBe(report);
    });

    it("keeps the chosen subtrees, drops their ancestors and the sections left empty, and re-sums the totals over what is left", () => {
        const filtered = filterAccountReport(accountsFixture(), new Set(["expenses:rent", "income"]));
        expect(filtered.sections.map((s) => [s.section, s.rows.map((r) => r.account)])).toEqual([
            ["revenue", ["income", "income:salary"]],
            ["expenses", ["expenses:rent"]],
        ]);
        // `income` alone is the top row: its child is inside it, not added to it.
        expect(filtered.sections[0]!.totals).toEqual([usd(-5000), usd(-5000), usd(-5000), usd(-5000)]);
        expect(filtered.sections[0]!.summary).toEqual(usd(-20000));
        expect(filtered.sections[1]!.totals).toEqual([usd(4200), usd(4200), usd(4200), usd(4200)]);
        expect(filtered.sections[1]!.summary).toEqual(usd(16800));
    });

    it("sums sibling choices side by side", () => {
        const report = accountsFixture();
        const expenses = report.sections[2]!;
        const shopping = {...expenses.rows[2]!, values: [usd(10), usd(20), usd(30), usd(40)], summary: usd(100)};
        const changed = {...report, sections: [{...expenses, rows: [expenses.rows[0]!, expenses.rows[1]!, shopping]}]};
        const [section] = filterAccountReport(changed, new Set(["expenses:rent", "expenses:shopping"])).sections;
        expect(section!.totals).toEqual([usd(4210), usd(4220), usd(4230), usd(4240)]);
        expect(section!.summary).toEqual(usd(16900));
    });

    it("hides the rows beneath a collapsed row, and marks which rows have any", () => {
        const [, , expenses] = accountDisplaySections(accountsFixture());
        const open = visibleAccountRows(expenses!.rows, new Set());
        expect(open.map((r) => [r.display.row.account, r.parent, r.expanded])).toEqual([
            ["expenses", true, true],
            ["expenses:rent", false, true],
            ["expenses:shopping", false, true],
        ]);
        const shut = visibleAccountRows(expenses!.rows, new Set(["expenses"]));
        expect(shut.map((r) => [r.display.row.account, r.expanded])).toEqual([["expenses", false]]);
        // Collapsing a leaf means nothing: it has nothing to hide.
        expect(visibleAccountRows(expenses!.rows, new Set(["expenses:rent"])).map((r) => r.expanded)).toEqual([true, true, true]);
    });

    it("names every row with rows beneath it, folded chains excluded", () => {
        expect(parentAccounts(accountDisplaySections(accountsFixture()))).toEqual(new Set(["expenses"]));
    });
});
