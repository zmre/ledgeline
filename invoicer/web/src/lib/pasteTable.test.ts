// Spreadsheet paste, the text half (the HTML half needs a DOM: ProseField's component test).
import {describe, expect, it} from "vitest";
import {EMPTY_TABLE, insertBlock, looksLikeTsv, markdownTable, parseTsv, pastedTable} from "./pasteTable.js";

describe("spreadsheet paste", () => {
    it("reads tab-separated cells as Excel and Sheets copy them", () => {
        expect(parseTsv("Item\tHours\nDesign\t4\r\n")).toEqual([
            ["Item", "Hours"],
            ["Design", "4"],
        ]);
        expect(parseTsv('"two\nlines"\t"a ""quote"""\n')).toEqual([["two\nlines", 'a "quote"']]);
    });

    it("tells cells from ordinary text", () => {
        expect(looksLikeTsv("Item\tHours\nDesign\t4")).toBe(true);
        expect(looksLikeTsv("a\tb\tc")).toBe(true);
        expect(looksLikeTsv("one tab\there")).toBe(false);
        expect(looksLikeTsv("no tabs\nat all")).toBe(false);
    });

    it("writes a markdown table, the first row its header, pipes escaped, edges trimmed", () => {
        expect(
            markdownTable([
                ["Item", "Hours", ""],
                ["Design | build", "4", ""],
                ["", "", ""],
            ])
        ).toBe("| Item | Hours |\n| --- | --- |\n| Design \\| build | 4 |");
        expect(markdownTable([["just one column"]])).toBeNull();
        expect(pastedTable("", "Item\tHours\nDesign\t4")).toBe("| Item | Hours |\n| --- | --- |\n| Design | 4 |");
        expect(pastedTable("", "plain words")).toBeNull();
    });

    it("inserts a block between blank lines", () => {
        expect(insertBlock("", EMPTY_TABLE).value).toBe(`${EMPTY_TABLE}\n`);
        expect(insertBlock("Hello", "| a | b |").value).toBe("Hello\n\n| a | b |\n");
        const mid = insertBlock("Before\nAfter", "| a | b |", 6);
        expect(mid.value).toBe("Before\n\n| a | b |\n\nAfter");
        expect(mid.caret).toBe("Before\n\n| a | b |".length);
    });
});
