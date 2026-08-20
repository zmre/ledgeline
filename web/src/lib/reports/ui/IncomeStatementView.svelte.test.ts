// The grouped income statement, mounted.
//
// The row-model tests can prove `isDisplayModel` returns seven rows for a
// collapsed section and still say nothing about whether the screen renders them,
// whether the disclosure is wired to anything, whether a subtotal ended up
// INSIDE the box it is supposed to sit between, or whether pressing `j` reaches
// a row that exists.
//
// jsdom has no layout engine, so nothing here asks how anything LOOKS. It asks
// what is in the document, which element contains which, what `aria-expanded`
// says, and what Enter dispatches.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {decodeIncomeStatementReport} from "$lib/api/nativeDecode";
import type {AmountStyle} from "$lib/domain/types";
import {keymap} from "$lib/keys/keymap.svelte";
import {GROUPED_INCOME_STATEMENT, MULTI_STEP_INCOME_STATEMENT, UNCOMPARED_INCOME_STATEMENT} from "$lib/testing/incomeStatementFixture";
import IncomeStatementView from "./IncomeStatementView.svelte";

// The drill-down navigates, and a router is neither available nor the subject
// here — what matters is that Enter on an account row asks for THAT account.
const openJournal = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("$lib/journal/openJournal", () => ({openJournal}));

