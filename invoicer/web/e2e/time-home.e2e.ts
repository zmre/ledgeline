// Time on Home and on a customer's page end to end (items 4 and 5): Home's
// time row (hours this week against last week, the billable share, what is
// unbilled with Bill…), each tile a link to the matching view; and a
// customer's Time accounts (the accounts linked to it, own and inherited,
// with this month's and the unbilled hours), linked into Entries.
//
// On the time-tracking books' server (:5194, TIME_SERVER_URL), after
// time-track.e2e.ts and time-dashboard.e2e.ts (its own project, depending
// on theirs). Track writes minutes on the real clock, so the figures here
// are checked by shape (a duration, a share, money), not exactly; the
// account rows and every link are exact. That the row is absent for books
// without time tracking is dashboard.e2e.ts's (the accrual books).
//
// `vite preview` proxies only to :5199, so this spec loads the SPA the
// server embeds and skips loudly when that is build.rs's placeholder
// (REQUIRE_REAL_SPA=1 makes it fail). The browser's clock is the fixture's
// (2026-11-26, a Thursday).
//
// Written with the feature and NOT run by the agent that wrote it (agents
// never run Playwright); the lead runs it.

import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, TIME_SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 26, 12, 0, 0);
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const DURATION = /^(\d+h( \d{2}m)?|\d+m)$/;

test.use({baseURL: TIME_SERVER_URL});

async function open(page: Page, path: string): Promise<void> {
    const shell = await page.request.get(`${TIME_SERVER_URL}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer time-home.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`${TIME_SERVER_URL}/app${path}#token=${API_TOKEN}`);
}

test.describe("time on Home", () => {
    test("shows this week's hours, the billable share and what is unbilled, each a link", async ({page}) => {
        await open(page, "/");
        const row = page.getByTestId("home-time-row");
        await expect(row).toBeVisible();
        await expect(page.getByTestId("home-time-hours-value")).toHaveText(DURATION);
        await expect(page.getByTestId("home-time-hours")).toContainText("Last week to Thu");
        await expect(page.getByTestId("home-time-hours")).toHaveAttribute("href", "/app/time?period=this-week");
        await expect(page.getByTestId("home-time-billable-value")).toHaveText(/^(\d+%|<1%|—)$/);
        await expect(page.getByTestId("home-time-billable")).toHaveAttribute("href", "/app/time?period=this-week&split=billable");
        // Acme's and Globex's unbilled time has item rates in the catalog: money.
        await expect(page.getByTestId("home-time-unbilled-value")).toHaveText(/^\$[\d,]+\.\d{2}/);
        await expect(page.getByTestId("home-time-unbilled-detail")).toContainText(/customers?/);

        // The hours tile opens the Overview on This week.
        await page.getByTestId("home-time-hours").click();
        await expect(page).toHaveURL(/\/app\/time\?period=this-week$/);
        await expect(page.getByTestId("time-dashboard")).toBeVisible();
        await page.goBack();

        await page.getByTestId("home-time-bill").click();
        await expect(page).toHaveURL(/\/app\/to-bill$/);
    });
});

test.describe("a customer's time accounts", () => {
    test("lists Acme's accounts, own and inherited, linked into Entries", async ({page}) => {
        await open(page, "/customers/customers/Acme%20Corp");
        const section = page.getByTestId("customer-time-accounts");
        await expect(section).toBeVisible();
        const accounts = section.getByTestId("customer-time-account");
        await expect(accounts.first()).toHaveAttribute("data-account", "clients:acme");
        for (const account of ["clients:acme:advisory", "clients:acme:tax-return", "clients:acme:travel"]) {
            await expect(section.locator(`[data-account="${account}"]`)).toBeVisible();
        }
        // Globex's accounts aren't Acme's.
        await expect(section.locator('[data-account^="clients:globex"]')).toHaveCount(0);
        await expect(accounts.first().getByTestId("customer-time-month")).toHaveText(DURATION);
        await expect(section.getByTestId("customer-time-accounts-link")).toHaveAttribute("href", "/app/time/accounts");
        await expect(section.getByTestId("customer-time-entries")).toHaveAttribute("href", "/app/time/entries?period=all&customer=Acme+Corp");

        // An account's hours open Entries on that account this month.
        await accounts.first().getByTestId("customer-time-month").click();
        await expect(page).toHaveURL(/\/app\/time\/entries\?period=this-month&account=clients%3Aacme$/);
    });
});
