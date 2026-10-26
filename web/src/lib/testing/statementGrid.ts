// Shared assertion for the statements' multi-column layout (statementGrid.ts).
//
// jsdom has no layout engine, so "the columns line up" cannot be measured. What
// CAN be checked is everything that makes them line up under `table-layout:
// fixed`: every block is a fixed-layout table with the SAME colgroup, it names
// its own columns (period headings, and the projected band when asked), and
// every body row fills exactly as many columns as the heading row has.

import {expect} from "vitest";

const colWidths = (block: Element): string[] => [...block.querySelectorAll("colgroup col")].map((col) => col.getAttribute("style") ?? "");

/** The columns a row occupies, colspans counted. */
const span = (tr: Element): number => [...tr.children].reduce((n, cell) => n + Number(cell.getAttribute("colspan") ?? 1), 0);

/**
 * Every block in `blocks` shares one column template of `1 + figures`
 * columns (the line name, then the figures), with headings and, when
 * `banded`, the Actual/Projected band.
 */
export function expectSharedGrid(blocks: readonly Element[], figures: number, banded: boolean): void {
    expect(blocks.length).toBeGreaterThan(1);
    const template = colWidths(blocks[0]!);
    expect(template).toHaveLength(1 + figures);
    for (const block of blocks) {
        const id = block.getAttribute("data-testid");
        expect(block.querySelector("table")?.classList.contains("table-fixed"), `${id} is fixed layout`).toBe(true);
        expect(colWidths(block), `${id} has the shared colgroup`).toEqual(template);
        const head = block.querySelector("thead tr:not([data-testid])");
        expect(head, `${id} has a heading row`).not.toBeNull();
        expect(span(head!), `${id} heading row`).toBe(1 + figures);
        expect(block.querySelector("thead [data-testid='projected-band']") !== null, `${id} band`).toBe(banded);
        const rows = [...block.querySelectorAll("tbody tr, tfoot tr")];
        expect(rows.length).toBeGreaterThan(0);
        for (const tr of rows) expect(span(tr), `${id} row "${tr.querySelector("th")?.textContent?.trim()}"`).toBe(1 + figures);
    }
}
