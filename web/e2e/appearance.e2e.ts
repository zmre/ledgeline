// Plan 39: themes follow the OS unless pinned, are chosen from Settings →
// Appearance, and the interface scale steps with Cmd/Ctrl +, −, 0. The e2e
// engine keeps view prefs in the browser (plan 38 WP-5), so a choice survives
// a reload through localStorage, and the per-ledger pin is not offered.
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

async function seedServerUrl(page: Page): Promise<void> {
    // Init scripts run again on every reload; seed once, or the reload would
    // wipe the very prefs these tests check survive it.
    await page.addInitScript((token) => {
        if (localStorage.getItem("ledgeline.settings.v1") !== null) return;
        localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: "http://127.0.0.1:5099", serverToken: token}));
    }, API_TOKEN);
}

const html = (page: Page) => page.locator("html");
const rootPx = (page: Page) => page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize));

test("the default theme follows the OS colour scheme", async ({page}) => {
    await page.emulateMedia({colorScheme: "light"});
    await page.goto("/");
    await expect(html(page)).toHaveAttribute("data-theme", "default-light");
    await page.emulateMedia({colorScheme: "dark"});
    await expect(html(page)).toHaveAttribute("data-theme", "default-dark");
});

test("a family follows the OS, a variant or dark-only theme stays put, and the choice survives a reload", async ({page}) => {
    await seedServerUrl(page);
    await page.emulateMedia({colorScheme: "light"});
    await page.goto("/settings?tab=appearance");
    await expect(page.getByTestId("settings-appearance")).toBeVisible();
    await expect(page.getByTestId("appearance-pin-theme")).toHaveCount(0);

    await page.getByTestId("theme-option-catppuccin").click();
    await expect(html(page)).toHaveAttribute("data-theme", "catppuccin-latte");
    await page.emulateMedia({colorScheme: "dark"});
    await expect(html(page)).toHaveAttribute("data-theme", "catppuccin-mocha");

    await page.getByTestId("theme-option-github-light").click();
    await expect(html(page)).toHaveAttribute("data-theme", "github-light");

    await page.getByTestId("theme-card-dracula").getByRole("radio").click();
    await page.emulateMedia({colorScheme: "light"});
    await expect(html(page)).toHaveAttribute("data-theme", "dracula");
    await expect(page.getByTestId("theme-card-dracula").getByRole("radio")).toHaveAttribute("aria-checked", "true");

    await page.reload();
    await expect(html(page)).toHaveAttribute("data-theme", "dracula");
});

test("Cmd/Ctrl +, −, 0 step the interface scale and it persists", async ({page}) => {
    await seedServerUrl(page);
    await page.goto("/");
    await expect(page).toHaveTitle("Ledgeline — Journal");
    expect(await rootPx(page)).toBe(16);

    await page.keyboard.press("ControlOrMeta+Equal");
    await expect.poll(() => rootPx(page)).toBeCloseTo(17.6);
    await page.keyboard.press("ControlOrMeta+Equal");
    await expect.poll(() => rootPx(page)).toBe(20);

    await page.reload();
    await expect(page.locator("footer")).toContainText("transactions"); // hydrated: the keymap is listening
    await expect.poll(() => rootPx(page)).toBe(20);

    await page.keyboard.press("ControlOrMeta+Minus");
    await expect.poll(() => rootPx(page)).toBeCloseTo(17.6);
    await page.keyboard.press("ControlOrMeta+Digit0");
    await expect.poll(() => rootPx(page)).toBe(16);
});

test("the scale control in Settings matches the shortcuts", async ({page}) => {
    await seedServerUrl(page);
    await page.goto("/settings?tab=appearance");
    await page.getByTestId("appearance-scale").selectOption("150");
    await expect.poll(() => rootPx(page)).toBe(24);
    await page.getByRole("button", {name: "Larger"}).click();
    await expect.poll(() => rootPx(page)).toBe(28);
    await page.getByRole("button", {name: "Reset"}).click();
    await expect.poll(() => rootPx(page)).toBe(16);
});
