// What the account chips do with the width they are given.
//
// jsdom has no layout engine and this suite's `unit` project has no DOM at all,
// so nothing here can render a chip and read its box. What CAN be stated — and
// is the half that was actually wrong — is the arithmetic that decides how much
// room each chip's text is told it has, and it is stated as an INVARIANT rather
// than as a screenshot: after fitting, no chip could show one more rung of its
// name without the row overflowing the cell. A label that fills its column
// satisfies that; the fixed thirty-character budget did not.
//
// A browser is still the only thing that can confirm the CSS agrees with this
// arithmetic. See the report accompanying this change for what a human should
// look at.

import {describe, expect, it} from "vitest";
import {accountRenderings, fitAccount} from "$lib/domain/accounts";
import {CHIP_CHROME_PX, FLOW_ARROW_PX, FLOW_GAP_PX, flowChipRooms, splitChipRooms} from "./chipGeometry";

// A uniform 6px glyph. The lumpy-font case is exercised in accounts.test.ts;
// here the arithmetic under test is the sharing, so an easy ruler keeps the
// expected numbers readable.
const px = (text: string): number => text.length * 6;

// The accounts cell at the widest the app goes: `max-w-7xl` (1280px) minus
// `p-4` (32px) is 1248px of main content, minus the scroller's 1px borders is
// 1246px of table. `table-fixed` hands the fixed columns their widths — date
// `w-24` 96, status `w-16` 64, amount `w-36` 144 — and splits the remaining 942
// evenly between the two auto columns, so the accounts cell is 471px wide, less
// `table-sm`'s 0.75rem padding-inline a side: 447px of content box.
const WIDE_CELL = 447;

// The same sum at a 1024px window: (1024 - 32 - 2 - 304) / 2 - 24.
const NARROW_CELL = 319;

const PAIRS = [
    ["assets:bank:checking", "expenses:household:repairs:plumbing"],
    ["liabilities:creditcards:visa", "expenses:auto:maintenance"],
    ["assets:morganstanley:pw-roth-ira:cash", "expenses:investment:advisory-fees"],
    ["income:salary", "assets:bank:checking"],
];

/** What the row would actually render, and how wide that comes out. */
function renderRow(names: string[], cell: number): {labels: string[]; used: number} {
    const rooms = flowChipRooms(names, cell, px);
    const labels = names.map((name, i) => fitAccount(name, rooms[i], px));
    // A chip is `width: fit-content`, so it occupies its text — or its share,
    // when the text overflows and CSS clips it.
    const used = labels.reduce((sum, label, i) => sum + Math.min(px(label), rooms[i]) + CHIP_CHROME_PX, FLOW_ARROW_PX + FLOW_GAP_PX);
    return {labels, used};
}

describe("UNIT chipGeometry", () => {
    describe("CHIP_CHROME_PX", () => {
        it("is daisyUI's badge-sm padding and border, from the stylesheet's own formula", () => {
            const size = 0.25 * 5 * 16; // `--size: calc(var(--size-selector) * 6)` at badge-sm's 5
            const border = 1; // `--border`
            const paddingInline = size / 2 - border; // daisyUI's own expression

            expect(size).toBe(20);
            expect(CHIP_CHROME_PX).toBe((paddingInline + border) * 2);
        });
    });

    describe("flowChipRooms", () => {
        it("REGRESSION: leaves no chip that could show more of its name without overflowing", () => {
            // The assertion the eye was making in the browser, made arithmetically:
            // every abbreviation present is one the cell could not have afforded
            // to skip. Under the old fixed budget this fails on the first pair at
            // every width, which is what "a third of the column is unused" was.
            for (const names of PAIRS) {
                for (const cell of [WIDE_CELL, 500, 420, 380, NARROW_CELL, 260]) {
                    const {labels, used} = renderRow(names, cell);
                    labels.forEach((label, i) => {
                        if (label === names[i]) return; // nothing left to ask for
                        const ladder = [...accountRenderings(names[i])];
                        const wider = ladder[ladder.indexOf(label) - 1];

                        expect(used - px(label) + px(wider)).toBeGreaterThan(cell);
                    });
                }
            }
        });

        it("shows both names whole when the cell can afford them", () => {
            const {labels, used} = renderRow(["assets:bank:checking", "expenses:auto:maintenance"], WIDE_CELL);

            expect(labels).toEqual(["assets:bank:checking", "expenses:auto:maintenance"]);
            // Short names do not stretch: the leftover is honest empty space,
            // not evidence of over-truncation.
            expect(used).toBeLessThan(WIDE_CELL);
        });

        it("REGRESSION: a long destination may take more than 45% of the cell", () => {
            // `assets:bank:checking → expenses:household:repairs:plumbing` is the
            // shape that wasted the most: a short chip beside a long one. Capped
            // independently at 45%, the long name could never exceed 201px of a
            // 447px cell however little the short one used.
            const names = ["assets:bank:checking", "expenses:household:repairs:plumbing"];
            const [, dest] = flowChipRooms(names, WIDE_CELL, px);

            expect(dest).toBeGreaterThan(WIDE_CELL * 0.45);
            expect(renderRow(names, WIDE_CELL).labels[1]).toBe("expenses:household:repairs:plumbing");
        });

        it("spends the whole cell when the names cannot all fit", () => {
            const rooms = flowChipRooms(["expenses:household:repairs:plumbing", "assets:morganstanley:pw-roth-ira:cash"], NARROW_CELL, px);
            const allotted = rooms.reduce((sum, room) => sum + room + CHIP_CHROME_PX, FLOW_ARROW_PX + FLOW_GAP_PX);

            expect(allotted).toBeCloseTo(NARROW_CELL);
        });

        it("gives the same names more room as the column widens", () => {
            const names = ["expenses:household:repairs:plumbing", "assets:morganstanley:pw-roth-ira:cash"];
            const narrow = flowChipRooms(names, NARROW_CELL, px);
            const wide = flowChipRooms(names, WIDE_CELL, px);

            expect(wide[0]).toBeGreaterThan(narrow[0]);
            expect(wide[1]).toBeGreaterThan(narrow[1]);
            // …and EVERY pixel the column gains reaches the text: the two chips
            // between them take up the whole 128px difference, none of it lost
            // to chrome or rounding. This is the part a fixed character budget
            // cannot do at all — it hands out the same width at both sizes.
            const gained = wide[0] + wide[1] - (narrow[0] + narrow[1]);
            expect(gained).toBeCloseTo(WIDE_CELL - NARROW_CELL);
        });
    });

    describe("splitChipRooms", () => {
        it("offers each wrapped chip the whole line, not the old 176px cap", () => {
            const rooms = splitChipRooms(["a:b", "c:d", "e:f"], WIDE_CELL);

            expect(rooms).toEqual([427, 427, 427]);
            expect(rooms[0]).toBeGreaterThan(176);
        });
    });
});
