// The grouped balance sheet, mounted.
//
// The first mounted test under `lib/reports/`, and it exists for the reason the
// vite config gives for having a `components` project at all: the row-model
// tests can prove `sectionDisplayRows` returns two rows for a collapsed section
// and still say nothing about whether the screen renders them, whether the
// disclosure is wired to anything, or whether pressing `j` reaches a row that
// exists.
//
// jsdom has no layout engine, so nothing here asks how anything LOOKS. It asks
// what is in the document, what `aria-expanded` says, and what Enter dispatches.

import {render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {decodeBalanceSheetReport} from "$lib/api/nativeDecode";
import type {AmountStyle} from "$lib/domain/types";
import {keymap} from "$lib/keys/keymap.svelte";
import {GROUPED_BALANCE_SHEET, UNBALANCED_BALANCE_SHEET} from "$lib/testing/balanceSheetFixture";
import BalanceSheetView from "./BalanceSheetView.svelte";

// The drill-down navigates, and a router is neither available nor the subject
// here — what matters is that Enter on an account row asks for THAT account.
const openJournal = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("$lib/journal/openJournal", () => ({openJournal}));

const STYLES: ReadonlyMap<string, AmountStyle> = new Map<string, AmountStyle>([
    ["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}],
    ["GLD", {side: "R", spaced: true, precision: 1, decimalPoint: ".", digitGroups: null}],
    ["TSLA", {side: "R", spaced: true, precision: 1, decimalPoint: ".", digitGroups: null}],
    ["EUR", {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]}],
]);

const REPORT = decodeBalanceSheetReport(GROUPED_BALANCE_SHEET);

const mount = (report = REPORT) => render(BalanceSheetView, {report, styles: STYLES});

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

describe("COMPONENT BalanceSheetView", () => {
    it("renders three separate boxes, one per section", () => {
        mount();

        for (const kind of ["assets", "liabilities", "equity"]) {
            expect(screen.getByTestId(`bs-section-${kind}`)).toBeDefined();
        }
        expect(screen.getByRole("heading", {name: "Assets"})).toBeDefined();
    });

    it("puts each box's total in a real tfoot row", () => {
        mount();
        const foot = screen.getByTestId("bs-section-assets").querySelector("tfoot");

        expect(foot).not.toBeNull();
        // $59,612.615 → half away from zero, the app-wide money convention.
        expect(foot?.textContent).toContain("Total Assets");
        expect(foot?.textContent).toContain("$59,612.62");
    });

    it("demotes the unvalued commodities to a secondary line instead of stacking balances", () => {
        mount();
        const foot = screen.getByTestId("bs-section-assets").querySelector("tfoot");

        // The old table rendered one <div> per commodity inside the cell — three
        // numbers in the space of one. Here there is one figure and a footnote.
        expect(foot?.textContent).toContain("5.0 GLD · -2.0 TSLA");
    });

    describe("groups are collapsed by default", () => {
        it("shows the group headings and none of their accounts", () => {
            const {container} = mount();

            expect(screen.getByText("Cash and cash equivalents")).toBeDefined();
            expect(screen.queryByText("checking")).toBeNull();
            // `data-account` marks account rows specifically; there should be none.
            expect(container.querySelectorAll("[data-account]")).toHaveLength(0);
        });

        it("still shows every group subtotal, so a collapsed sheet is a complete one", () => {
            mount();
            const assets = screen.getByTestId("bs-section-assets");

            expect(assets.textContent).toContain("$42,450.24"); // Cash
            expect(assets.textContent).toContain("$17,162.38"); // Investments
        });

        it("reports its state through aria-expanded", () => {
            mount();
            expect(disclosure("Cash and cash equivalents").getAttribute("aria-expanded")).toBe("false");
        });
    });

    describe("expanding a group", () => {
        it("reveals its accounts at full depth, each tagged with its account name", async () => {
            const {container} = mount();
            disclosure("Cash and cash equivalents").click();
            await tick();

            const accounts = [...container.querySelectorAll("[data-account]")].map((el) => el.getAttribute("data-account"));
            // `assets:bank:wise:eur`, not `assets:bank:wise`: the tab asks for an
            // unclamped report and the empty parent folds into it (compressSectionRows).
            expect(accounts).toEqual(["assets:bank", "assets:bank:checking", "assets:bank:savings", "assets:bank:wise:eur"]);
            expect(disclosure("Cash and cash equivalents").getAttribute("aria-expanded")).toBe("true");
        });

        it("keeps the rows clear of the sticky chrome the cursor scrolls them under", async () => {
            const {container} = mount();
            disclosure("Cash and cash equivalents").click();
            await tick();

            // `scroll-mt-10` is what makes `scrollIntoView({block: "nearest"})`
            // land below the pinned header rather than underneath it. jsdom
            // cannot see the effect, only that the contract is still declared.
            expect(container.querySelector('[data-account="assets:bank:checking"]')?.classList.contains("scroll-mt-10")).toBe(true);
        });

        it("leaves its neighbours closed", async () => {
            const {container} = mount();
            disclosure("Cash and cash equivalents").click();
            await tick();

            expect(disclosure("Investments").getAttribute("aria-expanded")).toBe("false");
            expect(container.querySelector('[data-account="assets:broker:taxable"]')).toBeNull();
        });

        it("collapses again on a second click", async () => {
            const {container} = mount();
            disclosure("Cash and cash equivalents").click();
            await tick();
            disclosure("Cash and cash equivalents").click();
            await tick();

            expect(container.querySelectorAll("[data-account]")).toHaveLength(0);
        });

        it("gives a computed group no disclosure at all", () => {
            mount();
            // "Retained earnings" summarizes accounts that are not on the balance
            // sheet: a total and no rows. A triangle that opens onto nothing is
            // worse than no triangle.
            expect(screen.getByText("Retained earnings")).toBeDefined();
            expect(screen.queryByRole("button", {name: "Retained earnings"})).toBeNull();
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
            expect(current?.textContent).toContain("Cash and cash equivalents");
        });

        it("expands the cursored group on Enter", async () => {
            const {container} = mount();
            await press("j");
            await press("Enter");

            expect(container.querySelector('[data-account="assets:bank:checking"]')).not.toBeNull();
            expect(openJournal).not.toHaveBeenCalled();
        });

        it("drills the cursored account into the journal on Enter", async () => {
            mount();
            await press("j");
            await press("Enter"); // expand the cash group
            await press("j"); // onto assets:bank
            await press("Enter");

            // `preset: "all"`, not the report's as-of date: that date lives in the
            // controls, not in the row, so narrowing to it would read as data loss.
            expect(openJournal).toHaveBeenCalledWith({accounts: ["assets:bank"], preset: "all"});
        });

        it("walks to the last row with G and back to the first with gg", async () => {
            const {container} = mount();
            await press("G");
            expect(container.querySelector("[aria-current='true']")?.textContent).toContain("Valuation adjustment");

            await press("g");
            await press("g");
            expect(container.querySelector("[aria-current='true']")?.textContent).toContain("Cash and cash equivalents");
        });

        it("clears the cursor on Escape", async () => {
            const {container} = mount();
            await press("j");
            await press("Escape");

            expect(container.querySelector("[aria-current='true']")).toBeNull();
        });
    });

    describe("the tie-out, net worth and the balance check", () => {
        /**
         * The summary block's rows as `[label, headline figure]`, in visual
         * order. The headline is the FIRST span in the amount cell — the
         * unpriced-commodity footnote is a second one below it.
         */
        const summaryRows = (): [string, string][] =>
            [...screen.getByTestId("bs-summary").querySelectorAll("tr")].map((tr) => [
                tr.querySelector("th")?.textContent?.trim() ?? "",
                tr.querySelector("td span")?.textContent?.trim() ?? "",
            ]);

        it("proves the statement balances instead of restating one number twice", () => {
            mount();

            // `Total equity ≡ Assets − Liabilities ≡ Net worth`, so a panel
            // showing net worth alone proved nothing. `L + E` against `A` is the
            // check a reader of a balance sheet actually performs.
            expect(summaryRows().map(([label]) => label)).toEqual([
                "Total Assets",
                "Total Liabilities",
                "Total Equity",
                "Liabilities + Equity",
                "Total Assets",
            ]);
            expect(summaryRows()).toContainEqual(["Total Liabilities", "$531.15"]);
        });

        it("adds Liabilities + equity from the exact Decs, not from the rendered strings", () => {
            mount();
            const tieOut = screen.getByTestId("bs-tie-out");

            // $531.15 + ($59,081.465 + 5 GLD − 2 TSLA) = $59,612.615 + the same
            // holdings — which is Total assets to the last half-cent. Re-adding
            // the DISPLAYED $531.15 and $59,081.47 gives $59,612.62 by luck here
            // and by nothing at all in general.
            const rows = summaryRows();
            expect(rows[3]).toEqual(["Liabilities + Equity", "$59,612.62"]);
            expect(tieOut.textContent).toContain("$59,612.62");
            expect(screen.getByTestId("bs-summary").textContent).toContain("5.0 GLD · -2.0 TSLA");
        });

        it("hangs the verdict off the tie-out", () => {
            mount();
            expect(screen.getByTestId("bs-tie-out").textContent).toContain("Balanced");
            expect(screen.getByTestId("bs-tie-out").textContent).not.toContain("Out of balance");
        });

        it("marks the tie-out failed on a half-cent imbalance the display cannot show", () => {
            mount(decodeBalanceSheetReport(UNBALANCED_BALANCE_SHEET));
            // $0.005: both tie-out figures still print $59,612.62, so a verdict
            // read off the rendered strings would say "Balanced" here.
            expect(screen.getByTestId("bs-tie-out").textContent).toContain("Out of balance");
        });

        it("keeps net worth prominent, with its own unvalued footnote", () => {
            mount();
            const box = screen.getByTestId("bs-net-worth");

            expect(box.textContent).toContain("Net worth");
            expect(box.textContent).toContain("$59,081.47"); // $59,081.465
            expect(box.textContent).toContain("5.0 GLD · -2.0 TSLA");
        });

        it("says nothing at all when the journal balances", () => {
            mount();
            // `check` is `{}`. A permanent "balance: OK" BANNER trains the reader
            // to ignore the one place a real failure would appear; the tie-out's
            // ✓ is a different thing — it is the statement's own arithmetic.
            expect(screen.queryByTestId("bs-check")).toBeNull();
        });

        it("warns, with the figure, when it does not", () => {
            mount(decodeBalanceSheetReport(UNBALANCED_BALANCE_SHEET));
            const alert = screen.getByTestId("bs-check");

            expect(within(alert).getByText(/doesn't balance/)).toBeDefined();
            expect(alert.getAttribute("role")).toBe("alert");
            // Half a cent — invisible at the display cap, so the decision to show
            // this at all was made on the exact Dec, not on the rendered string.
            expect(alert.textContent).toContain("$0.01");
        });

        it("stays quiet on sub-cent cost dust, which is not an imbalance", () => {
            // A non-empty `check` with `balanced: true` — what the engine sends
            // for any journal holding fractional lots, because `quantity × price`
            // carries more decimal places than the cash leg can be written to.
            // Deciding locally with `maIsZero(check)` put a permanent, wrong
            // "this journal doesn't balance" warning on a valid journal.
            mount(decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), check: {$: {mantissa: "22797", places: 7}}, balanced: true}));

            expect(screen.queryByTestId("bs-check")).toBeNull();
            expect(screen.getByTestId("bs-tie-out").textContent).toContain("Balanced");
            expect(screen.getByTestId("bs-tie-out").textContent).not.toContain("Out of balance");
        });
    });
});
