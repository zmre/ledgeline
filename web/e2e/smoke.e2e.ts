// E2E smoke against the REAL stack (WP-09): hledger-web serving
// fixtures/sample.journal on :5099 (see playwright.config.ts webServer) with
// the built SPA in front of it.
//
// Fixture facts, verified against hledger 1.52 CLI (see plans/09):
//   - 176 transactions total, 3 of them in 2026-07 (journal spans 2024-07-01..2026-07-04)
//   - `hledger bal expenses -b 2026-07-01 -e 2026-08-01` → $2,344.04
//   - `hledger bs --depth 2 -e 2026-07-09` (CLI -e is exclusive ≙ our asOf 2026-07-08):
//     Total Assets $52,077.96 (+ 19.5 AAPL + 566,75 EUR), Net $51,546.81
//   - 3 deliberate problem records (pending txn, expenses:unknown, empty description)
//
// The clock is pinned to 2026-07-08 so the "this month" preset, the reports
// default as-of date, and the future-date check stay glued to those facts.

import {expect, test} from "@playwright/test";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW); // Date is fake, timers keep running (URL-sync debounce, polling)
    await page.addInitScript((url) => {
        localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url}));
    }, API_URL);
});

test("journal: this-month preset filters, all-time shows the full journal", async ({page}) => {
    await page.goto("/");

    // Default preset is "this month" — the table is virtualized (row count is
    // viewport-bound), so the TotalsFooter is the source of truth for counts.
    const footer = page.locator("footer");
    await expect(footer).toContainText("3 transactions");
    await expect(footer).toContainText("2026-07-01 – 2026-07-31");
    await expect(page.locator("tbody tr").first()).toBeVisible();

    await page.locator("summary").filter({hasText: "This month"}).click();
    await page.getByRole("button", {name: "All time"}).click();
    await expect(footer).toContainText("176 transactions");
    await expect(footer).toContainText("all dates");
});

test("journal: selecting the expenses subtree narrows the totals footer", async ({page}) => {
    await page.goto("/");

    const footer = page.locator("footer");
    await expect(footer).toContainText("3 transactions"); // journal loaded, July filter active

    await page.locator("summary").filter({hasText: "Accounts"}).click();
    await page.getByRole("checkbox", {name: "expenses", exact: true}).check();

    // Sum of expenses postings in 2026-07 (verified vs `hledger bal`).
    await expect(footer).toContainText("$2,344.04");
});

test("reports: balance sheet shows known fixture numbers", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Reports"}).click();

    // Default balance-sheet params with the pinned clock: asOf 2026-07-08, depth 2.
    await expect(page.locator("tr", {has: page.locator('th:text-is("Total Assets")')})).toContainText("$52,077.96");
    await expect(page.locator("tr", {has: page.locator('th:text-is("Net")')})).toContainText("$51,546.81");
});

test("problems badge shows the deliberate problem count", async ({page}) => {
    await page.goto("/");

    // WP-09's three deliberate problems (no future-dated txns at the pinned clock).
    await expect(page.getByRole("button", {name: "3 problems"})).toBeVisible();
});
