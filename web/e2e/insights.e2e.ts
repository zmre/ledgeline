// E2E for the Insights dashboard against the REAL stack: the ledgeline engine
// serving fixtures/sample.journal on :5099 (playwright.config.ts webServer) with
// the built SPA in front. Same pinned clock + localStorage seeding as the other
// suites.
//
// At the pinned clock (2026-07-08) the default "Year over year" preset resolves
// to the 24 complete months 2024-07-01 … 2026-06-30, which the engine splits on
// the calendar boundary into two clean 12-month halves:
//   previous 2024-07-01 … 2025-06-30, current 2025-07-01 … 2026-06-30.
//
// Fixture facts for that split, verified against the live /api/insights response:
//   - revenue      $68,007.50 (prev $64,843.75, +4.9%)
//   - expenses     $44,908.85 (+ EUR 933.25 as a secondary commodity line)
//   - cash balance $50,277.56
//   - cost of living $27,388.85 over 12 months = $2,282.40/mo — i.e. expenses
//     MINUS the default tax exclusion (federal $13,800 + state $3,720)
//   - biggest expense changes are LEAF accounts only (no parent rollups)
//   - movers: AAPL +58.6%, VTI +31.0% (windowed over the current half)
//   - top transactions: the $5,660 Acme Corp salary deposits

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
        },
        [API_URL, API_TOKEN]
    );
});

test("insights: is the default reports tab and renders the core metric boxes", async ({page}) => {
    await page.goto("/reports");

    // Insights is the landing tab — no ?tab= needed.
    await expect(page.getByRole("tab", {name: "Insights"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("insights-dashboard")).toBeVisible();

    // The comparison window is stated as current-vs-previous halves.
    await expect(page.getByTestId("insights-dashboard")).toContainText("2025-07-01");
    await expect(page.getByTestId("insights-dashboard")).toContainText("2026-06-30");
    await expect(page.getByTestId("insights-dashboard")).toContainText("2024-07-01");

    // Boxes 1-3 headline figures.
    await expect(page.getByTestId("insights-box-revenue-big")).toHaveText("$68,007.50");
    await expect(page.getByTestId("insights-box-expenses-big")).toHaveText("$48,408.85");
    await expect(page.getByTestId("insights-box-networth")).toBeVisible();

    // Box 4: cost of living is the monthly average EXCLUDING taxes, so it is well
    // below the raw monthly expense rate ($48,408.85 / 12 = $4,034).
    await expect(page.getByTestId("insights-box-costofliving-big")).toHaveText("$2,574.07");

    // Box 6: cash balance at the end of the current period.
    await expect(page.getByTestId("insights-box-cash-big")).toHaveText("$50,277.56");

    // Revenue rose, so its delta line is the "good" (success) colour with a percent.
    const revenueDelta = page.getByTestId("insights-box-revenue").locator(".text-success");
    await expect(revenueDelta).toContainText("4.9%");
});

test("insights: list boxes rank leaf accounts, movers, and top transactions", async ({page}) => {
    await page.goto("/reports");

    // Boxes 7 & 9: leaf accounts only — a parent rollup like "expenses:taxes"
    // must never appear alongside its children.
    const expenseChanges = page.getByTestId("insights-box-expensechanges");
    await expect(expenseChanges).toContainText("rent");
    await expect(expenseChanges).toContainText("federal");
    const revenueChanges = page.getByTestId("insights-box-revenuechanges");
    await expect(revenueChanges).toContainText("salary");

    // Box 8: both priced holdings moved up over the current half.
    const movers = page.getByTestId("insights-box-movers");
    await expect(movers).toContainText("AAPL");
    await expect(movers).toContainText("VTI");

    // Box 10: the largest transactions by money moved are the salary deposits.
    const topTxns = page.getByTestId("insights-box-toptxns");
    await expect(topTxns).toContainText("Acme Corp");
    await expect(topTxns).toContainText("$5,660.00");
});

test("insights: change lists rank by money moved and hide categories with no prior history", async ({page}) => {
    await page.goto("/reports");

    const expenseChanges = page.getByTestId("insights-box-expensechanges");
    // Ranked by SIZE OF THE MOVE, so federal tax (+$1,200) leads rent (+$825) —
    // and a category with no previous-period activity is never listed at all.
    const rows = expenseChanges.locator("li");
    await expect(rows.first()).toContainText("federal");
    await expect(expenseChanges).not.toContainText("new");

    // Revenue: salary (+$3,120) outranks dividends (+$43.75) despite dividends
    // having the larger percentage change (+100%).
    const revenueRows = page.getByTestId("insights-box-revenuechanges").locator("li");
    await expect(revenueRows.first()).toContainText("salary");
});

test("insights: a span with no prior activity says so instead of showing nothing", async ({page}) => {
    // Both halves predate the fixture journal entirely (it starts 2024-07).
    await page.goto("/reports?tab=insights&istart=2020-01-01&iend=2021-12-31");

    await expect(page.getByTestId("insights-box-expensechanges")).toContainText("Not enough history");
    await expect(page.getByTestId("insights-box-revenuechanges")).toContainText("Not enough history");
});

test("insights: changing the preset refetches and round-trips through the URL", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("insights-box-revenue-big")).toHaveText("$68,007.50");

    // Month over month: a 2-month span split into two 1-month halves.
    await page.getByLabel("Comparison preset").selectOption("mom");
    await expect(page.getByTestId("insights-dashboard")).toContainText("2026-06-01");

    // The span lives in the URL (debounced replaceState), so a reload reproduces it.
    await expect(page).toHaveURL(/istart=2026-05-01/);
    await expect(page).toHaveURL(/iend=2026-06-30/);
});

test("insights: switching to another tab still renders that report", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("insights-dashboard")).toBeVisible();

    await page.getByRole("tab", {name: "Balance Sheet"}).click();
    await expect(page.getByTestId("insights-dashboard")).toHaveCount(0);
    // The grouped balance sheet takes over: three boxes, groups collapsed, each
    // box's total in a `<tfoot>` row header. Account names are NOT visible yet —
    // they are behind the disclosures (plans/12).
    await expect(page.getByTestId("balance-sheet")).toBeVisible();
    const assets = page.getByTestId("bs-section-assets");
    await expect(assets).toBeVisible();
    // Scoped to the box: the summary and the tie-out below carry "Total Assets"
    // too, so the bare role locator matches three rows and fails strict mode.
    await expect(assets.getByRole("rowheader", {name: "Total Assets"})).toBeVisible();
});
