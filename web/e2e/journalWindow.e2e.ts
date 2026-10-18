// The windowed journal (plan 26 B5), end to end against the real engine serving
// fixtures/sample.journal on :5099 (playwright.config.ts webServer).
//
// The client no longer holds the journal: rows arrive in 200-row blocks, a jump
// asks the engine where a transaction sits (`/api/journal/position`), and the
// edit popup fetches the one transaction it edits (`/api/transactions/{index}`).
// These are the flows that changed shape; the footer counts and the cursor keys
// are covered in smoke.e2e.ts and keys.e2e.ts.

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08, as smoke.e2e.ts

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            // `insightsOpen: false` for the reason keys.e2e.ts gives: at 1280x720
            // the open charts panel squeezes the table to a sliver below the
            // fold, and these assertions are about the table.
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token, insightsOpen: false}));
        },
        [API_URL, API_TOKEN]
    );
});

test("journal: a problem in the drawer jumps to its row, wherever it sits", async ({page}) => {
    await page.goto("/?preset=all");
    await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();

    await page.getByRole("button", {name: "6 problems"}).click();
    // The drawer draws each finding's date and description from the engine's
    // payload — the journal is not on the client to look them up in.
    const finding = page.locator("aside").getByRole("button").filter({hasText: "Delta Airlines"});
    await expect(finding).toBeVisible();
    await expect(finding).toContainText("2026-07-02");
    await finding.click();

    // The cursored row is the flagged one, and it is on screen.
    const cursored = page.locator("[aria-current='true']");
    await expect(cursored).toContainText("Delta Airlines");
    await expect(cursored).toBeInViewport();
});

test("journal: the edit popup opens on the transaction the engine has now", async ({page}) => {
    await page.goto("/");
    await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();
    // Edit keys wait on the write probe, like the toolbar button.
    await expect(page.getByRole("button", {name: "Add transaction"}).first()).toBeVisible();

    await page.keyboard.press("j");
    const row = page.locator("[aria-current='true']");
    const description = (await row.locator('button[title^="Click to edit"]').innerText()).trim();
    await page.keyboard.press("Enter");

    const popup = page.getByRole("dialog", {name: "Edit transaction"});
    await expect(popup).toBeVisible();
    await expect(popup.getByLabel("Description", {exact: true})).toHaveValue(description);
    await popup.getByRole("button", {name: "Cancel"}).click();
    await expect(popup).toBeHidden();
});

test("journal: a search narrows the rows and the footer together", async ({page}) => {
    await page.goto("/?preset=all");
    const footer = page.locator("footer");
    await expect(footer).toContainText("189 transactions");

    await page.getByLabel("Search transactions").fill("  SAFEWAY ");
    // Case and surrounding space are ignored, exactly as before the search
    // moved into the engine.
    await expect(footer).not.toContainText("189 transactions");
    const rows = page.locator("tbody tr[data-txn]");
    await expect(rows.first()).toContainText("Safeway");
    const count = await rows.count();
    for (let i = 0; i < count; i += 1) await expect(rows.nth(i)).toContainText("Safeway");
});
