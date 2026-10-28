// The Accounts report's display model: section titles, last-column headings,
// and folded chains — shared by the screen and the workbook.

import {describe, expect, it} from "vitest";
import {accountsFixture} from "#lib/testing/projectionFixture.js";
import {accountDisplaySections, DELTA, READING_HELP, summaryHeading} from "./accountReportRows";

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
});
