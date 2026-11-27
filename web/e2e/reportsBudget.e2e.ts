// E2E smoke for the P&L's Budget column (plans/50 WP5, D12–D14): the checkbox,
// its `budget=1` in the URL, and the italic Budget column beside the Total.
//
// Against the default-projection engine on :5095 (see playwright.config.ts),
// because its journal is the one e2e fixture whose main file includes a `~`
// goal: `fixtures/projection-default/budget.journal` budgets
// `(expenses:food) $600.00` monthly, and the journal posts nothing to food.
// This spec only READS, so sharing the engine with projectionDefault.e2e.ts
// (which writes scenario files, never the main journal) is safe.
//
// # Fixture facts
//
// June 2026 holds rent ($1,500.00) and a paycheck ($4,000.00). Without the
// column there is no Food line at all; with it, Food appears as a zero-actual
// line carrying its $600.00 goal, because a goal with no postings still reads
// somewhere.

import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5095";
const PNL = "/reports?tab=is&from=2026-06-01&to=2026-06-30&interval=total";

/** Point the SPA at the :5095 engine, keeping the rest of the stored settings. */
async function connect(page: Page): Promise<void> {
    await page.addInitScript(
        ([url, token]) => {
            const key = "ledgeline.settings.v1";
            const previous = JSON.parse(localStorage.getItem(key) ?? "{}") as Record<string, unknown>;
            localStorage.setItem(key, JSON.stringify({...previous, serverUrl: url, serverToken: token, insightsOpen: false}));
        },
        [API_URL, API_TOKEN]
    );
}

test("reports: the P&L Budget checkbox adds an italic Budget column and survives in the URL", async ({page}) => {
    await connect(page);
    await page.goto(PNL);

    const expenses = page.getByTestId("is-section-opex");
    const toggle = page.getByTestId("is-budget-toggle");
    await expect(expenses).toBeVisible();
    await expect(toggle).not.toBeChecked();
    await expect(expenses.getByRole("columnheader", {name: "Budget"})).toHaveCount(0);
    await expect(expenses.locator('[data-is-row="opex/Food"]')).toHaveCount(0);

    await toggle.check();
    await expect(page).toHaveURL(/[?&]budget=1(&|$)/);
    const budgetHead = expenses.getByRole("columnheader", {name: "Budget"});
    await expect(budgetHead).toBeVisible();
    await expect(budgetHead).toHaveClass(/italic/);
    const food = expenses.locator('[data-is-row="opex/Food"]');
    await expect(food.locator("[data-column='budget']")).toContainText("600.00");
    await expect(expenses.locator("tfoot [data-column='budget']")).toContainText("600.00");

    // A reload keeps the column: it lives in the URL.
    await page.reload();
    await expect(page.getByTestId("is-budget-toggle")).toBeChecked();
    await expect(page.getByTestId("is-section-opex").getByRole("columnheader", {name: "Budget"})).toBeVisible();

    await page.getByTestId("is-budget-toggle").uncheck();
    await expect(page).not.toHaveURL(/budget=1/);
    await expect(page.getByTestId("is-section-opex").getByRole("columnheader", {name: "Budget"})).toHaveCount(0);
});
