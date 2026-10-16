// docs/journal/ — the journal view, its filter bar, the editor, the problems
// drawer and the export. Engine: the staged copy of fixtures/sample.journal.
import type {Page} from "@playwright/test";
import {annotate, around, expect, shot, test} from "./harness";

const SECTION = "journal";

// The Insights panel otherwise fills the 800px viewport above the table.
test.use({settings: {insightsOpen: false}});

test.beforeEach(async ({page}) => {
    await page.goto("/");
    await expect(page.locator("footer")).toContainText("28 transactions");
    await expect(page.locator("tbody tr").first()).toBeVisible();
});

const accountsSummary = (page: Page) => page.locator("summary").filter({hasText: "Accounts"});

test("overview", async ({page}) => {
    await shot(page, SECTION, "overview");
});

test("filter bar: account picker and date range", async ({page}) => {
    // A real filter in place, so the chips row shows what the filter does.
    await accountsSummary(page).click();
    await page.getByRole("checkbox", {name: "expenses", exact: true}).check();
    await page.keyboard.press("Escape");
    await expect(page.getByLabel("Search accounts")).toBeHidden();
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());

    const bar = page.getByLabel("From date").locator("xpath=ancestor::div[contains(@class,'rounded-box')][1]");
    const preset = page.locator("summary").filter({hasText: "Last 90 days"});
    const drawn = await annotate(page, [
        {kind: "box", target: await around([preset, page.getByLabel("To date")])},
        {kind: "circle", target: accountsSummary(page)},
    ]);
    await shot(page, SECTION, "filter-bar", {clip: [bar, drawn], padding: 8});
});

test("filter bar: account picker open", async ({page}) => {
    await accountsSummary(page).click();
    const menu = page.getByLabel("Search accounts").locator("xpath=ancestor::div[contains(@class,'dropdown-content')][1]");
    await shot(page, SECTION, "account-picker", {clip: [menu, accountsSummary(page)], padding: 10});
});

test("transaction editor", async ({page}) => {
    const row = page.locator("tbody tr").filter({hasText: "Acme"}).first();
    await row.hover();
    await row.getByRole("button", {name: "Edit whole transaction"}).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    // The date field takes focus on open; a focus ring in a manual reads as "click here".
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await shot(page, SECTION, "transaction-editor", {clip: dialog.locator(".modal-box"), padding: 0});
});

test("problems badge", async ({page}) => {
    const badge = page.getByRole("button", {name: "6 problems"});
    const header = page.locator("header, nav").first();
    const drawn = await annotate(page, [{kind: "circle", target: await around([badge, badge.locator(".indicator-item")]), pad: 0}]);
    const box = await header.boundingBox();
    if (!box) throw new Error("no header");
    // The right-hand end of the navbar: settings, problems, refresh, journal name.
    await shot(page, SECTION, "problems-badge", {clip: [drawn, {x: box.x + box.width - 260, y: box.y, width: 260, height: box.height}], padding: 4});
});

test("problems drawer", async ({page}) => {
    await page.getByRole("button", {name: "6 problems"}).click();
    await expect(page.getByRole("heading", {name: "Problems", exact: true})).toBeVisible();
    await shot(page, SECTION, "problems-drawer");
});

test("export button", async ({page}) => {
    const button = page.getByRole("button", {name: "Export journal as xlsx"});
    const footer = page.locator("footer");
    const drawn = await annotate(page, [{kind: "arrow", target: button, from: {dx: 230, dy: 0}, label: "Download as .xlsx"}]);
    await shot(page, SECTION, "export-button", {clip: [footer, drawn], padding: 6});
});

test("keyboard help", async ({page}) => {
    // At 800px tall the sheet's top edge sits over the navbar, whose labels
    // show through its translucent border. Taller, it clears the navbar.
    await page.setViewportSize({width: 1280, height: 1000});
    await page.keyboard.press("?");
    const sheet = page.getByTestId("key-help");
    await expect(sheet).toBeVisible();
    await shot(page, SECTION, "keyboard-help", {clip: sheet.locator(".modal-box"), padding: 0});
});
