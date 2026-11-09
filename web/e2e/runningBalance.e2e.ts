// Running balances (plan 36), end to end against the real engine serving
// fixtures/sample.journal on :5099 (playwright.config.ts webServer).
//
// The flow the plan's phase C is "done" on: pin an account, change the account
// filter, and the Change/Balance columns survive (the pin is NOT the filter);
// the pin's ✕ removes them and leaves the filter alone. Fixture fact:
// `assets:bank:savings` asserts `= $13,500.00` on 2026-04-05.

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08, as smoke.e2e.ts
const SAVINGS = "assets:bank:savings";

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token, insightsOpen: false}));
        },
        [API_URL, API_TOKEN]
    );
});

const balanceHeader = (page: import("@playwright/test").Page) => page.getByRole("columnheader", {name: "Balance", exact: true});

test("running balance: pin → change the account filter → columns survive → ✕ removes them", async ({page}) => {
    await page.goto(`/?preset=all&acct=${encodeURIComponent(SAVINGS)}`);
    await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();
    await expect(balanceHeader(page)).toHaveCount(0);

    // Pin from the account chip's ⚖.
    await page.getByRole("button", {name: `Show running balance of ${SAVINGS}`}).click();
    await expect(page.getByTestId("running-pin-chip")).toContainText(`Running balance: ${SAVINGS}`);
    await expect(balanceHeader(page)).toBeVisible();
    await expect(page.getByRole("columnheader", {name: "Change", exact: true})).toBeVisible();
    await expect(page).toHaveURL(/bal=assets%3Abank%3Asavings/);

    // The assertion on 2026-04-05 holds, and the column agrees with it.
    const asserted = page.locator("tbody tr[data-txn]").filter({hasText: "transfer to savings"}).filter({hasText: "2026-04-05"});
    await expect(asserted.getByTestId("running-balance")).toContainText("$13,500.00");
    await expect(asserted.getByTestId("running-assertion-ok")).toBeVisible();

    // Drop the account filter: every row shows now, the columns stay.
    await page.getByRole("button", {name: `Remove account filter ${SAVINGS}`}).click();
    await expect(page.getByRole("button", {name: `Remove account filter ${SAVINGS}`})).toHaveCount(0);
    await expect(balanceHeader(page)).toBeVisible();
    await expect(page.getByTestId("running-pin-chip")).toBeVisible();
    // A row that never touches savings carries the balance, dimmed.
    await expect(page.locator("tbody [data-carried]").first()).toBeVisible();

    // ✕ on the pin removes the columns and nothing else.
    await page.getByRole("button", {name: "Remove running balance"}).click();
    await expect(balanceHeader(page)).toHaveCount(0);
    await expect(page.getByTestId("running-pin-chip")).toHaveCount(0);
    await expect(page).not.toHaveURL(/bal=/);
});

test("running balance: a Balances-view row filters to the account and pins it", async ({page}) => {
    await page.addInitScript(() => {
        const key = "ledgeline.settings.v1";
        const stored = JSON.parse(localStorage.getItem(key) ?? "{}") as Record<string, unknown>;
        localStorage.setItem(key, JSON.stringify({...stored, insightsOpen: true, insightsTab: "balances"}));
    });
    // Pinning keeps the date range (plan 36 decision 5), and the fixture's last
    // savings transaction (2026-04-05) is outside the default 90 days.
    await page.goto("/?preset=all");
    const row = page.getByTestId("balances-view").locator(`button[data-account="${SAVINGS}"]`);
    await row.click();
    await expect(page.getByTestId("running-pin-chip")).toContainText(SAVINGS);
    await expect(page.getByRole("button", {name: `Remove account filter ${SAVINGS}`})).toBeVisible();
    await expect(balanceHeader(page)).toBeVisible();
});

// Amendment A1: while an account is pinned, the Balances tab's cash pie is that
// account's balance line; ✕ on the pin brings the pie back. 1280px wide, the
// width the plan says to check: the chart, like the pie, is hidden below `lg`.
test("balance line: pin from Balances → the line replaces the pie → ✕ brings the pie back", async ({page}) => {
    await page.setViewportSize({width: 1280, height: 900});
    await page.addInitScript(() => {
        const key = "ledgeline.settings.v1";
        const stored = JSON.parse(localStorage.getItem(key) ?? "{}") as Record<string, unknown>;
        localStorage.setItem(key, JSON.stringify({...stored, insightsOpen: true, insightsTab: "balances"}));
    });
    // All dates: the default 90 days (to 2026-07-08) has no savings rows.
    await page.goto("/?preset=all");
    const balances = page.getByTestId("balances-view");
    await expect(balances.getByTestId("balances-pie")).toBeVisible();
    await expect(balances.getByTestId("balances-line")).toHaveCount(0);

    const row = balances.locator(`button[data-account="${SAVINGS}"]`);
    await row.click();
    await expect(page.getByTestId("running-pin-chip")).toContainText(SAVINGS);

    // The line, named for the account and commodity, in the pie's place.
    const line = balances.getByTestId("balances-line");
    await expect(line).toBeVisible();
    await expect(line.getByTestId("balance-trend").locator("path.lc-path")).toHaveCount(1);
    await expect(line.locator("h3")).toContainText(SAVINGS);
    await expect(line.locator("h3")).toContainText("$ ·");
    await expect(balances.getByTestId("balances-pie")).toHaveCount(0);
    // Every savings assertion holds: no failure markers.
    await expect(line.locator("[data-failure]")).toHaveCount(0);
    // The row you clicked is the one charted.
    await expect(row).toHaveAttribute("aria-current", "true");

    // ✕ on the pin: the pie returns and the highlight goes.
    await page.getByRole("button", {name: "Remove running balance"}).click();
    await expect(balances.getByTestId("balances-pie")).toBeVisible();
    await expect(balances.getByTestId("balances-line")).toHaveCount(0);
    await expect(row).not.toHaveAttribute("aria-current", "true");
});
