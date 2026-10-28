// Reports › Accounts (2026-10-28): every account per period, each section read
// its own way, over cash flow's window — read-only, like every report spec.

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

test("reports: the Accounts tab shows balances for assets and changes for flows, over cash flow's window", async ({page}) => {
    await page.goto("/reports?tab=accounts&from=2026-01-01&to=2026-06-30&interval=monthly&depth=3");
    await expect(page.getByRole("tab", {name: "Accounts"})).toHaveAttribute("aria-selected", "true");
    const table = page.getByTestId("accounts-table");
    await expect(table).toBeVisible();
    await expect(table.locator("thead tr:not([data-testid]) th")).toHaveCount(8); // Account, six months, last column
    const assets = table.locator('[data-testid="account-section"][data-section="assets"]');
    await expect(assets).toHaveAttribute("data-reading", "balance");
    await expect(assets.locator('[data-column="summary-heading"]')).toHaveText("Δ over the period");
    const expenses = table.locator('[data-testid="account-section"][data-section="expenses"]');
    await expect(expenses).toHaveAttribute("data-reading", "change");
    // No Actual/Projected band on Reports.
    await expect(table.getByTestId("projected-label")).toHaveCount(0);

    // Empty accounts on request, in the URL.
    await expect(table.locator('[data-account="expenses:shopping"]')).toHaveCount(0);
    await page.getByTestId("accounts-empty").check();
    await expect(page).toHaveURL(/empty=1/);
    await expect(table.locator('[data-account="expenses:shopping"]')).toHaveCount(1);

    // Switching to Cash Flow keeps the range: the two share one window.
    await page.getByRole("tab", {name: "Cash Flow"}).click();
    await expect(page).toHaveURL(/from=2026-01-01&to=2026-06-30/);

    // And it downloads.
    await page.getByRole("tab", {name: "Accounts"}).click();
    await expect(table).toBeVisible();
    const [download] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", {name: "Export report as xlsx"}).click()]);
    expect(download.suggestedFilename()).toBe("accounts-2026-01-01-to-2026-06-30-monthly.xlsx");
});