const STYLES: ReadonlyMap<string, AmountStyle> = new Map<string, AmountStyle>([
    ["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}],
]);

const REPORT = decodeIncomeStatementReport(GROUPED_INCOME_STATEMENT);
const MULTI = decodeIncomeStatementReport(MULTI_STEP_INCOME_STATEMENT);
const UNCOMPARED = decodeIncomeStatementReport(UNCOMPARED_INCOME_STATEMENT);

const mount = (report = REPORT) => render(IncomeStatementView, {report, styles: STYLES});

/** Press a key the way the app's window listener would. */
async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

/** The disclosure button for a group, by its visible name. */
const disclosure = (name: string): HTMLElement => screen.getByRole("button", {name});

beforeEach(() => {
    openJournal.mockClear();
});

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT IncomeStatementView", () => {
    it("renders one box per section and nothing for the sections the engine omitted", () => {
        mount();

        expect(screen.getByTestId("is-section-revenue")).toBeDefined();
        expect(screen.getByTestId("is-section-opex")).toBeDefined();
        // The adaptive shape: an untagged personal journal gets two boxes, not
        // seven with five reading zero.
        for (const kind of ["cogs", "depreciation", "interest", "tax", "other"]) {
            expect(screen.queryByTestId(`is-section-${kind}`)).toBeNull();
        }
        expect(screen.getByRole("heading", {name: "Revenue"})).toBeDefined();
        expect(screen.getByRole("heading", {name: "Expenses"})).toBeDefined();
    });

    it("puts each box's total in a real tfoot row, once", () => {
        mount();
        const foot = screen.getByTestId("is-section-revenue").querySelector("tfoot");

        expect(foot).not.toBeNull();
        expect(foot?.textContent).toContain("Total Revenue");
        expect(foot?.textContent).toContain("$34,010.00");
        // The duplicate-total complaint: $34,010.00 must appear ONCE in this box,
        // in its footer — not also as an `income` roll-up row above its children.
        const box = screen.getByTestId("is-section-revenue");
        expect(box.textContent?.match(/\$34,010\.00/g)).toHaveLength(1);
    });

    it("shows the range it covers and the window it compares against", () => {
        mount();
        const panel = screen.getByTestId("income-statement");

        expect(panel.textContent).toContain("Market value in $");
        expect(panel.textContent).toContain("2026-01-01 to 2026-07-08");
        expect(panel.textContent).toContain("2025-06-26 to 2025-12-31");
    });

    describe("groups are collapsed by default", () => {
        it("shows the group headings and none of their accounts", () => {
            const {container} = mount();

            expect(screen.getByText("Food")).toBeDefined();
            expect(screen.queryByText("groceries")).toBeNull();
            // `data-account` marks account rows specifically; there should be none.
            expect(container.querySelectorAll("[data-account]")).toHaveLength(0);
        });

        it("still shows every group subtotal, so a collapsed statement is a complete one", () => {
            mount();
            const expenses = screen.getByTestId("is-section-opex");

            expect(expenses.textContent).toContain("$3,500.00"); // Depreciation
            expect(expenses.textContent).toContain("$1,654.38"); // Food
            expect(expenses.textContent).toContain("$13,125.00"); // Housing
            expect(expenses.textContent).toContain("$28,626.48"); // the section total
        });

        it("reports its state through aria-expanded", () => {
            mount();
            expect(disclosure("Food").getAttribute("aria-expanded")).toBe("false");
        });
    });

    describe("expanding a group", () => {
        it("reveals its accounts at full depth, each tagged with its account name", async () => {
            const {container} = mount();
            disclosure("Food").click();
            await tick();

            const accounts = [...container.querySelectorAll("[data-account]")].map((el) => el.getAttribute("data-account"));
            expect(accounts).toEqual(["expenses:food", "expenses:food:groceries", "expenses:food:restaurants"]);
            expect(disclosure("Food").getAttribute("aria-expanded")).toBe("true");
        });

        it("folds a single-child chain into one row, as every other report table does", async () => {
            const {container} = mount();
            disclosure("Housing").click();
            await tick();

            // `expenses:housing` has one child and the same figure in both
            // windows, so the pair is one row carrying the child's account name.
            expect(container.querySelector('[data-account="expenses:housing:rent"]')).not.toBeNull();
            expect(container.querySelector('[data-account="expenses:housing"]')).toBeNull();
        });

        it("keeps the rows clear of the sticky chrome the cursor scrolls them under", async () => {
            const {container} = mount();
            disclosure("Food").click();
            await tick();

            // `scroll-mt-10` is what makes `scrollIntoView({block: "nearest"})`
            // land below the pinned header rather than underneath it. jsdom
            // cannot see the effect, only that the contract is still declared.
            expect(container.querySelector('[data-account="expenses:food:groceries"]')?.classList.contains("scroll-mt-10")).toBe(true);
        });

        it("leaves its neighbours closed", async () => {
            const {container} = mount();
            disclosure("Food").click();
            await tick();

            expect(disclosure("Taxes").getAttribute("aria-expanded")).toBe("false");
            expect(container.querySelector('[data-account="expenses:taxes:federal"]')).toBeNull();
        });

        it("collapses again on a second click", async () => {
            const {container} = mount();
            disclosure("Food").click();
            await tick();
            disclosure("Food").click();
            await tick();

            expect(container.querySelectorAll("[data-account]")).toHaveLength(0);
        });
    });

    describe("the comparison and percentage columns", () => {
        it("shows the prior figure beside the current one", () => {
            mount();
            const revenue = screen.getByTestId("is-section-revenue");

            expect(revenue.textContent).toContain("$34,010.00"); // current
            expect(revenue.textContent).toContain("$39,397.50"); // prior
        });

        it("prints each line's share of revenue to one decimal", () => {
            mount();

            expect(screen.getByTestId("is-section-opex").textContent).toContain("84.2%"); // 28,626.48 / 34,010.00
            expect(screen.getByTestId("is-section-revenue").textContent).toContain("100.0%");
            // 1.9675% → "2.0%", not "2%": the trailing zero says the column has a
            // decimal place, which is what makes 0.5% and 38.6% read as a column.
            expect(screen.getByTestId("is-section-opex").textContent).toContain("2.0%");
        });

        it("drops the prior column entirely when the report is not comparing", () => {
            mount(UNCOMPARED);

            // A blank column headed "Prior" would read as "the prior period was
            // zero" rather than as "there is no prior period".
            expect(screen.queryByRole("columnheader", {name: "Prior"})).toBeNull();
            expect(screen.queryByTestId("is-net-income-prior")).toBeNull();
            expect(screen.getByTestId("income-statement").textContent).not.toContain("$39,397.50");
            // …and the current figures are untouched by its absence.
            expect(screen.getByTestId("is-section-revenue").textContent).toContain("$34,010.00");
        });

        it("shows — where there is no revenue to divide by", () => {
            const noRevenue = {...REPORT, sections: REPORT.sections.filter((s) => s.kind !== "revenue")};
            mount(noRevenue);
            // The last cell of each row is the percentage. The column HEADING
            // still says "% of revenue" — the column exists, it just has no
            // value on this report — so the cells are what has to be asked.
            const pctCells = [...screen.getByTestId("is-section-opex").querySelectorAll("tr")].map((tr) =>
                (tr.querySelector("td:last-child") ?? tr.querySelector("th:last-child"))?.textContent?.trim()
            );

            // Not 0%, not ∞, not NaN: with no revenue there is no such ratio.
            expect(pctCells.filter((text) => text === "—").length).toBe(9); // 8 groups + the section total
            expect(pctCells.filter((text) => text?.endsWith("%"))).toEqual([]);
        });
    });

    describe("the ladder", () => {
        it("prints no rungs at all in simple form", () => {
            mount();
            for (const kind of ["grossProfit", "ebitda", "operatingIncome", "pretaxIncome"]) {
                expect(screen.queryByTestId(`is-subtotal-${kind}`)).toBeNull();
            }
        });

        it("renders every rung of a multi-step statement", () => {
            mount(MULTI);

            expect(screen.getByTestId("is-subtotal-grossProfit").textContent).toContain("Gross profit");
            expect(screen.getByTestId("is-subtotal-grossProfit").textContent).toContain("$517,400.00");
            expect(screen.getByTestId("is-subtotal-ebitda").textContent).toContain("EBITDA");
            expect(screen.getByTestId("is-subtotal-operatingIncome").textContent).toContain("Operating income");
            expect(screen.getByTestId("is-subtotal-pretaxIncome").textContent).toContain("Income before taxes");
        });

        it("puts each rung BETWEEN the boxes, never inside one", () => {
            mount(MULTI);
            const grossProfit = screen.getByTestId("is-subtotal-grossProfit");

            // A subtotal spans everything printed above it, so rendering it inside
            // a box would claim it belonged to that box alone.
            for (const kind of ["revenue", "cogs", "opex", "depreciation", "interest", "tax", "other"]) {
                expect(screen.getByTestId(`is-section-${kind}`).contains(grossProfit)).toBe(false);
            }
        });

        it("orders EBITDA above D&A and Operating income below it", () => {
            const {container} = mount(MULTI);
            const order = [...container.querySelectorAll("[data-testid^='is-section-'],[data-testid^='is-subtotal-']")].map((el) =>
                el.getAttribute("data-testid")
            );

            // Each rung is then a running total of everything above it — no line
            // is ever the sum of things both above and below it.
            expect(order).toEqual([
                "is-section-revenue",
                "is-section-cogs",
                "is-subtotal-grossProfit",
                "is-section-opex",
                "is-subtotal-ebitda",
                "is-section-depreciation",
                "is-subtotal-operatingIncome",
                "is-section-other",
                "is-section-interest",
                "is-subtotal-pretaxIncome",
                "is-section-tax",
            ]);
        });

        it("lets the `other` box print its negative net, flagged for the caller to paint", () => {
            mount(MULTI);
            const other = screen.getByTestId("is-section-other");

            expect(other.textContent).toContain("$-15,000.00");
            expect(other.querySelector("tfoot .text-error")).not.toBeNull();
        });

        it("uses ONE minus character for the amount and the percentage on the same row", () => {
            mount(MULTI);
            const footer = screen.getByTestId("is-section-other").querySelector("tfoot")?.textContent ?? "";

            // The whole row, as rendered: `$-15,000.00 … -2.4%`. `formatDec`
            // writes money with an ASCII hyphen, so a typographic U+2212 on the
            // percentage put two different minus signs an inch apart in one line.
            expect(footer).toContain("$-15,000.00");
            expect(footer).toContain("-2.4%");
            expect(footer).not.toContain("−");
        });
    });

    describe("keyboard", () => {
        it("moves the cursor onto the first group row, which exists before anything is expanded", async () => {
            const {container} = mount();
            await press("j");

            // The whole reason group headings are cursorable: with everything
            // collapsed there is no account row for `j` to land on, and a cursor
            // that does nothing on a freshly-loaded report is a broken cursor.
            const current = container.querySelector("[aria-current='true']");
            expect(current?.textContent).toContain("Dividends"); // groups sort by name
        });

        it("expands the cursored group on Enter", async () => {
            const {container} = mount();
            await press("j");
            await press("Enter");

            expect(container.querySelector('[data-account="income:dividends"]')).not.toBeNull();
            expect(openJournal).not.toHaveBeenCalled();
        });

        it("drills the cursored account into the journal, narrowed to the report's own range", async () => {
            mount();
            await press("j");
            await press("Enter"); // expand Dividends, the first group by name
            await press("j"); // onto income:dividends
            await press("Enter");

            // The range, not `preset: "all"`: unlike the balance sheet's as-of
            // date, a P&L's window IS the report, so widening the drill-down would
            // show postings that are not in the number that was clicked.
            expect(openJournal).toHaveBeenCalledWith({accounts: ["income:dividends"], from: "2026-01-01", to: "2026-07-08"});
        });

        it("never lands on a subtotal, which has nothing to do on Enter", async () => {
            const {container} = mount(MULTI);
            for (let i = 0; i < 40; i += 1) await press("j");

            expect(container.querySelector("[aria-current='true']")?.textContent).not.toContain("Gross profit");
            expect(screen.getByTestId("is-subtotal-grossProfit").querySelector("[aria-current='true']")).toBeNull();
        });

        it("walks to the last row with G and back to the first with gg", async () => {
            const {container} = mount();
            await press("G");
            expect(container.querySelector("[aria-current='true']")?.textContent).toContain("Utilities");

            await press("g");
            await press("g");
            expect(container.querySelector("[aria-current='true']")?.textContent).toContain("Dividends");
        });

        it("clears the cursor on Escape", async () => {
            const {container} = mount();
            await press("j");
            await press("Escape");

            expect(container.querySelector("[aria-current='true']")).toBeNull();
        });
    });

    describe("the bottom line", () => {
        it("shows net income prominently, with its prior figure and its margin", () => {
            mount();
            const box = screen.getByTestId("is-net-income");

            expect(box.textContent).toContain("Net income");
            expect(box.textContent).toContain("$5,383.52");
            expect(box.textContent).toContain("$10,880.79"); // the prior period's
            expect(box.textContent).toContain("15.8%");
        });

        // REGRESSION: this panel rendered prior BEFORE current, so it read
        // "$10,880.79  $5,383.52" — inviting anyone who had just read the
        // Amount / Prior / % headers above to take net income as $10,880.79 when
        // the period earned $5,383.52. `toContain` above cannot see order, and
        // this is the one place on the page with no column header to disambiguate,
        // so pin the sequence — and pin it against the workbook's order too, since
        // `setIsAmounts` writes current into Amount and prior into Prior.
        it("orders the bottom line current-then-prior, like the columns and the export", () => {
            mount();
            const text = screen.getByTestId("is-net-income").textContent ?? "";

            expect(text.indexOf("$5,383.52")).toBeLessThan(text.indexOf("$10,880.79"));
            expect(text.indexOf("$10,880.79")).toBeLessThan(text.indexOf("15.8%"));
        });

        it("restates nothing the boxes already showed", () => {
            mount(MULTI);

            // A condensed "Total Revenue / Less: Cost of revenue / …" table used
            // to sit here. Every one of those figures is already in a box footer
            // directly above, and every intermediate one is already a rung of the
            // ladder, so the block was seven duplicated totals — the exact
            // complaint this redesign exists to fix, one panel further down.
            expect(screen.queryByTestId("is-summary")).toBeNull();
            expect(screen.queryByText("Less: Cost of revenue")).toBeNull();
            expect(screen.queryByText("Less: Operating expenses")).toBeNull();

            // Each section total therefore appears exactly ONCE on the page, in
            // its own box's footer.
            const bodyText = screen.getByTestId("income-statement").textContent ?? "";
            expect(bodyText.match(/\$356,500\.00/g)).toHaveLength(1);
            expect(bodyText.match(/\$102,600\.00/g)).toHaveLength(1);
        });

        it("is the one figure on the page that is not already in a box or on the ladder", () => {
            mount(MULTI);
            const bodyText = screen.getByTestId("income-statement").textContent ?? "";

            expect(bodyText.match(/\$86,500\.00/g)).toHaveLength(1);
            expect(screen.getByTestId("is-net-income").textContent).toContain("$86,500.00");
            expect(screen.getByTestId("is-net-income").textContent).toContain("14.0%");
        });

        it("shows a loss as a negative, flagged for the caller to paint", () => {
            const loss = {...REPORT, netIncome: {current: new Map([["$", {m: -50000n, p: 2}]]), prior: new Map()}};
            mount(loss);

            expect(screen.getByTestId("is-net-income").textContent).toContain("$-500.00");
            expect(screen.getByTestId("is-net-income").querySelector(".text-error")).not.toBeNull();
        });
    });
});
