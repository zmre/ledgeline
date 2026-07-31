// App-shell smoke (WP-01, updated by WP-09 for the real pages): theme,
// first-run setup modal, and navigation. With no stored server URL the setup
// modal overlays everything, so the nav tests seed one (the fixture API from
// playwright.config.ts webServer).
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

// This was the one suite left running against the real wall clock, so its footer
// assertions drifted with the calendar while every sibling spec was pinned.
test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW); // Date is fake, timers keep running (URL-sync debounce)
});

async function seedServerUrl(page: Page): Promise<void> {
    await page.addInitScript((token) => {
        localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: "http://127.0.0.1:5099", serverToken: token}));
    }, API_TOKEN);
}

test("dark theme is the default", async ({page}) => {
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("first run prompts for a server URL", async ({page}) => {
    await page.goto("/");
    await expect(page.getByRole("heading", {name: "Connect to hledger-web"})).toBeVisible();
});

test("navigates between journal and reports", async ({page}) => {
    await seedServerUrl(page);
    await page.goto("/");
    await expect(page).toHaveTitle("Ledgeline — Journal");
    await expect(page.locator("footer")).toContainText("transactions");

    await page.getByRole("link", {name: "Reports"}).click();
    await expect(page).toHaveTitle("Ledgeline — Reports");
    await expect(page.getByRole("tab", {name: "Balance Sheet"})).toBeVisible();

    // The Imports item is present because this engine HAS `/api/rules`. On an
    // older engine that route 404s and the item is hidden rather than leading to
    // a screen that can only apologize.
    await page.getByRole("link", {name: "Imports"}).click();
    await expect(page).toHaveTitle("Ledgeline — Imports");

    await page.getByRole("link", {name: "Journal", exact: true}).click();
    await expect(page).toHaveTitle("Ledgeline — Journal");
});

test("shell works at mobile width (375px)", async ({page}) => {
    await seedServerUrl(page);
    await page.setViewportSize({width: 375, height: 667});
    await page.goto("/");
    await expect(page.locator("footer")).toContainText("transactions");

    await page.getByRole("link", {name: "Reports"}).click();
    await expect(page.getByRole("tab", {name: "Balance Sheet"})).toBeVisible();
});
