import {describe, expect, it} from "vitest";
import {fileRow} from "./fileList";
import type {RulesFileSummary} from "./types";

/** A summary with everything defaulted, so each test states only what it is about. */
function file(over: Partial<RulesFileSummary> = {}): RulesFileSummary {
    return {
        id: "capitalone.csv.rules",
        label: "capitalone",
        revision: "1a-00000000000000ff",
        sizeBytes: 512,
        parsed: true,
        account1: null,
        account2: null,
        ifBlockCount: 0,
        editableBlockCount: 0,
        opaqueItemCount: 0,
        warnings: [],
        ...over,
    };
}

describe("UNIT fileList — the row and its tooltip", () => {
    it("splits a nested id into the folder the row shows and the file name", () => {
        const row = fileRow(file({id: "2025/imports/capitalone.csv.rules"}));
        expect(row.directory).toBe("2025/imports");
        expect(row.fileName).toBe("capitalone.csv.rules");
    });

    it("leaves the folder empty for a file in the journal's own directory", () => {
        const row = fileRow(file({id: "capitalone.csv.rules"}));
        expect(row.directory).toBe("");
        expect(row.fileName).toBe("capitalone.csv.rules");
    });

    // The whole reason for this module: two files, same label, and the row has
    // to make them tellable apart.
    it("distinguishes two same-named files by folder", () => {
        const a = fileRow(file({id: "2025/imports/capitalone.csv.rules"}));
        const b = fileRow(file({id: "2026/imports/capitalone.csv.rules"}));
        expect(a.directory).not.toBe(b.directory);
        expect(a.detail).not.toBe(b.detail);
    });

    it("leads the tooltip with the full relative path", () => {
        const row = fileRow(file({id: "2025/imports/capitalone.csv.rules", ifBlockCount: 3}));
        expect(row.detail.startsWith("2025/imports/capitalone.csv.rules")).toBe(true);
    });

    it("counts rules, and singularizes one", () => {
        expect(fileRow(file({ifBlockCount: 3})).detail).toContain("3 rules");
        expect(fileRow(file({ifBlockCount: 1})).detail).toContain("1 rule");
        expect(fileRow(file({ifBlockCount: 1})).detail).not.toContain("1 rules");
        expect(fileRow(file({ifBlockCount: 0})).detail).toContain("0 rules");
    });

    it("mentions advanced items only when there are some", () => {
        expect(fileRow(file({ifBlockCount: 3, opaqueItemCount: 1})).detail).toContain("3 rules, 1 advanced");
        expect(fileRow(file({ifBlockCount: 3, opaqueItemCount: 0})).detail).not.toContain("advanced");
    });

    it("shows the accounts when the file declares them", () => {
        const row = fileRow(file({account1: "assets:bank:checking", account2: "expenses:unknown"}));
        expect(row.detail).toContain("assets:bank:checking → expenses:unknown");
    });

    it("marks a half-declared pair rather than hiding it", () => {
        expect(fileRow(file({account1: "assets:bank:checking", account2: null})).detail).toContain("assets:bank:checking → ?");
    });

    it("omits the accounts line entirely when the file declares neither", () => {
        expect(fileRow(file()).detail).not.toContain("→");
    });

    // An unparsed file's counts are all zero, and reporting "0 rules" for a file
    // we never opened would be a lie rather than a summary.
    it("says a file is unreadable instead of summarizing it", () => {
        const row = fileRow(file({parsed: false, account1: "assets:bank:checking"}));
        expect(row.detail).toContain("not readable");
        expect(row.detail).not.toContain("0 rules");
        expect(row.detail).not.toContain("→");
    });

    it("still leads with the path when the file is unreadable", () => {
        const row = fileRow(file({id: "2025/imports/broken.csv.rules", parsed: false}));
        expect(row.detail).toBe("2025/imports/broken.csv.rules · not readable");
    });

    // web/e2e/imports.e2e.ts asserts this exact string, both as the `data-tip`
    // attribute and inside the row's accessible name. Pinning it here means a
    // change to the format fails in vitest — where it is one command away —
    // rather than in Playwright, which needs a built SPA and a running engine.
    // The values are the ones a live engine reports for that spec's fixture.
    it("produces the exact tooltip the e2e spec asserts on", () => {
        const row = fileRow(
            file({
                id: "scratch/imports-e2e/scratch.csv.rules",
                label: "scratch",
                account1: "assets:bank:checking",
                account2: "expenses:unknown",
                ifBlockCount: 4,
                editableBlockCount: 3,
                opaqueItemCount: 1,
            })
        );
        expect(row.detail).toBe("scratch/imports-e2e/scratch.csv.rules · 4 rules, 1 advanced · assets:bank:checking → expenses:unknown");
        expect(row.directory).toBe("scratch/imports-e2e");
    });
});
