// E2E smoke against the REAL stack (WP-09): hledger-web serving
// fixtures/sample.journal on :5099 (see playwright.config.ts webServer) with
// the built SPA in front of it.
//
// Fixture facts, verified against hledger 1.52 CLI (see plans/09):
//   - 185 transactions total; 27 fall in the last-90 window 2026-04-10..2026-07-08
//     at the pinned clock (journal spans 2024-07-01..2026-07-04)
//   - `hledger bal expenses -b 2026-04-10 -e 2026-07-09` → $11,526.62 (+ 228,75 EUR);
//     the footer shows the negated primary-commodity net, $-11,526.62
//   - deepest account is 4 segments (assets:broker:taxable:vti) → depth-slider max 4
//   - `hledger bs --depth 2 -e 2026-07-09` (CLI -e is exclusive ≙ our asOf 2026-07-08):
//     Total Assets $48,402.56 (+ 19.5 AAPL + 566,75 EUR + 5 GLD − 2 TSLA + 17 VTI),
//     Net $47,871.41
//   - 6 deliberate problem records: pending txn, expenses:unknown, empty description,
//     GLD missing basis, GLD unpriced, TSLA negative shares (WP-10)
//
// The clock is pinned to 2026-07-08 so the last-90 default preset, the reports
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

test("journal: last-90 default preset filters, all-time shows the full journal", async ({page}) => {
    await page.goto("/");

    // Default preset is "last 90 days" (defaultFilter → last90); at the pinned
    // clock that's 2026-04-10 … 2026-07-08. The table is virtualized (row count
    // is viewport-bound), so the TotalsFooter is the source of truth for counts.
    const footer = page.locator("footer");
    await expect(footer).toContainText("27 transactions");
    await expect(footer).toContainText("2026-04-10 – 2026-07-08");
    await expect(page.locator("tbody tr").first()).toBeVisible();

    await page.locator("summary").filter({hasText: "Last 90 days"}).click();
    await page.getByRole("button", {name: "All time"}).click();
    await expect(footer).toContainText("185 transactions");
    await expect(footer).toContainText("all dates");
});

test("journal: selecting the expenses subtree nets the totals footer", async ({page}) => {
    await page.goto("/");

    const footer = page.locator("footer");
    await expect(footer).toContainText("27 transactions"); // journal loaded, last-90 filter active

    await page.locator("summary").filter({hasText: "Accounts"}).click();
    await page.getByRole("checkbox", {name: "expenses", exact: true}).check();

    // Visible Journal Total = net of the selected expenses postings over the
    // last-90 window, shown negative (money spent). The footer reports the primary
    // (most-used) commodity only — $ here — verified vs
    // `hledger bal expenses -b 2026-04-10 -e 2026-07-09` ($11,526.62).
    await expect(footer).toContainText("$-11,526.62");
});

test("journal: insights depth slider starts at the default, not browser-clamped (regression)", async ({page}) => {
    await page.goto("/");

    // The insights panel mounts before the journal finishes loading, when the max
    // account depth is still 1; the browser clamps the range input to that max.
    // Once the real accounts arrive (fixture's deepest is 4) the slider must
    // re-apply its default (2) rather than stay stuck at 1.
    const slider = page.locator('input[aria-label="Account depth"]');
    await expect(slider).toHaveAttribute("max", "4"); // real max loaded, not the initial 1
    await expect(slider).toHaveValue("2"); // default depth, matching the chart
});

test("reports: balance sheet shows known fixture numbers", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Reports"}).click();

    // Default balance-sheet params with the pinned clock: asOf 2026-07-08, depth 2.
    await expect(page.locator("tr", {has: page.locator('th:text-is("Total Assets")')})).toContainText("$48,402.56");
    await expect(page.locator("tr", {has: page.locator('th:text-is("Net")')})).toContainText("$47,871.41");
});

test("problems badge shows the deliberate problem count", async ({page}) => {
    await page.goto("/");

    // WP-09's three deliberate problems + WP-10's three stock warnings
    // (no future-dated txns at the pinned clock).
    await expect(page.getByRole("button", {name: "6 problems"})).toBeVisible();
});
