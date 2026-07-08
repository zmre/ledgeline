// App-shell smoke (WP-01, updated by WP-09 for the real pages): theme,
// first-run setup modal, and navigation. With no stored server URL the setup
// modal overlays everything, so the nav tests seed one (the fixture API from
// playwright.config.ts webServer).
import {expect, test, type Page} from "@playwright/test";

async function seedServerUrl(page: Page): Promise<void> {
    await page.addInitScript(() => {
        localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: "http://127.0.0.1:5099"}));
    });
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
