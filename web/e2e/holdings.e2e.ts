// E2E for the holdings tab (WP-10) against the REAL stack: hledger-web serving
// fixtures/sample.journal on :5099 (playwright.config.ts webServer) with the
// built SPA in front. Same pinned clock + localStorage seeding as smoke.e2e.ts.
//
// Fixture facts, verified against hledger 1.52 CLI (see plans/09 + 10):
//   - AAPL (name: Apple Inc.): 19.5 sh, basis $4,346.10, P 2026-06-30 $270.25 → $5,269.88
//     (at asOf 2025-01-01: 10 sh — the 5-sh and 4.5-sh buys are 2025-05-20/2026-03-10 —
//     priced by P 2024-09-30 $228.00)
//   - VTI (assets:broker:taxable:vti): 17 sh, remaining basis $4,693.36, P 2026-06-30
//     $310.75 → $5,282.75; first buy is 2025-02-20, so VTI is absent at 2025-01-01
//   - GLD: 5 sh gifted 2025-08-20 with no cost annotation and no P price → null basis,
//     unpriced; NVDA fully sold 2026-01-20; TSLA net -2 sh (never bought) → row hidden
//   - default scope (GLD in scope) ⇒ totals basis/gain are NULL (honest-totals rule):
//     market value $10,552.63 with em-dash basis/gain and inline warnings

import {expect, test, type Page} from "@playwright/test";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

const EM_DASH = "—";

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW); // Date is fake, timers keep running (URL-sync debounce)
    await page.addInitScript((url) => {
        localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url}));
    }, API_URL);
});

function stat(page: Page, title: string) {
    return page.locator('[data-testid="holdings-stats"] .stat', {hasText: title});
}

test("holdings: table shows fixture holdings with honest (null) totals", async ({page}) => {
    await page.goto("/holdings");

    // AAPL row: name tag, shares, directive price + date, market value.
    const aapl = page.getByTestId("holding-AAPL");
    await expect(aapl).toContainText("Apple Inc.");
    await expect(page.getByTestId("shares-AAPL")).toHaveText("19.5");
    await expect(aapl).toContainText("$270.25");
    await expect(aapl).toContainText("2026-06-30");
    await expect(aapl).toContainText("$5,269.88");

    // VTI row: shares and average-cost basis after the partial sell.
    const vti = page.getByTestId("holding-VTI");
    await expect(vti).toContainText("Vanguard Total Market");
    await expect(page.getByTestId("shares-VTI")).toHaveText("17");
    await expect(vti).toContainText("$4,693.36");
    await expect(vti).toContainText("$310.75");

    // GLD row present with em-dash basis (gifted without a cost annotation).
    const gld = page.getByTestId("holding-GLD");
    await expect(page.getByTestId("shares-GLD")).toHaveText("5");
    await expect(gld).toContainText(EM_DASH);

    // Fully-sold and negative symbols never render.
    await expect(page.getByTestId("holding-NVDA")).toHaveCount(0);
    await expect(page.getByTestId("holding-TSLA")).toHaveCount(0);

    // Both priced holdings are gainers (AAPL +21.3%, VTI +12.6%): only the
    // gainers list renders — an empty losers list is hidden individually.
    const gainers = page.getByTestId("top-gainers");
    await expect(gainers).toContainText("AAPL");
    await expect(gainers).toContainText("VTI");
    await expect(page.getByTestId("top-losers")).toHaveCount(0);

    // Partial totals: GLD (tainted+unpriced) is excluded; cost basis + gain sum the
    // known holdings (AAPL + VTI) and a note names the excluded row. Warnings explain.
    await expect(stat(page, "Market value")).toContainText("$10,552.63");
    await expect(stat(page, "Cost basis")).toContainText("$9,039.46");
    await expect(stat(page, "Unrealized gain %")).toContainText("+16.7%");
    const warnings = page.getByTestId("holdings-warnings");
    await expect(warnings).toContainText("GLD");
    await expect(warnings).toContainText("TSLA");
});

test("holdings: excluding the VTI account removes VTI and shrinks the pie and stats", async ({page}) => {
    await page.goto("/holdings");
    await expect(page.getByTestId("holding-VTI")).toBeVisible();

    // Select the VTI account, then flip the mode to exclude — selection is kept.
    await page.locator("summary").filter({hasText: "Accounts"}).click();
    await page.getByRole("checkbox", {name: "vti", exact: true}).check();
    await page.getByRole("button", {name: "All except"}).click();

    await expect(page.getByTestId("holding-VTI")).toHaveCount(0);
    await expect(page.getByTestId("holding-AAPL")).toBeVisible();
    await expect(stat(page, "Market value")).toContainText("$5,269.88");
    // Only one priced holding left → the pie loses the slice and gainers/losers hide.
    await expect(page.getByTestId("holdings-pie-legend")).not.toContainText("VTI");
    await expect(page.getByTestId("gainers-losers")).toHaveCount(0);
});

test("holdings: as-of time travel recomputes shares, prices, and warnings", async ({page}) => {
    await page.goto("/holdings");
    await expect(page.getByTestId("shares-AAPL")).toHaveText("19.5");

    await page.getByLabel("As of date").fill("2025-01-01");

    // Only the first AAPL buy has happened; the 2024-09-30 P directive prices it.
    await expect(page.getByTestId("shares-AAPL")).toHaveText("10");
    await expect(page.getByTestId("holding-AAPL")).toContainText("$228.00");
    await expect(page.getByTestId("holding-AAPL")).toContainText("2024-09-30");
    // VTI's first buy is 2025-02-20 and GLD arrives 2025-08-20 — neither exists yet.
    await expect(page.getByTestId("holding-VTI")).toHaveCount(0);
    await expect(page.getByTestId("holding-GLD")).toHaveCount(0);
    // Clean scope: totals become real numbers and the warnings disappear.
    await expect(stat(page, "Cost basis")).toContainText("$2,200.00");
    await expect(page.getByTestId("holdings-warnings")).toHaveCount(0);
});

test("holdings: value-over-time trend renders and time-travel shrinks the window", async ({page}) => {
    await page.goto("/holdings");
    // Priced holdings exist within the trailing 12 months at the pinned clock → the chart draws.
    await expect(page.getByTestId("holdings-trend")).toBeVisible();

    // Time-travel before any position was opened: the 12-month window is all zero → empty-state copy, no chart.
    await page.getByLabel("As of date").fill("2024-01-01");
    await expect(page.getByTestId("holdings-empty")).toBeVisible();
});

test("holdings: nav link works and the problems badge still shows 6", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Holdings"}).click();
    await expect(page).toHaveTitle("Ledgeline — Holdings");
    await expect(page.getByTestId("holdings-table")).toBeVisible();
    await expect(page.getByRole("button", {name: "6 problems"})).toBeVisible();
});
