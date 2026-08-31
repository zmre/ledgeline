// E2E for the Subscriptions report tab against the REAL stack: the ledgeline
// engine serving fixtures/sample.journal on :5099 (playwright.config.ts
// webServer) with the built SPA in front.
//
// The detection window is anchored to the BROWSER's today, which the pinned
// clock fixes at 2026-07-08 — so the trailing 24 months are a stable
// 2024-07-08 → 2026-07-08 and these numbers do not rot with real time.
//
// In that window the sample journal has exactly one genuine recurring charge:
// rent at its current $1,875 lease price ($22,500/yr). Its variable-amount
// utilities, its grocery runs, and the payroll-tax withholding on its paychecks
// are all correctly excluded.

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

test("subscriptions: has its own tab and lists recurring charges", async ({page}) => {
    await page.goto("/reports");

    // It is a peer of the other reports, not part of the Insights dashboard.
    await page.getByRole("tab", {name: "Subscriptions"}).click();
    await expect(page.getByTestId("subscriptions-panel")).toBeVisible();
    await expect(page.getByTestId("insights-dashboard")).toHaveCount(0);
    await expect(page.getByTestId("subscriptions-panel")).toContainText("2024-07-08");

    const monthly = page.getByTestId("subs-box-monthly");
    await expect(monthly).toContainText("Oakview Properties");
    await expect(monthly).toContainText("$1,875.00");
    await expect(monthly).not.toContainText("Acme Corp");
    await expect(monthly).not.toContainText("Safeway");

    // No annual charges here — the box says so rather than sitting blank.
    await expect(page.getByTestId("subs-box-annual")).toContainText("No recurring charges found");
});

test("subscriptions: each box totals its charges per month and per year", async ({page}) => {
    await page.goto("/reports?tab=subs");

    // The single $1,875/mo charge → $1,875.00/mo and $22,500.00/yr.
    const total = page.getByTestId("subs-box-monthly-total");
    await expect(total).toContainText("$1,875.00/mo");
    await expect(total).toContainText("$22,500.00/yr");
});

test("subscriptions: clicking a charge opens the journal filtered to that payee", async ({page}) => {
    await page.goto("/reports?tab=subs");

    await page
        .getByTestId("subs-box-monthly")
        .getByRole("link", {name: /Oakview Properties/})
        .click();

    // Lands on the journal, searching that payee across all dates.
    await expect(page).toHaveURL(/[?&]q=Oakview\+Properties/);
    await expect(page).toHaveURL(/[?&]preset=all/);
    await expect(page.getByLabel("Search transactions")).toHaveValue("Oakview Properties");

    // The filter really applied: all dates, and far fewer than the journal's 185
    // transactions — just this subscription's own charges. 25 rather than the 24
    // the detector matched, because "all dates" reaches back past the 24-month
    // detection window to the very first rent payment.
    const footer = page.locator("footer");
    await expect(footer).toContainText("all dates");
    await expect(footer).toContainText("25 transactions");
    await expect(page.locator("tbody tr").first()).toContainText("Oakview Properties");
});

test("subscriptions: the tab round-trips through the URL", async ({page}) => {
    await page.goto("/reports");
    await page.getByRole("tab", {name: "Subscriptions"}).click();
    await expect(page).toHaveURL(/tab=subs/);

    await page.reload();
    await expect(page.getByTestId("subscriptions-panel")).toBeVisible();
});
