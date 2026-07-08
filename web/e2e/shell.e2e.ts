import {expect, test} from "@playwright/test";

test("dark theme is the default", async ({page}) => {
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("navigates between journal and reports", async ({page}) => {
    await page.goto("/");
    await expect(page.getByRole("heading", {name: "Journal"})).toBeVisible();

    await page.getByRole("link", {name: "Reports"}).click();
    await expect(page.getByRole("heading", {name: "Reports"})).toBeVisible();

    await page.getByRole("link", {name: "Journal", exact: true}).click();
    await expect(page.getByRole("heading", {name: "Journal"})).toBeVisible();
});

test("shell works at mobile width (375px)", async ({page}) => {
    await page.setViewportSize({width: 375, height: 667});
    await page.goto("/");
    await expect(page.getByRole("heading", {name: "Journal"})).toBeVisible();

    await page.getByRole("link", {name: "Reports"}).click();
    await expect(page.getByRole("heading", {name: "Reports"})).toBeVisible();
});
